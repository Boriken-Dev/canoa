//! PAC (proxy auto-config) scripts, evaluated in-process by the Boa JavaScript engine, and WPAD
//! discovery.
//!
//! A script runs on a thread of its own, so a runaway one can be abandoned: each call is bounded
//! by a time limit, and the engine's loop and recursion limits end whatever is left running.
//! Windows resolves PAC through WinHTTP and needs none of this unless asked for a script by hand.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::IpAddr;
use core::time::Duration;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use boa_engine::object::builtins::JsFunction;
use boa_engine::{Context, JsArgs, JsResult, JsValue, NativeFunction, Source, js_string};

use crate::dns::{resolve, resolve_all};
use crate::error::{Error, Reason};
use crate::netutil::wildcard_match;
use crate::proxy::{Route, Target};
use crate::settings::PacSettings;

mod builtins;
mod source;
pub use source::{FindProxy, PacSource};
pub(crate) mod dhcp;
pub(crate) mod fetch;
pub(crate) mod wpad;

/// The default largest script, [`PacSettings::max_script_bytes`].
pub const DEFAULT_MAX_PAC_BYTES: usize = PacSettings::DEFAULT_MAX_SCRIPT_BYTES;

const PRELUDE: &str = include_str!("prelude.js");
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

/// A builtin a caller adds to a script: it gets the call's arguments as text and answers text.
pub type CustomFunction = Arc<dyn Fn(&[String]) -> Result<String, Error> + Send + Sync>;

/// What the builtins of one script share: its name-lookup cache, kept across restarts of its worker.
struct Shared {
    settings: PacSettings,
    functions: Vec<(String, CustomFunction)>,
    dns: Mutex<HashMap<String, (Instant, Option<String>)>>,
}

/// The script state a builtin reaches; it holds nothing the collector owns.
#[derive(Clone, boa_gc::Trace, boa_gc::Finalize)]
struct Captured(
    #[unsafe_ignore_trace] Arc<Shared>,
    #[unsafe_ignore_trace] usize,
);

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
    shared: Arc<Shared>,
    clock: Option<i64>,
    state: Mutex<State>,
}

impl core::fmt::Debug for PacScript {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PacScript")
            .field("bytes", &self.source.len())
            .field("timeout", &self.shared.settings.call_timeout)
            .finish_non_exhaustive()
    }
}

/// Builds a [`PacScript`].
#[derive(Clone)]
#[must_use]
pub struct PacScriptBuilder {
    source: String,
    settings: PacSettings,
    functions: Vec<(String, CustomFunction)>,
    clock: Option<i64>,
}

