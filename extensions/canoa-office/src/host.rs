//! Finding the person's Canoa host with no address written anywhere, and starting one when none answers.
//!
//! The host that holds a home opens a door in it: a named pipe whose name is made from the home's path, which the browser
//! extension's native host also uses. Its `@hello` says, among other things, where the host's web app is. The home is the one
//! `canoa` itself uses (`COQUI_AGENTIC_PEER_HOME`, else `.canoa` in the user's folder). With no host running, the program the
//! home recorded for starting one (`<home>/native/launch.json`) is started detached, so that sessions outlive Office.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// The environment variable that names a home other than the user's own.
pub const HOME_ENV: &str = "COQUI_AGENTIC_PEER_HOME";

/// The most a door's answer may weigh.
const MAX_HELLO: usize = 64 * 1024;

/// `ERROR_PIPE_BUSY`: every instance of the pipe is serving someone; it is free again in a moment.
const PIPE_BUSY: i32 = 231;

/// How long the add-in waits for a host it started to open its door.
pub const START_WAIT: Duration = Duration::from_secs(25);

/// Where the host's web app is, as its door says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Web {
    /// The address, on this computer.
    pub url: String,
    /// Whether the host serves the add-in's chat-only app.
    pub office: bool,
    /// Whether it accepts the key of the page it serves from this computer, which is how the pane is let in with nothing typed.
    pub office_local: bool,
}

/// The host as the add-in needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    /// The web app.
    pub web: Web,
    /// The home's secret, when the host asks the page to carry it (`office_secret`).
    pub secret: Option<String>,
}

/// Why no chat could be shown, in words for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No home directory can be worked out for this user.
    NoHome,
    /// No host holds the home and the home does not say how to start one.
    NotRunning,
    /// A host was started and did not open its door in time, or could not be started.
    NotStarted(String),
    /// The host runs but its door does not say where a web app is.
    NoWebApp,
    /// The web app is there but not for the add-in: `office` or `office_local` is off.
    WebAppNotForOffice,
    /// The door named an address that is not on this computer, which the add-in does not open.
    NotLocal(String),
    /// The door answered something that is not a host's answer.
    Unreadable(String),
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoHome => f.write_str("Canoa's home folder could not be worked out for this user."),
            Self::NotRunning => f.write_str(
                "Canoa is not running, and the add-in does not know how to start it. Start Canoa, or run `canoa native install` once.",
            ),
            Self::NotStarted(why) => write!(f, "Canoa could not be started: {why}"),
            Self::NoWebApp => f.write_str(
                "Canoa is running but does not serve its web app. Add a [host.web] table with office = true and office_local = true to canoa.cfg.",
            ),
            Self::WebAppNotForOffice => f.write_str(
                "Canoa's web app is served without the Office pages. Set office = true and office_local = true in [host.web] of canoa.cfg.",
            ),
            Self::NotLocal(url) => write!(f, "Canoa's door named a web address that is not on this computer ({url}), so it was not opened."),
            Self::Unreadable(why) => write!(f, "Canoa's door answered something unexpected: {why}"),
        }
    }
}

/// The home `canoa` uses for this user.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    home_dir_from(
        std::env::var_os(HOME_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        std::env::home_dir(),
    )
}

/// The home from the variable's value and the user's folder: the variable, else `.canoa` in the user's folder.
#[must_use]
pub fn home_dir_from(variable: Option<PathBuf>, user: Option<PathBuf>) -> Option<PathBuf> {
    variable.or_else(|| user.map(|dir| dir.join(".canoa")))
}

/// The name of the door of the host that holds `home`: `canoa-native-` and 16 hexadecimal digits of SHA-256 over the home's
/// path, in lower case (the one `canoa_host::native::door_name` makes).
#[must_use]
pub fn door_name(home: &Path) -> String {
    let path = std::fs::canonicalize(home)
        .or_else(|_| std::path::absolute(home))
        .unwrap_or_else(|_| home.to_owned());
    let text = path.to_string_lossy().to_lowercase();
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::new();
    for byte in digest.iter().take(8) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("canoa-native-{hex}")
}

/// The file-system path of a pipe.
#[must_use]
pub fn pipe_path(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

fn write_frame(pipe: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let length = u32::try_from(body.len()).map_err(io::Error::other)?;
    let mut frame = length.to_le_bytes().to_vec();
    frame.extend_from_slice(body);
    pipe.write_all(&frame)
}

fn read_frame(pipe: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    pipe.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_le_bytes(length)).unwrap_or(usize::MAX);
    if length == 0 || length > MAX_HELLO {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a door answers with a message of a size no door sends",
        ));
    }
    let mut body = vec![0; length];
    pipe.read_exact(&mut body)?;
    Ok(body)
}

