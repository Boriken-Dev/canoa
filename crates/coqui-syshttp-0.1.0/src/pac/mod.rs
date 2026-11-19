//! PAC (proxy auto-config) scripts, evaluated in-process by the Boa JavaScript engine, and WPAD
//! discovery.
//!
//! A script runs on a thread of its own, so a runaway one can be abandoned: each call is bounded
//! by a time limit, and the engine's loop and recursion limits end whatever is left running.
//! Windows resolves PAC through WinHTTP and needs none of this unless asked for a script by hand.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::net::IpAddr;
use core::time::Duration;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use boa_engine::object::builtins::JsFunction;
use boa_engine::{Context, JsArgs, JsResult, JsValue, NativeFunction, Source, js_string};

use crate::error::{Error, Reason};
use crate::netutil::wildcard_match;
use crate::proxy::{Route, Target};
use dns::{resolve, resolve_all};

mod builtins;
pub(crate) mod dhcp;
mod dns;
pub(crate) mod fetch;
pub(crate) mod wpad;

pub use fetch::MAX_PAC_BYTES;

const PRELUDE: &str = include_str!("prelude.js");
/// How long each name lookup a builtin makes may take: a script's own budget is not spent on DNS.
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `dnsResolve` remembers an answer, a failure included.
const DNS_TTL: Duration = Duration::from_secs(30);
/// How long a script that overran is refused before it is started afresh.
const RETRY_AFTER: Duration = Duration::from_secs(30);
const LOOP_LIMIT: u64 = 20_000_000;
const RECURSION_LIMIT: usize = 400;

thread_local! {
    // One script owns its thread, so its DNS cache is per thread.
    static DNS_CACHE: RefCell<HashMap<String, (Instant, Option<String>)>> = RefCell::new(HashMap::new());
}

fn pac_error(text: impl Into<String>) -> Error {
    Error::Pac(Reason::new(text))
}

struct Request {
    url: String,
    host: String,
    reply: Sender<Result<String, String>>,
}

struct Worker {
    requests: Sender<Request>,
}

struct State {
    worker: Option<Worker>,
    failed_at: Option<Instant>,
}

/// One PAC script: a `FindProxyForURL` function and the builtins it may call.
///
/// Calls are serialised. A call that runs past the time limit is abandoned and the script is
/// refused for 30 seconds, then started afresh.
pub struct PacScript {
    source: Arc<str>,
    timeout: Duration,
    clock: Option<i64>,
    state: Mutex<State>,
}

impl core::fmt::Debug for PacScript {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PacScript")
            .field("bytes", &self.source.len())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// Builds a [`PacScript`].
#[derive(Debug, Clone)]
#[must_use]
pub struct PacScriptBuilder {
    source: String,
    timeout: Duration,
    clock: Option<i64>,
}

impl PacScriptBuilder {
    /// The time limit of loading the script and of each call (default 5 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Fixes the clock the date and time builtins read to `unix_milliseconds`.
    #[cfg(test)]
    pub(crate) fn clock(mut self, unix_milliseconds: i64) -> Self {
        self.clock = Some(unix_milliseconds);
        self
    }

    /// Loads the script.
    ///
    /// # Errors
    ///
    /// [`Error::Pac`] when the script has a syntax error, raises while loading, overruns the time
    /// limit, or defines no `FindProxyForURL` function.
    pub fn build(self) -> Result<PacScript, Error> {
        let script = PacScript {
            source: self.source.into(),
            timeout: self.timeout,
            clock: self.clock,
            state: Mutex::new(State {
                worker: None,
                failed_at: None,
            }),
        };
        let worker = script.start()?;
        script.lock().worker = Some(worker);
        Ok(script)
    }
}

impl PacScript {
    /// Starts a script from its text.
    pub fn builder(source: impl Into<String>) -> PacScriptBuilder {
        PacScriptBuilder {
            source: source.into(),
            timeout: Duration::from_secs(5),
            clock: None,
        }
    }

