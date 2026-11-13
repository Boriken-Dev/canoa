//! The delegation tools for an agent that takes no HTTP MCP server: a stdio MCP server that relays each request line to the host's
//! loopback tool server and the answer back. It holds no state of its own: the address and the bearer come from its environment
//! (a command line is visible to every user of the computer), a request is one connection, and it ends when its input does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and the clock are std's"
)]

use std::io::{BufRead, Read as _, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs as _};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// The variable that holds the tool server's URL.
pub(crate) const URL_VAR: &str = "CANOA_DELEGATE_URL";
/// The variable that holds the session's bearer.
pub(crate) const TOKEN_VAR: &str = "CANOA_DELEGATE_TOKEN";
/// Requests under way at once: what the tool server takes from one bearer.
const IN_FLIGHT: usize = 8;
/// How long to wait for a connection to the tool server.
const CONNECT_LIMIT: Duration = Duration::from_secs(5);
/// How long an answer may take: the longest wait a call may ask of the host, and a margin.
const ANSWER_LIMIT: Duration = Duration::from_secs(super::MAX_WAIT.as_secs() + 60);
/// The largest line either way.
const MAX_LINE: usize = crate::serve::MAX_LINE;

/// Why the bridge will not start.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    NoUrl,
    NoToken,
    BadToken,
    BadUrl,
    NotLoopback,
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NoUrl => "CANOA_DELEGATE_URL is not set",
            Self::NoToken => "CANOA_DELEGATE_TOKEN is not set",
            Self::BadToken => "CANOA_DELEGATE_TOKEN holds a character a header cannot carry",
            Self::BadUrl => "CANOA_DELEGATE_URL is not an http:// address",
            Self::NotLoopback => "CANOA_DELEGATE_URL is not on this computer's loopback",
        })
    }
}

/// The tool server a bridge relays to, checked.
pub(crate) struct Target {
    addresses: Vec<SocketAddr>,
    authority: String,
    path: String,
    token: String,
}

impl core::fmt::Debug for Target {
    /// Never the bearer.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Target")
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

impl Target {
    /// The server at `url`, which must be a loopback address, with `token` as its bearer.
    pub(crate) fn new(url: Option<&str>, token: Option<&str>) -> Result<Self, Refused> {
        let url = url.filter(|url| !url.is_empty()).ok_or(Refused::NoUrl)?;
        let token = token
            .filter(|token| !token.is_empty())
            .ok_or(Refused::NoToken)?;
        if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(Refused::BadToken);
        }
        if url.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
            return Err(Refused::BadUrl);
        }
        let rest = url
            .get(..7)
            .filter(|scheme| scheme.eq_ignore_ascii_case("http://"))
            .map(|_| &url[7..])
            .ok_or(Refused::BadUrl)?;
        let (authority, path) = rest
            .find('/')
            .map_or((rest, "/"), |at| (&rest[..at], &rest[at..]));
        if authority.contains('@') {
            return Err(Refused::BadUrl);
        }
        let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
            let (host, after) = inner.split_once(']').ok_or(Refused::BadUrl)?;
            (host, after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        let port: u16 = match port {
            Some(port) => port.parse().map_err(|_| Refused::BadUrl)?,
            None => 80,
        };
        let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) if host.eq_ignore_ascii_case("localhost") => (host, port)
                .to_socket_addrs()
                .map_err(|_| Refused::NotLoopback)?
                .collect(),
            Err(_) => return Err(Refused::NotLoopback),
        };
        // Whatever a name resolved to, only loopback is ever dialled.
        let addresses: Vec<SocketAddr> = addresses
            .into_iter()
            .filter(|address| address.ip().is_loopback())
            .collect();
        if addresses.is_empty() {
            return Err(Refused::NotLoopback);
        }
        Ok(Self {
            addresses,
            authority: authority.to_owned(),
            path: path.to_owned(),
            token: token.to_owned(),
        })
    }

    /// One request to the tool server: its status and body.
    fn post(&self, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let mut stream = self
            .addresses
            .iter()
            .find_map(|address| TcpStream::connect_timeout(address, CONNECT_LIMIT).ok())
            .ok_or("the host's tool server is not listening")?;
        let _ = stream.set_read_timeout(Some(ANSWER_LIMIT));
        let _ = stream.set_write_timeout(Some(CONNECT_LIMIT * 6));
        let mut sent = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\n\
             Content-Type: application/json\r\nAccept: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.path,
            self.authority,
            self.token,
            body.len()
        )
        .into_bytes();
        sent.extend_from_slice(body);
        stream
            .write_all(&sent)
            .and_then(|()| stream.flush())
            .map_err(|_| "the request could not be sent to the host's tool server")?;
        let mut answer = Vec::new();
        let limit = u64::try_from(MAX_LINE)
            .unwrap_or(u64::MAX)
            .saturating_add(8192);
        let _ = (&mut stream).take(limit).read_to_end(&mut answer);
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut parsed = httparse::Response::new(&mut headers);
        let Ok(httparse::Status::Complete(used)) = parsed.parse(&answer) else {
            return Err("the host's tool server's answer was cut off".to_owned());
        };
        let status = parsed.code.unwrap_or(0);
        let length = parsed
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case("content-length"))
            .and_then(|header| {
                core::str::from_utf8(header.value)
                    .ok()?
                    .trim()
                    .parse::<usize>()
                    .ok()
            });
        let mut body = answer.split_off(used);
        if let Some(length) = length {
            if body.len() < length {
                return Err("the host's tool server's answer was cut off".to_owned());
            }
            body.truncate(length);
        }
        Ok((status, body))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Requests under way, so a client that sends more than the server takes waits instead of being refused.
