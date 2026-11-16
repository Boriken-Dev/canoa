//! The browser extension's way to this host: a native messaging host (`dev.boriken.canoa`) that is a relay, not a host. The
//! browser starts the program once per connection; it moves bytes between the browser and the door of the host that holds the
//! home, and starts that host when asked. Proof, pairing and scope happen end to end between the extension and the host, so
//! the relay sees sealed records and can say nothing for either side.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a relay names what it uses from std"
)]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use coqui_agentic_peer::Error;
use coqui_agentic_peer::companion::{self, Browser, Registration};
use coqui_transport_pipe::{OsIdentity, ReadTimeout as _};
use coqui_transport_web::frame;
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::control::discover;
use crate::home::{Home, LockError};
use crate::pipe::{self, PipeName, Stream};

/// The name the extension passes to `connectNative`.
pub const HOST_NAME: &str = "dev.boriken.canoa";
/// The ids of the extension builds that may start the native host without being named at install, from `manifest.json`'s `key`
/// (an unpacked build) and, once published, the stores' ids.
pub const KNOWN_EXTENSIONS: &[&str] = &["lpppofcphpjkedfnpbpabhmiamaicaed"];
/// What a door connection's first message may weigh, and the most a reply to it may.
pub const MAX_PREAMBLE: usize = 64 * 1024;
/// The most one piece of a link's bytes carries, in either direction. Base64 of it and the message around it stay under the
/// 1 MiB the browser takes from a native host.
pub const MAX_PIECE: usize = 768_000;
/// What `@hello` is opened as.
pub const HELLO: &str = "@hello";
/// What the pairing door is opened as.
pub const PAIR: &str = "@pair";
/// How long a relay waits for the door to answer.
const DOOR_WAIT: Duration = Duration::from_secs(5);
/// The version of the preamble.
const PREAMBLE_VERSION: u64 = 1;

/// What a door connection says first.
#[derive(Debug, PartialEq, Eq)]
pub struct Preamble {
    /// An entry's name, `@control` or `@hello`.
    pub open: String,
    /// The origin the browser reported to the relay, or empty.
    pub origin: String,
}

/// The preamble in a door connection's first message, or `None` for anything else.
#[must_use]
pub fn read_preamble(bytes: &[u8]) -> Option<Preamble> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("canoa_native")?.as_u64()? != PREAMBLE_VERSION {
        return None;
    }
    let open = value.get("open")?.as_str()?.to_owned();
    let origin = value
        .get("origin")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    (!open.is_empty() && open.len() <= 256).then_some(Preamble { open, origin })
}

/// The door of the host that holds `home`: on Windows a pipe named for the home's path, on Unix `native.sock` in the home, or
/// in the runtime directory when the path is too long for a socket.
///
/// # Errors
/// When the system has no place for a socket.
pub fn door_name(home: &Path) -> Result<PipeName, pipe::Error> {
    name_in(home, "native", "")
}

/// The pipe `canoa pair --native` waits on while the person has asked to pair a browser: beside the door, never the door itself,
/// so a host that is not running can still be paired with and the host never holds a PIN.
///
/// # Errors
/// When the system has no place for a socket.
pub fn pair_door_name(home: &Path) -> Result<PipeName, pipe::Error> {
    name_in(home, "native-pair", "-pair")
}

fn name_in(home: &Path, file: &str, suffix: &str) -> Result<PipeName, pipe::Error> {
    let path = fs::canonicalize(home)
        .or_else(|_| std::path::absolute(home))
        .unwrap_or_else(|_| home.to_owned());
    let mut text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let named = format!("canoa-native-{hex}{suffix}");
    if cfg!(windows) {
        return PipeName::named(&named);
    }
    PipeName::at(path.join(format!("{file}.sock"))).or_else(|_| PipeName::named(&named))
}