    /// Loads the script at `location`: an `http(s)` URL (downloaded directly, never through a
    /// proxy; `https` needs the `tls` feature), a `file:` URL or a path. A body over
    /// [`MAX_PAC_BYTES`] is refused; text is UTF-8, else Latin-1.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the file cannot be read, [`Error::Pac`] for a download that fails or a
    /// script that does not load.
    pub fn from_location(location: &str, timeout: Duration) -> Result<Self, Error> {
        let location = location.trim();
        let bytes = if location.starts_with("http://") || location.starts_with("https://") {
            fetch::get(location, timeout)?
        } else {
            let path = location.strip_prefix("file:").unwrap_or(location);
            let path = path.strip_prefix("//").unwrap_or(path);
            // `file:///C:/x.pac` names `C:/x.pac`.
            let path = match path.strip_prefix('/') {
                Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
                _ => path,
            };
            let bytes = std::fs::read(path)?;
            if bytes.len() > MAX_PAC_BYTES {
                return Err(pac_error("script too large"));
            }
            bytes
        };
        let source = match String::from_utf8(bytes) {
            Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
            Err(error) => error
                .into_bytes()
                .iter()
                .map(|&byte| char::from(byte))
                .collect(),
        };
        Self::builder(source).build()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn start(&self) -> Result<Worker, Error> {
        let (requests, incoming) = mpsc::channel::<Request>();
        let (ready_sender, ready) = mpsc::channel::<Result<(), String>>();
        let source = Arc::clone(&self.source);
        let clock = self.clock;
        thread::Builder::new()
            .name("coqui-syshttp-pac".into())
            .stack_size(16 << 20)
            .spawn(move || run_worker(&source, clock, &ready_sender, &incoming))?;
        match ready.recv_timeout(self.timeout) {
            Ok(Ok(())) => Ok(Worker { requests }),
            Ok(Err(message)) => Err(pac_error(message)),
            Err(_) => Err(pac_error("loading the script timed out")),
        }
    }

    /// Calls `FindProxyForURL(url, host)` and returns its text, as `PROXY a:1; DIRECT`.
    /// A result that is not a string is `DIRECT`.
    ///
    /// # Errors
    ///
    /// [`Error::Pac`] when the script raises, overruns the time limit, or is being refused after an
    /// overrun.
    pub fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error> {
        let mut state = self.lock();
        if let Some(failed) = state.failed_at {
            if failed.elapsed() < RETRY_AFTER {
                return Err(pac_error("the script overran its time limit"));
            }
            state.failed_at = None;
        }
        if state.worker.is_none() {
            state.worker = Some(self.start()?);
        }
        let Some(worker) = state.worker.as_ref() else {
            return Err(pac_error("the script is not running"));
        };
        let (reply, answer) = mpsc::channel();
        let request = Request {
            url: url.to_string(),
            host: host.to_ascii_lowercase(),
            reply,
        };
        if worker.requests.send(request).is_err() {
            state.worker = None;
            return Err(pac_error("the script stopped"));
        }
        match answer.recv_timeout(self.timeout) {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(message)) => Err(pac_error(message)),
            Err(_) => {
                // Dropping the worker closes its channel; the engine's limits end what still runs.
                state.worker = None;
                state.failed_at = Some(Instant::now());
                Err(pac_error("the script overran its time limit"))
            }
        }
    }