/// The door's `@hello` answer, or the error that says no door answers: `NotFound` for no pipe of that name.
///
/// # Errors
///
/// When the pipe cannot be opened or does not speak the door's framing.
pub fn hello(pipe: &str) -> io::Result<Value> {
    let mut attempts = 0;
    let mut stream = loop {
        match OpenOptions::new().read(true).write(true).open(pipe) {
            Ok(stream) => break stream,
            Err(error) if error.raw_os_error() == Some(PIPE_BUSY) && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    };
    // The door's answer names the address the pane opens: it is believed only from this user's own program.
    match crate::owner::server_is_this_user(&stream) {
        Ok(true) => {}
        Ok(false) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "another user's program holds the door",
            ));
        }
        Err(error) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the door's holder could not be checked ({error})"),
            ));
        }
    }
    let ask = json!({ "canoa_native": 1, "open": "@hello", "origin": "" });
    write_frame(&mut stream, ask.to_string().as_bytes())?;
    let body = read_frame(&mut stream)?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Whether `url` names this computer by a loopback name or address, over HTTP or HTTPS.
#[must_use]
pub fn is_local(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && match parsed.host() {
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
}

/// What the door's `hello` says of the web app.
///
/// # Errors
///
/// [`Failure::NoWebApp`] with no `web` in it, [`Failure::WebAppNotForOffice`] when the add-in's pages or its admission are off,
/// [`Failure::NotLocal`] for an address that is not on this computer, [`Failure::Unreadable`] for anything else.
pub fn web_of(hello: &Value) -> Result<Web, Failure> {
    let Some(web) = hello.get("web_app") else {
        return Err(Failure::NoWebApp);
    };
    let url = web
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::Unreadable("the web address is not text".to_owned()))?;
    let web = Web {
        url: url.to_owned(),
        office: web.get("office").and_then(Value::as_bool).unwrap_or(false),
        office_local: web
            .get("office_local")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    if !is_local(&web.url) {
        return Err(Failure::NotLocal(web.url));
    }
    if !(web.office && web.office_local) {
        return Err(Failure::WebAppNotForOffice);
    }
    Ok(web)
}

/// The home's secret, when the host made one: `<home>/office/secret`.
#[must_use]
pub fn secret(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("office").join("secret")).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// What starting a host runs: `<home>/native/launch.json`, which `canoa native install` writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// The program.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
}

impl Launch {
    /// The launch recorded in `home`, or `None` when there is none or it is not a launch.
    #[must_use]
    pub fn read(home: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(home.join("native").join("launch.json")).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        let program = PathBuf::from(value.get("program")?.as_str()?);
        let args = value
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(|arg| arg.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self { program, args })
    }

    /// Starts the program, detached from this process and from the job Office may have put it in, so it outlives both.
    ///
    /// # Errors
    ///
    /// When the program cannot be started.
    pub fn start(&self) -> io::Result<u32> {
        use std::os::windows::process::CommandExt as _;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let run = |flags: u32| {
            Command::new(&self.program)
                .args(&self.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(flags)
                .spawn()
        };
        match run(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(child) => Ok(child.id()),
            // A job that forbids leaving it: the process still starts, in the job.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                run(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).map(|child| child.id())
            }
            Err(error) => Err(error),
        }
    }
}

/// Finds the host of `home`, starting one with `starter` when no door answers and waiting up to `wait` for it to open its door.
///
/// # Errors
///
/// A [`Failure`] saying why there is no web app to show.
pub fn locate_with(
    home: &Path,
    wait: Duration,
    starter: &dyn Fn(&Launch) -> io::Result<u32>,
) -> Result<Located, Failure> {
    let pipe = pipe_path(&door_name(home));
    let answer = match hello(&pipe) {
        Ok(answer) => answer,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let launch = Launch::read(home).ok_or(Failure::NotRunning)?;
            starter(&launch).map_err(|error| Failure::NotStarted(error.to_string()))?;
            let until = Instant::now() + wait;
            loop {
                std::thread::sleep(Duration::from_millis(250));
                match hello(&pipe) {
                    Ok(answer) => break answer,
                    Err(error)
                        if error.kind() == io::ErrorKind::NotFound && Instant::now() < until => {}
                    Err(error) => {
                        return Err(Failure::NotStarted(format!(
                            "it did not open its door in time ({error})"
                        )));
                    }
                }
            }
        }
        Err(error) => return Err(Failure::Unreadable(error.to_string())),
    };
    let web = web_of(&answer)?;
    Ok(Located {
        web,
        secret: secret(home),
    })
}

/// [`locate_with`] for the person's own home, starting the recorded program.
///
/// # Errors
///
/// A [`Failure`] saying why there is no web app to show.
pub fn locate() -> Result<Located, Failure> {
    let home = home_dir().ok_or(Failure::NoHome)?;
    locate_with(&home, START_WAIT, &Launch::start)
}