fn read_frame(stream: &Stream, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut reader = stream;
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = usize::try_from(u32::from_le_bytes(length)).unwrap_or(usize::MAX);
    if length == 0 || length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the door sent a message no door sends",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Connects to the door and says what to open. `Ok(None)` when no host listens.
fn knock(home: &Path, origin: &str, open: &str) -> io::Result<Option<Stream>> {
    let name = if open == PAIR {
        pair_door_name(home)
    } else {
        door_name(home)
    }
    .map_err(|error| io::Error::other(error.to_string()))?;
    let stream = match Stream::connect(&name) {
        Ok(stream) => stream,
        Err(pipe::Error::NotListening) => return Ok(None),
        Err(error) => return Err(io::Error::other(error.to_string())),
    };
    // Whatever listens on the name, it is the user's own process or it is told nothing: the system says who owns the pipe's server
    // end (its owner on Windows, the peer's credentials on Unix), and a system that will not say is not trusted.
    let owner = stream.peer().and_then(|identity| same_user(&identity));
    if !owner.is_ok_and(|same| same) {
        return Err(io::Error::other("the door is not held by this user"));
    }
    let preamble = json!({ "canoa_native": PREAMBLE_VERSION, "open": open, "origin": origin });
    let framed = frame::encode(preamble.to_string().as_bytes(), MAX_PREAMBLE)
        .map_err(|error| io::Error::other(error.to_string()))?;
    (&stream).write_all(&framed)?;
    stream.set_read_timeout(Some(DOOR_WAIT))?;
    Ok(Some(stream))
}

/// The pairing door's side of its first message: the origin the browser reported, once the connection has asked for `@pair`.
/// The answer that it is open is sent here.
pub fn accept_pairing(channel: &pipe::PipeChannel) -> Option<String> {
    use coqui_transport::Channel as _;
    let first = channel.receive(Some(DOOR_WAIT)).ok().flatten()?;
    let wanted = read_preamble(&first).filter(|wanted| wanted.open == PAIR)?;
    channel.send(br#"{"opened":true}"#).ok()?;
    channel.set_max_message(coqui_agentic_peer::auth::MAX_PROOF_MESSAGE);
    Some(wanted.origin)
}

fn door_error_file(home: &Path) -> PathBuf {
    home.join("native").join("door-error.txt")
}

/// Records why the door could not be opened, or that it is open: what `hello` and `status` say when a browser finds the host
/// running with no door.
pub(crate) fn note_door(home: &Path, error: Option<&str>) {
    let file = door_error_file(home);
    match error {
        Some(text) => {
            let _ = fs::create_dir_all(file.parent().unwrap_or(home));
            let _ = fs::write(&file, text);
        }
        None => {
            let _ = fs::remove_file(&file);
        }
    }
}

fn door_error(home: &Path) -> Option<String> {
    fs::read_to_string(door_error_file(home))
        .ok()
        .map(|text| text.chars().filter(|c| !c.is_control()).take(200).collect())
}

fn origins_file(home: &Path) -> PathBuf {
    home.join("native").join("origins.json")
}

/// The record of which extension paired which key. A record that is not there is empty; one that is there and cannot be read
/// whole is an error, and nothing is allowed on it.
fn origins(home: &Path) -> io::Result<serde_json::Map<String, Value>> {
    let text = match fs::read_to_string(origins_file(home)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(serde_json::Map::new()),
        Err(error) => return Err(error),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the record of which extension paired which key is not readable",
        )),
    }
}

/// Binds a key paired through the door to the origin the browser gave the relay: the key is that extension's, and no other
/// origin may use it over the door.
///
/// # Errors
/// When the record cannot be written.
pub fn record_origin(home: &Path, peer: &str, origin: &str) -> io::Result<()> {
    let mut map = origins(home)?;
    map.insert(peer.to_owned(), origin.into());
    let file = origins_file(home);
    fs::create_dir_all(file.parent().unwrap_or(home))?;
    let temporary = file.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_string_pretty(&Value::Object(map)).map_err(io::Error::other)?,
    )?;
    fs::rename(temporary, file)
}

/// Whether `peer` may use the door from `origin`: a key bound to an origin only from that one; a key paired another way (by
/// `canoa accept`) is bound to none and keeps the access its pairing gave it.
#[must_use]
pub fn origin_allows(home: &Path, peer: &str, origin: &str) -> bool {
    origins(home).is_ok_and(|map| {
        map.get(peer)
            .and_then(Value::as_str)
            .is_none_or(|bound| bound == origin)
    })
}

/// Whether the other end of a stream is the user this process runs as.
fn same_user(identity: &OsIdentity) -> io::Result<bool> {
    Ok(identity.is_current_user()?.unwrap_or(false))
}

/// Whether a host holds the home now.
fn running(home: &Home) -> bool {
    matches!(home.probe(), Err(LockError::Held { .. }))
}

/// The most addresses `hello` names.
const MAX_HELLO_ADDRESSES: usize = 16;

/// This computer's name, as the system keeps it.
pub(crate) fn computer_name() -> Option<String> {
    #[cfg(unix)]
    let name = Some(
        rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .into_owned(),
    );
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(any(unix, windows)))]
    let name: Option<String> = None;
    name.map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