    /// The routes the script chooses for `url`.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for a URL without a host, else as [`PacScript::find_proxy_for_url`].
    pub fn routes(&self, url: &str) -> Result<Vec<Route>, Error> {
        let target = Target::parse(url)?;
        let host = target.host.as_str();
        let text = self.find_proxy_for_url(url, host)?;
        Ok(Route::parse_pac_result(&text))
    }
}

fn run_worker(
    source: &str,
    clock: Option<i64>,
    ready: &Sender<Result<(), String>>,
    incoming: &Receiver<Request>,
) {
    let mut context = match load(source, clock) {
        Ok(context) => context,
        Err(message) => {
            let _ = ready.send(Err(message));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    while let Ok(request) = incoming.recv() {
        let answer = call(&mut context, &request.url, &request.host);
        let _ = request.reply.send(answer);
    }
}

fn describe(error: &boa_engine::JsError, context: &mut Context) -> String {
    let text = error
        .try_native(context)
        .map_or_else(|_| "script error".to_string(), |native| native.to_string());
    text.chars().take(200).collect()
}

fn load(source: &str, clock: Option<i64>) -> Result<Context, String> {
    let mut context = Context::default();
    let limits = context.runtime_limits_mut();
    limits.set_loop_iteration_limit(LOOP_LIMIT);
    limits.set_recursion_limit(RECURSION_LIMIT);
    register(&mut context).map_err(|error| describe(&error, &mut context))?;
    context
        .eval(Source::from_bytes(PRELUDE))
        .map_err(|error| describe(&error, &mut context))?;
    if let Some(milliseconds) = clock {
        let fixed = alloc::format!("__clock = function () {{ return new Date({milliseconds}); }};");
        context
            .eval(Source::from_bytes(fixed.as_str()))
            .map_err(|error| describe(&error, &mut context))?;
    }
    context
        .eval(Source::from_bytes(source))
        .map_err(|error| describe(&error, &mut context))?;
    let function = context
        .global_object()
        .get(js_string!("FindProxyForURL"), &mut context)
        .map_err(|error| describe(&error, &mut context))?;
    if function.as_callable().is_none() {
        return Err("the script defines no FindProxyForURL function".to_string());
    }
    Ok(context)
}

fn call(context: &mut Context, url: &str, host: &str) -> Result<String, String> {
    let function = context
        .global_object()
        .get(js_string!("FindProxyForURL"), context)
        .map_err(|error| describe(&error, context))?;
    let Some(function) = function.as_object().and_then(JsFunction::from_object) else {
        return Err("the script defines no FindProxyForURL function".to_string());
    };
    let arguments = [
        JsValue::from(boa_engine::JsString::from(url)),
        JsValue::from(boa_engine::JsString::from(host)),
    ];
    let result = function
        .call(&JsValue::undefined(), &arguments, context)
        .map_err(|error| describe(&error, context))?;
    Ok(result
        .as_string()
        .map_or_else(|| "DIRECT".to_string(), |text| text.to_std_string_escaped()))
}

fn text(arguments: &[JsValue], index: usize, context: &mut Context) -> JsResult<String> {
    Ok(arguments
        .get_or_undefined(index)
        .to_string(context)?
        .to_std_string_escaped())
}

fn string(value: &str) -> JsValue {
    JsValue::from(boa_engine::JsString::from(value))
}

fn dns_resolve(host: &str) -> Option<String> {
    DNS_CACHE.with(|cache| {
        if let Some((at, answer)) = cache.borrow().get(host)
            && at.elapsed() < DNS_TTL
        {
            return answer.clone();
        }
        let answer = resolve(host, DNS_TIMEOUT).map(|address| address.to_string());
        cache
            .borrow_mut()
            .insert(host.to_string(), (Instant::now(), answer.clone()));
        answer
    })
}

fn resolve_to_address(host: &str) -> Option<IpAddr> {
    host.parse()
        .ok()
        .or_else(|| dns_resolve(host)?.parse().ok())
}

fn local_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

fn register(context: &mut Context) -> JsResult<()> {
    type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;
    let natives: [(boa_engine::JsString, usize, Native); 16] = [
        (js_string!("dnsResolve"), 1, |_, a, c| {
            Ok(dns_resolve(&text(a, 0, c)?).map_or_else(JsValue::null, |address| string(&address)))
        }),
        (js_string!("dnsResolveEx"), 1, |_, a, c| {
            let list: Vec<String> = resolve_all(&text(a, 0, c)?, DNS_TIMEOUT)
                .iter()
                .map(ToString::to_string)
                .collect();
            Ok(string(&list.join("; ")))
        }),
        (js_string!("myIpAddress"), 0, |_, _, _| {
            Ok(string(&resolve(&local_name(), DNS_TIMEOUT).map_or_else(
                || "127.0.0.1".to_string(),
                |a| a.to_string(),
            )))
        }),
        (js_string!("myIpAddressEx"), 0, |_, _, _| {
            let list: Vec<String> = resolve_all(&local_name(), DNS_TIMEOUT)
                .iter()
                .map(ToString::to_string)
                .collect();
            Ok(string(&list.join("; ")))
        }),
        (js_string!("isResolvable"), 1, |_, a, c| {
            Ok(JsValue::from(
                resolve(&text(a, 0, c)?, DNS_TIMEOUT).is_some(),
            ))
        }),
        (js_string!("isResolvableEx"), 1, |_, a, c| {
            Ok(JsValue::from(
                !resolve_all(&text(a, 0, c)?, DNS_TIMEOUT).is_empty(),
            ))
        }),
        (js_string!("isInNet"), 3, |_, a, c| {
            let host = text(a, 0, c)?;
            let (pattern, mask) = (text(a, 1, c)?, text(a, 2, c)?);
            Ok(JsValue::from(resolve_to_address(&host).is_some_and(
                |address| builtins::is_in_net(address, &pattern, &mask),
            )))
        }),
        (js_string!("isInNetEx"), 2, |_, a, c| {
            Ok(JsValue::from(builtins::is_in_net_ex(
                &text(a, 0, c)?,
                &text(a, 1, c)?,
            )))
        }),
        (js_string!("sortIpAddressList"), 1, |_, a, c| {
            Ok(string(&builtins::sort_ip_address_list(&text(a, 0, c)?)))
        }),
        (js_string!("getClientVersion"), 0, |_, _, _| {
            Ok(string(&builtins::client_version()))
        }),
        (js_string!("shExpMatch"), 2, |_, a, c| {
            // Only `*` and `?` are wildcards, and the match is case-sensitive, as in browsers.
            Ok(JsValue::from(wildcard_match(
                &text(a, 1, c)?,
                &text(a, 0, c)?,
                false,
            )))
        }),
        (js_string!("isPlainHostName"), 1, |_, a, c| {
            Ok(JsValue::from(!text(a, 0, c)?.contains('.')))
        }),
        (js_string!("dnsDomainIs"), 2, |_, a, c| {
            Ok(JsValue::from(text(a, 0, c)?.ends_with(&text(a, 1, c)?)))
        }),
        (js_string!("localHostOrDomainIs"), 2, |_, a, c| {
            Ok(JsValue::from(builtins::local_host_or_domain_is(
                &text(a, 0, c)?,
                &text(a, 1, c)?,
            )))
        }),
        (js_string!("dnsDomainLevels"), 1, |_, a, c| {
            Ok(JsValue::from(
                i32::try_from(builtins::dns_domain_levels(&text(a, 0, c)?)).unwrap_or(i32::MAX),
            ))
        }),
        (js_string!("convert_addr"), 1, |_, a, c| {
            Ok(builtins::convert_addr(&text(a, 0, c)?).map_or_else(JsValue::nan, JsValue::from))
        }),
    ];
    for (name, length, function) in natives {
        context.register_global_builtin_callable(
            name,
            length,
            NativeFunction::from_fn_ptr(function),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