struct Slots {
    held: Mutex<usize>,
    freed: Condvar,
}

impl Slots {
    fn take(self: &Arc<Self>) -> Slot {
        let mut held = lock(&self.held);
        while *held >= IN_FLIGHT {
            held = self
                .freed
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *held += 1;
        Slot(Arc::clone(self))
    }
}

struct Slot(Arc<Slots>);

impl Drop for Slot {
    fn drop(&mut self) {
        *lock(&self.0.held) -= 1;
        self.0.freed.notify_one();
    }
}

/// The JSON-RPC error answering `id`.
fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Writes one message as a line. `false` when the output is gone.
fn emit<W: Write>(output: &Mutex<W>, message: &Value) -> bool {
    let mut line = message.to_string().into_bytes();
    line.push(b'\n');
    let mut output = lock(output);
    output
        .write_all(&line)
        .and_then(|()| output.flush())
        .is_ok()
}

/// The answer to one request line, or `None` for a notification the server took.
fn relay(target: &Target, line: &[u8], id: Option<&Value>) -> Option<Value> {
    let said = |message: &str| id.map(|id| failure(id, -32000, message));
    match target.post(line) {
        Ok((200, body)) => match serde_json::from_slice::<Value>(&body) {
            Ok(answer) => Some(answer),
            Err(_) => said("the host's tool server's answer is not JSON"),
        },
        Ok((202, _)) => None,
        Ok((401, _)) => said("the host refused this session's bearer: the session has ended"),
        Ok((429, _)) => said("the host's tool server is busy with this session's other calls"),
        Ok((status, _)) => said(&format!("the host's tool server answered {status}")),
        Err(why) => said(&why),
    }
}

/// Relays every line of `input` to `target` and each answer to `output`, until `input` ends. A request is answered as it
/// completes, not in order, as MCP's ids allow; one the server never takes gets an error, never silence.
///
/// # Errors
///
/// Reading `input` failed, or a line was longer than any message the host takes.
pub(crate) fn serve<R, W>(target: Target, mut input: R, output: W) -> std::io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let target = Arc::new(target);
    let output = Arc::new(Mutex::new(output));
    let slots = Arc::new(Slots {
        held: Mutex::new(0),
        freed: Condvar::new(),
    });
    let limit = u64::try_from(MAX_LINE)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    loop {
        let mut line = Vec::new();
        if (&mut input).take(limit).read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.len() > MAX_LINE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "a request line is longer than the host takes",
            ));
        }
        let text = line.trim_ascii();
        if text.is_empty() {
            continue;
        }
        let id = match serde_json::from_slice::<Value>(text) {
            Ok(message) => message.get("id").cloned(),
            Err(_) => {
                if !emit(
                    &output,
                    &failure(&Value::Null, -32700, "the line is not JSON"),
                ) {
                    return Ok(());
                }
                continue;
            }
        };
        let slot = slots.take();
        let (target, sink, text) = (Arc::clone(&target), Arc::clone(&output), text.to_vec());
        let spawned = std::thread::Builder::new()
            .name("canoa-mcp-bridge".to_owned())
            .spawn({
                let id = id.clone();
                move || {
                    let _slot = slot;
                    if let Some(answer) = relay(&target, &text, id.as_ref()) {
                        let _ = emit(&sink, &answer);
                    }
                }
            });
        if spawned.is_err()
            && let Some(id) = id
        {
            let _ = emit(
                &output,
                &failure(&id, -32000, "the bridge could not start a request"),
            );
        }
    }
}