/// The addresses this computer is reached at from its network, in the order the system lists its interfaces: not loopback, not
/// link-local (nothing names a link-local address without a zone), not multicast or unspecified.
fn lan_addresses() -> Vec<String> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    for interface in interfaces {
        let address = interface.ip();
        let wanted = match address {
            std::net::IpAddr::V4(v4) => {
                !v4.is_loopback()
                    && !v4.is_link_local()
                    && !v4.is_multicast()
                    && !v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                !v6.is_loopback()
                    && !v6.is_multicast()
                    && !v6.is_unspecified()
                    && (v6.segments()[0] & 0xffc0) != 0xfe80
            }
        };
        let text = address.to_string();
        if wanted && !found.contains(&text) {
            found.push(text);
        }
        if found.len() == MAX_HELLO_ADDRESSES {
            break;
        }
    }
    found
}

/// What `hello` says of the computer the host runs on, so the extension counts an agent at one of these names as on this computer:
/// `{"hostname"?, "addresses": [...]}`. A claim, like the rest of `hello`, and one that only narrows what a page may do.
pub(crate) fn this_computer() -> Value {
    let mut claim = json!({ "addresses": lan_addresses() });
    if let Some(name) = computer_name() {
        let mut names = vec![name.clone()];
        names.extend(dns_name(&name).filter(|dns| *dns != name));
        claim["hostname"] = name.into();
        claim["hostnames"] = names.into();
    }
    claim
}

/// The name this computer has in the network's DNS, when it is one the system says: a domain's, which Windows keeps in
/// `USERDNSDOMAIN`; on Unix the node name already is whatever the system calls it, which may carry its domain.
fn dns_name(name: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let domain = std::env::var("USERDNSDOMAIN").ok()?;
        let domain = domain.trim().trim_matches('.').to_ascii_lowercase();
        (!domain.is_empty()).then(|| format!("{name}.{domain}"))
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

/// Adds what [`this_computer`] says to a `hello` reply that does not already say it.
fn add_this_computer(reply: &mut Value) {
    if let (Value::Object(reply), Value::Object(names)) = (reply, this_computer()) {
        for (key, value) in names {
            reply.entry(key).or_insert(value);
        }
    }
}

/// What a relay answers to `hello`: the host's claim of itself, with `running`. A claim: nothing pins a key from it.
fn hello(home: &Home, origin: &str) -> Value {
    let claim = match knock(home.dir(), origin, HELLO) {
        Ok(Some(stream)) => read_frame(&stream, MAX_PREAMBLE)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()),
        _ => None,
    };
    match claim {
        Some(Value::Object(mut claim)) if !claim.contains_key("error") => {
            claim.insert("reply".into(), "hello".into());
            claim.insert("running".into(), true.into());
            let mut reply = Value::Object(claim);
            add_this_computer(&mut reply);
            reply
        }
        _ => {
            let held = running(home);
            let mut reply = json!({ "reply": "hello", "running": held, "door": false });
            if held {
                // Canoa runs and the door is not open: say why, as far as the host could.
                let why = door_error(home.dir()).unwrap_or_else(|| {
                    "Canoa runs, but its door for browsers is not open".to_owned()
                });
                reply["why"] = why.into();
            }
            add_this_computer(&mut reply);
            reply
        }
    }
}

/// What `op:start` runs, kept in the home by `native install`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// The program.
    pub program: PathBuf,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

fn native_dir(home: &Home) -> PathBuf {
    home.dir().join("native")
}

fn launch_file(home: &Home) -> PathBuf {
    native_dir(home).join("launch.json")
}

impl Launch {
    /// The launch `native install` recorded in `home`.
    ///
    /// # Errors
    /// When none was recorded, or the file is not a launch.
    pub fn read(home: &Home) -> io::Result<Self> {
        let text = fs::read_to_string(launch_file(home))?;
        serde_json::from_str(&text).map_err(io::Error::other)
    }

    fn write(&self, home: &Home) -> io::Result<()> {
        fs::create_dir_all(native_dir(home))?;
        fs::write(
            launch_file(home),
            serde_json::to_string_pretty(self).map_err(io::Error::other)?,
        )
    }