impl core::fmt::Debug for PacScriptBuilder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PacScriptBuilder")
            .field("bytes", &self.source.len())
            .field("settings", &self.settings)
            .field(
                "functions",
                &self
                    .functions
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl PacScriptBuilder {
    /// The time limit of loading the script and of each call (default 5 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.settings.call_timeout = timeout;
        self
    }

    /// Adds a builtin `name` to the script, callable as `name(a, b)` from `FindProxyForURL`: the
    /// arguments arrive as text and the answer is text. An error becomes a script error.
    pub fn function(
        mut self,
        name: impl Into<String>,
        function: impl Fn(&[String]) -> Result<String, Error> + Send + Sync + 'static,
    ) -> Self {
        self.functions.push((name.into(), Arc::new(function)));
        self
    }

    /// Every bound the script runs under, at once.
    pub fn settings(mut self, settings: &PacSettings) -> Self {
        self.settings = settings.clone();
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
            shared: Arc::new(Shared {
                settings: self.settings,
                functions: self.functions,
                dns: Mutex::new(HashMap::new()),
            }),
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
            settings: PacSettings::default(),
            functions: Vec::new(),
            clock: None,
        }
    }

    /// Loads the script at `location`: an `http(s)` URL (downloaded directly, never through a
    /// proxy; `https` needs the `tls` feature), a `file:` URL or a path. A body over
    /// `max_script_bytes` is refused; text is UTF-8, else Latin-1.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the file cannot be read, [`Error::Pac`] for a download that fails or a
    /// script that does not load.
    pub fn from_location(location: &str, settings: &PacSettings) -> Result<Self, Error> {
        let location = location.trim();
        let bytes = if location.starts_with("http://") || location.starts_with("https://") {
            fetch::get(location, settings)?
        } else {
            let path = location.strip_prefix("file:").unwrap_or(location);
            let path = path.strip_prefix("//").unwrap_or(path);
            // `file:///C:/x.pac` names `C:/x.pac`.
            let path = match path.strip_prefix('/') {
                Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
                _ => path,
            };
            let bytes = std::fs::read(path)?;
            if bytes.len() > settings.max_script_bytes {
                return Err(pac_error("script too large"));
            }
            bytes
        };
        Self::builder(decode_text(bytes)).settings(settings).build()
    }

    /// The text of the script.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The time limit of loading the script and of each call.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.shared.settings.call_timeout
    }

    /// The script in the file at `path`, read as `encoding`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the file cannot be read, [`Error::Pac`] when it is over
    /// `max_script_bytes`, is not valid UTF-8 for [`PacEncoding::Utf8`], or does not load.
    pub fn from_file(
        path: impl AsRef<std::path::Path>,
        encoding: PacEncoding,
        settings: &PacSettings,
    ) -> Result<Self, Error> {
        let bytes = std::fs::read(path)?;
        if bytes.len() > settings.max_script_bytes {
            return Err(pac_error("script too large"));
        }
        let source = match encoding {
            PacEncoding::Latin1 => bytes.iter().map(|&byte| char::from(byte)).collect(),
            PacEncoding::Utf8 => String::from_utf8(bytes)
                .map_err(|_| pac_error("the script is not valid UTF-8"))?
                .trim_start_matches('\u{feff}')
                .to_string(),
        };
        Self::builder(source).settings(settings).build()
    }

    /// The script downloaded from the `http(s)` URL `url`, directly, never through a proxy. A
    /// caller that wants the download reused keeps the [`PacScript`] (a [`Resolver`](crate::Resolver)
    /// downloads again after `download_cache_ttl`).
    ///
    /// # Errors
    ///
    /// [`Error::Pac`] for a download that fails or a script that does not load.
    pub fn from_url(url: &str, settings: &PacSettings) -> Result<Self, Error> {
        let bytes = fetch::get(url, settings)?;
        Self::builder(decode_text(bytes)).settings(settings).build()
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
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name("coqui-proxy-pac".into())
            .stack_size(16 << 20)
            .spawn(move || run_worker(&source, clock, &shared, &ready_sender, &incoming))?;
        match ready.recv_timeout(self.shared.settings.call_timeout) {
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
        let reduced = self
            .shared
            .settings
            .browser_compatibility
            .then(|| browser_url(url));
        let url = reduced.as_deref().unwrap_or(url);
        let mut state = self.lock();
        if let Some(failed) = state.failed_at {
            if failed.elapsed() < self.shared.settings.refusal_after_overrun {
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
        match answer.recv_timeout(self.shared.settings.call_timeout) {
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

/// The WPAD answer of this machine: the first `wpad.dat` (DHCP option 252, then DNS devolution)
/// that downloads and loads, or `None`. `fqdn` names the machine (default: its own name).
///
/// # Errors
///
/// Reserved for failures of the discovery itself; a location whose script fails is skipped.
pub fn discover(fqdn: Option<&str>, settings: &PacSettings) -> Result<Option<PacScript>, Error> {
    Ok(wpad::discover(fqdn, settings))
}

/// The names of the builtins every script has, those of the PAC specification and Microsoft's
/// `*Ex` extensions.
#[must_use]
pub fn builtin_names() -> &'static [&'static str] {
    &[
        "convert_addr",
        "dateRange",
        "dnsDomainIs",
        "dnsDomainLevels",
        "dnsResolve",
        "dnsResolveEx",
        "getClientVersion",
        "isInNet",
        "isInNetEx",
        "isPlainHostName",
        "isResolvable",
        "isResolvableEx",
        "localHostOrDomainIs",
        "myIpAddress",
        "myIpAddressEx",
        "shExpMatch",
        "sortIpAddressList",
        "timeRange",
        "weekdayRange",
    ]
}

/// How a script file is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PacEncoding {
    /// UTF-8, a byte order mark dropped.
    #[default]
    Utf8,
    /// ISO 8859-1: every byte is the character of that code.
    Latin1,
}

/// UTF-8 (a byte order mark dropped), else Latin-1.
fn decode_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
        Err(error) => error
            .into_bytes()
            .iter()
            .map(|&byte| char::from(byte))
            .collect(),
    }
}

/// `https://host/` for an `https:` URL, as browsers show a script: the path and query of a secure
/// URL are not for a script to see. Any other URL is unchanged.
fn browser_url(url: &str) -> String {
    match Target::parse(url) {
        Ok(target) if target.scheme == "https" => {
            alloc::format!(
                "https://{}/",
                crate::proxy::join_host_port(&target.host, target.port)
            )
        }
        _ => url.to_string(),
    }
}

fn run_worker(
    source: &str,
    clock: Option<i64>,
    shared: &Arc<Shared>,
    ready: &Sender<Result<(), String>>,
    incoming: &Receiver<Request>,
) {
    let mut context = match load(source, clock, shared) {
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

fn load(source: &str, clock: Option<i64>, shared: &Arc<Shared>) -> Result<Context, String> {
    let mut context = Context::default();
    let limits = context.runtime_limits_mut();
    limits.set_loop_iteration_limit(shared.settings.loop_iterations);
    limits.set_recursion_limit(shared.settings.recursion);
    register(&mut context, shared).map_err(|error| describe(&error, &mut context))?;
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

fn dns_resolve(shared: &Shared, host: &str) -> Option<String> {
    let mut cache = shared
        .dns
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let ttl = shared.settings.dns_cache_ttl;
    if let Some((at, answer)) = cache.get(host)
        && at.elapsed() < ttl
    {
        return answer.clone();
    }
    let answer = resolve(host, shared.settings.dns_timeout).map(|address| address.to_string());
    cache.retain(|_, (at, _)| at.elapsed() < ttl);
    if cache.len() >= shared.settings.dns_cache_capacity
        && let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(name, _)| name.clone())
    {
        cache.remove(&oldest);
    }
    cache.insert(host.to_string(), (Instant::now(), answer.clone()));
    answer
}

fn resolve_to_address(shared: &Shared, host: &str) -> Option<IpAddr> {
    host.parse()
        .ok()
        .or_else(|| dns_resolve(shared, host)?.parse().ok())
}

fn local_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

fn register(context: &mut Context, shared: &Arc<Shared>) -> JsResult<()> {
    type Native = fn(&Shared, &JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;
    let natives: [(boa_engine::JsString, usize, Native); 16] = [
        (js_string!("dnsResolve"), 1, |s, _, a, c| {
            Ok(dns_resolve(s, &text(a, 0, c)?)
                .map_or_else(JsValue::null, |address| string(&address)))
        }),
        (js_string!("dnsResolveEx"), 1, |s, _, a, c| {
            let list: Vec<String> = resolve_all(&text(a, 0, c)?, s.settings.dns_timeout)
                .iter()
                .map(ToString::to_string)
                .collect();
            Ok(string(&list.join("; ")))
        }),
        (js_string!("myIpAddress"), 0, |s, _, _, _| {
            Ok(string(
                &resolve(&local_name(), s.settings.dns_timeout)
                    .map_or_else(|| "127.0.0.1".to_string(), |a| a.to_string()),
            ))
        }),
        (js_string!("myIpAddressEx"), 0, |s, _, _, _| {
            let list: Vec<String> = resolve_all(&local_name(), s.settings.dns_timeout)
                .iter()
                .map(ToString::to_string)
                .collect();
            Ok(string(&list.join("; ")))
        }),
        (js_string!("isResolvable"), 1, |s, _, a, c| {
            Ok(JsValue::from(
                resolve(&text(a, 0, c)?, s.settings.dns_timeout).is_some(),
            ))
        }),
        (js_string!("isResolvableEx"), 1, |s, _, a, c| {
            Ok(JsValue::from(
                !resolve_all(&text(a, 0, c)?, s.settings.dns_timeout).is_empty(),
            ))
        }),
        (js_string!("isInNet"), 3, |s, _, a, c| {
            let host = text(a, 0, c)?;
            let (pattern, mask) = (text(a, 1, c)?, text(a, 2, c)?);
            Ok(JsValue::from(resolve_to_address(s, &host).is_some_and(
                |address| builtins::is_in_net(address, &pattern, &mask),
            )))
        }),
        (js_string!("isInNetEx"), 2, |_s, _, a, c| {
            Ok(JsValue::from(builtins::is_in_net_ex(
                &text(a, 0, c)?,
                &text(a, 1, c)?,
            )))
        }),
        (js_string!("sortIpAddressList"), 1, |_s, _, a, c| {
            Ok(string(&builtins::sort_ip_address_list(&text(a, 0, c)?)))
        }),
        (js_string!("getClientVersion"), 0, |_s, _, _, _| {
            Ok(string(&builtins::client_version()))
        }),
        (js_string!("shExpMatch"), 2, |_s, _, a, c| {
            // Only `*` and `?` are wildcards, and the match is case-sensitive, as in browsers.
            Ok(JsValue::from(wildcard_match(
                &text(a, 1, c)?,
                &text(a, 0, c)?,
                false,
            )))
        }),
        (js_string!("isPlainHostName"), 1, |_s, _, a, c| {
            Ok(JsValue::from(!text(a, 0, c)?.contains('.')))
        }),
        (js_string!("dnsDomainIs"), 2, |_s, _, a, c| {
            Ok(JsValue::from(text(a, 0, c)?.ends_with(&text(a, 1, c)?)))
        }),
        (js_string!("localHostOrDomainIs"), 2, |_s, _, a, c| {
            Ok(JsValue::from(builtins::local_host_or_domain_is(
                &text(a, 0, c)?,
                &text(a, 1, c)?,
            )))
        }),
        (js_string!("dnsDomainLevels"), 1, |_s, _, a, c| {
            Ok(JsValue::from(
                i32::try_from(builtins::dns_domain_levels(&text(a, 0, c)?)).unwrap_or(i32::MAX),
            ))
        }),
        (js_string!("convert_addr"), 1, |_s, _, a, c| {
            Ok(builtins::convert_addr(&text(a, 0, c)?).map_or_else(JsValue::nan, JsValue::from))
        }),
    ];
    for (index, (name, _)) in shared.functions.iter().enumerate() {
        context.register_global_builtin_callable(
            boa_engine::JsString::from(name.as_str()),
            0,
            NativeFunction::from_copy_closure_with_captures(
                |_this, arguments, captured: &Captured, context| {
                    let mut texts = Vec::new();
                    for position in 0..arguments.len() {
                        texts.push(text(arguments, position, context)?);
                    }
                    match (captured.0.functions[captured.1].1)(&texts) {
                        Ok(answer) => Ok(string(&answer)),
                        Err(error) => Err(boa_engine::JsNativeError::error()
                            .with_message(error.to_string())
                            .into()),
                    }
                },
                Captured(Arc::clone(shared), index),
            ),
        )?;
    }
    for (name, length, function) in natives {
        let captures = Captured(Arc::clone(shared), 0);
        context.register_global_builtin_callable(
            name,
            length,
            NativeFunction::from_copy_closure_with_captures(
                move |this, arguments, captured: &Captured, context| {
                    function(&captured.0, this, arguments, context)
                },
                captures,
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