    /// Starts the program detached from this process and from the job the browser put it in, so it outlives both. Returns its
    /// process id. Only when no host holds `home` now.
    ///
    /// # Errors
    /// When a host already holds the home, or the program cannot be started.
    pub fn start(&self, home: &Home) -> Result<u32, String> {
        if running(home) {
            return Err("Canoa is already running".to_owned());
        }
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        match command.spawn() {
            Ok(child) => Ok(child.id()),
            // A browser's job may forbid leaving it: the process still starts, in the job.
            Err(error) if cfg!(windows) && error.kind() == io::ErrorKind::PermissionDenied => {
                let mut again = Command::new(&self.program);
                again
                    .args(&self.args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                detach_in_job(&mut again);
                again
                    .spawn()
                    .map(|child| child.id())
                    .map_err(|error| format!("Canoa could not be started: {error}"))
            }
            Err(error) => Err(format!("Canoa could not be started: {error}")),
        }
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
}

#[cfg(windows)]
fn detach_in_job(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(unix)]
fn detach_in_job(command: &mut Command) {
    detach(command);
}

/// The native messages a relay takes before a link is open.
struct Relay<W: Write + Send + 'static> {
    home: Home,
    origin: String,
    output: Arc<Mutex<W>>,
}

impl<W: Write + Send + 'static> Relay<W> {
    fn say(&self, message: &Value) -> io::Result<()> {
        say(&self.output, message)
    }

    fn answer(&self, request: &Value) -> Option<Stream> {
        match request.get("op").and_then(Value::as_str) {
            Some("hello") => {
                self.say(&hello(&self.home, &self.origin)).ok()?;
                None
            }
            Some("start") => {
                let started = Launch::read(&self.home)
                    .map_err(|_| "no program was registered to start Canoa".to_owned())
                    .and_then(|launch| launch.start(&self.home));
                let message = match started {
                    Ok(pid) => json!({ "reply": "start", "started": pid }),
                    Err(error) => json!({ "reply": "start", "error": error }),
                };
                self.say(&message).ok()?;
                None
            }
            Some("discover") => {
                // Canoa installed is enough: the search is the same call the host answers `discover/nearby` with, and it
                // reads only what this user's own processes advertised and what answers on the link.
                let found = discover::wait_of(request)
                    .map_err(|(_, error)| error)
                    .map(discover::search);
                let message = match found {
                    Ok(mut found) => {
                        found["reply"] = "discover".into();
                        found
                    }
                    Err(error) => json!({ "reply": "discover", "error": error }),
                };
                self.say(&message).ok()?;
                None
            }
            Some("open") => {
                let entry = request
                    .get("entry")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match self.open(entry) {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        let _ = self.say(&json!({ "reply": "open", "error": error }));
                        None
                    }
                }
            }
            _ => {
                let _ = self.say(&json!({ "error": "unknown op" }));
                None
            }
        }
    }

    /// Asks the door for `entry`; the stream is the link, past its answer.
    fn open(&self, entry: &str) -> Result<Stream, String> {
        if entry.is_empty() || entry.len() > 256 {
            return Err("no such entry".to_owned());
        }
        let stream = match knock(self.home.dir(), &self.origin, entry) {
            Ok(Some(stream)) => stream,
            Ok(None) if entry == PAIR => {
                return Err(
                    "Canoa is not waiting to pair a browser: ask it to show a PIN first".to_owned(),
                );
            }
            Ok(None) => return Err("Canoa is not running".to_owned()),
            Err(error) => return Err(format!("the door could not be reached: {error}")),
        };
        let reply = read_frame(&stream, MAX_PREAMBLE)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .ok_or_else(|| "the door gave no answer".to_owned())?;
        if let Some(error) = reply.get("error").and_then(Value::as_str) {
            return Err(error.chars().take(200).collect());
        }
        if reply.get("opened") != Some(&Value::Bool(true)) {
            return Err("the door gave no answer".to_owned());
        }
        stream
            .set_read_timeout(None)
            .map_err(|error| error.to_string())?;
        Ok(stream)
    }
}

fn say<W: Write>(output: &Mutex<W>, message: &Value) -> io::Result<()> {
    let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
    companion::write_message(&mut *output, message.to_string().as_bytes())
}

/// What the browser may send a relay: a question, or a piece of a link's bytes.
const MAX_FROM_BROWSER: usize = companion::MAX_REPLY + 1024;

/// Serves one native port: the browser's messages on `input`, answers and pieces on `output`, until either the browser or the
/// link ends. One port is one link.
///
/// # Errors
/// When the browser's stream fails or a message is longer than a browser sends.
pub fn relay<W: Write + Send + 'static>(
    home: &Home,
    origin: &str,
    input: &mut impl Read,
    output: W,
    ended: impl Fn() + Send + Sync + 'static,
) -> io::Result<()> {
    let relay = Relay {
        home: home.clone(),
        origin: origin.to_owned(),
        output: Arc::new(Mutex::new(output)),
    };
    let ended = Arc::new(ended);
    let mut link: Option<Arc<Stream>> = None;
    while let Some(message) = companion::read_message_up_to(input, MAX_FROM_BROWSER)? {
        let Ok(request) = serde_json::from_slice::<Value>(&message) else {
            relay.say(&json!({ "error": "not JSON" }))?;
            continue;
        };
        match &link {
            None => {
                if let Some(stream) = relay.answer(&request) {
                    let stream = Arc::new(stream);
                    link = Some(Arc::clone(&stream));
                    relay.say(&json!({ "reply": "open", "opened": true }))?;
                    let output = Arc::clone(&relay.output);
                    let ended = Arc::clone(&ended);
                    std::thread::Builder::new()
                        .name("canoa-relay".to_owned())
                        .spawn(move || pump_to_browser(&stream, &output, &*ended))?;
                }
            }
            Some(stream) => {
                // After the link is open the browser sends pieces of it and nothing else.
                let Some(piece) = request.get("d").and_then(Value::as_str) else {
                    relay.say(&json!({ "error": "a link is open: send its bytes" }))?;
                    continue;
                };
                let bytes = BASE64
                    .decode(piece.as_bytes())
                    .ok()
                    .filter(|bytes| bytes.len() <= MAX_PIECE)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "a piece no extension sends")
                    })?;
                if (&**stream).write_all(&bytes).is_err() {
                    let _ = relay.say(&json!({ "closed": true }));
                    return Ok(());
                }
            }
        }
    }
    if let Some(stream) = link {
        let _ = stream.shutdown();
    }
    Ok(())
}

/// Moves what the host sends to the browser, in pieces, until the link ends; it then tells the browser and calls `ended`
/// (the program ends), because one port is one link.
fn pump_to_browser<W: Write + Send>(stream: &Stream, output: &Mutex<W>, ended: &dyn Fn()) {
    let mut reader = stream;
    let mut buffer = vec![0_u8; MAX_PIECE];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let piece = json!({ "d": BASE64.encode(&buffer[..read]) });
                if say(output, &piece).is_err() {
                    break;
                }
            }
        }
    }
    let _ = say(output, &json!({ "closed": true }));
    ended();
}

/// What `native install` was asked for.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Install {
    /// The native host's name; [`HOST_NAME`] when `None`. Another name registers a host the extension does not look for.
    pub name: Option<String>,
    /// Extension ids besides [`KNOWN_EXTENSIONS`].
    pub extensions: Vec<String>,
    /// The browsers to register for; every one when empty.
    pub browsers: Vec<Browser>,
    /// What `op:start` runs.
    pub launch: Option<Launch>,
    /// A browser's `--user-data-dir` to register in too, for Chromium on Unix.
    pub profiles: Vec<PathBuf>,
}

impl Install {
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(HOST_NAME)
    }
}

fn registration(home: &Home, program: &Path, options: &Install) -> Result<Registration, Error> {
    let mut extensions: Vec<String> = KNOWN_EXTENSIONS.iter().map(|id| (*id).to_owned()).collect();
    extensions.extend(options.extensions.iter().cloned());
    extensions.sort();
    extensions.dedup();
    let mut registration = Registration::new(
        options.name(),
        "Connects the Canoa browser extension to the Canoa on this computer",
        program,
    )?
    .extensions(extensions)
    .browsers(&options.browsers)
    .manifest_dir(native_dir(home));
    for profile in &options.profiles {
        registration = registration.profile(profile);
    }
    Ok(registration)
}

/// Registers `program` as the native host for this user, and records what `op:start` runs. Returns what it wrote.
///
/// # Errors
/// When a file or the registry refuses, or no browser profile exists to register in.
pub fn install(home: &Home, program: &Path, options: &Install) -> Result<Vec<String>, Error> {
    let registration = registration(home, program, options)?;
    fs::create_dir_all(native_dir(home)).map_err(|error| {
        Error::Discovery(format!("the native folder could not be made: {error}"))
    })?;
    let mut wrote = companion::install(&registration)?;
    if let Some(launch) = &options.launch {
        launch.write(home).map_err(|error| {
            Error::Discovery(format!("launch.json could not be written: {error}"))
        })?;
        wrote.push(launch_file(home).to_string_lossy().into_owned());
    }
    Ok(wrote)
}

/// Removes what [`install`] wrote under `name`, and the launch record, leaving an empty `native` folder out.
///
/// # Errors
/// When a file exists and cannot be removed.
pub fn uninstall(home: &Home, name: Option<&str>, profiles: &[PathBuf]) -> Result<(), Error> {
    companion::uninstall(name.unwrap_or(HOST_NAME), Some(&native_dir(home)), profiles)?;
    let removed = fs::remove_file(launch_file(home));
    if let Err(error) = removed
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(Error::Discovery(format!(
            "launch.json could not be removed: {error}"
        )));
    }
    // Empty or not there; a folder with something else in it stays.
    let _ = fs::remove_dir(native_dir(home));
    Ok(())
}

/// Where the native host is registered, and what `op:start` would run.
#[derive(Debug)]
pub struct Status {
    /// Each manifest file and registry key that exists.
    pub registered: Vec<String>,
    /// What `op:start` runs, if a launch was recorded.
    pub launch: Option<Launch>,
    /// Whether a host holds the home.
    pub running: bool,
    /// Who holds it, by what it wrote: its name and process id.
    pub holder: Option<crate::home::Holder>,
    /// Why the running host could not open its door for browsers, when it could not.
    pub door_error: Option<String>,
}

/// What is registered for `name` and whether a host runs.
///
/// # Errors
/// When the registry cannot be read.
pub fn status(home: &Home, name: Option<&str>, profiles: &[PathBuf]) -> Result<Status, Error> {
    Ok(Status {
        registered: companion::installed(
            name.unwrap_or(HOST_NAME),
            Some(&native_dir(home)),
            profiles,
        )?,
        launch: Launch::read(home).ok(),
        running: running(home),
        holder: home.holder(),
        door_error: door_error(home.dir()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preamble_names_what_it_opens_and_nothing_else_passes() {
        let ok = read_preamble(
            br#"{"canoa_native":1,"open":"@hello","origin":"chrome-extension://x/"}"#,
        )
        .unwrap();
        assert_eq!(ok.open, "@hello");
        assert_eq!(ok.origin, "chrome-extension://x/");
        for bad in [
            &br#"{"open":"x"}"#[..],
            br#"{"canoa_native":2,"open":"x"}"#,
            br#"{"canoa_native":1}"#,
            br#"{"canoa_native":1,"open":""}"#,
            b"not json",
        ] {
            assert!(read_preamble(bad).is_none());
        }
        let long = format!(r#"{{"canoa_native":1,"open":"{}"}}"#, "x".repeat(300));
        assert!(read_preamble(long.as_bytes()).is_none());
    }

    #[test]
    fn the_door_is_named_for_the_home_and_two_homes_have_two_doors() {
        let a = std::env::temp_dir().join("canoa-door-a");
        let b = std::env::temp_dir().join("canoa-door-b");
        assert_eq!(door_name(&a).unwrap(), door_name(&a).unwrap());
        assert_ne!(door_name(&a).unwrap(), door_name(&b).unwrap());
    }

    #[test]
    fn only_this_users_door_is_spoken_to() {
        let mut identity = OsIdentity::default();
        assert!(
            !same_user(&identity).expect("answers"),
            "a system that will not say is not trusted"
        );
        identity.user = Some("S-1-5-21-not-me".to_owned());
        assert!(
            !same_user(&identity).expect("answers"),
            "another user's door is not this user's"
        );
        identity.user = Some(coqui_transport_pipe::current_user().expect("this user"));
        assert!(same_user(&identity).expect("answers"));
    }

    #[test]
    fn a_record_of_origins_that_cannot_be_read_allows_nothing_and_is_not_written_over() {
        let dir = std::env::temp_dir().join(format!("canoa-origins-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("native")).expect("a folder");
        assert!(
            origin_allows(&dir, "sha256:a", "chrome-extension://x/"),
            "no record, no binding"
        );
        fs::write(dir.join("native").join("origins.json"), "[1]").expect("a file");
        assert!(!origin_allows(&dir, "sha256:a", "chrome-extension://x/"));
        assert!(record_origin(&dir, "sha256:a", "chrome-extension://x/").is_err());
        assert_eq!(
            fs::read_to_string(dir.join("native").join("origins.json")).expect("kept"),
            "[1]"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_piece_and_its_message_fit_what_a_browser_takes() {
        let piece = json!({ "d": BASE64.encode(&vec![0xff_u8; MAX_PIECE]) }).to_string();
        assert!(piece.len() <= companion::MAX_REPLY, "{}", piece.len());
    }
}
