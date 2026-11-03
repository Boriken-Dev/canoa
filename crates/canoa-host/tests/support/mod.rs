//! The real host binary, run on a configuration in a directory of its own, and a client that dials it, proves a key and
//! speaks ACP inside the sealed session, as Canoa's chat does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    dead_code,
    reason = "test support names what it uses from std, and not every test uses all of it"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use coqui_agentic_peer::auth::{self, Identity, Method, Proof, Sealed, Trust};
use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_web::Dialer as _;
use coqui_web::websocket::{self, Frames};
use coqui_web::{Channel, Kind, Limits};
use serde_json::{Value, json};

pub const WAIT: Duration = Duration::from_secs(20);
const MAX: usize = 16 * 1024 * 1024;

/// The proof method a run of these tests uses: `CANOA_TEST_METHOD`, `noise` or `webcrypto`, else the default method. The
/// suite runs once per method.
pub fn method() -> Method {
    std::env::var("CANOA_TEST_METHOD")
        .ok()
        .filter(|name| !name.is_empty())
        .map_or(Method::DEFAULT, |name| {
            name.parse()
                .expect("CANOA_TEST_METHOD is noise or webcrypto")
        })
}

/// A new client key of this run's method.
pub fn key() -> Identity {
    Identity::generate(method()).expect("a key")
}

pub fn host_binary() -> &'static str {
    env!("CARGO_BIN_EXE_canoa-host")
}

/// A fixture `cargo test` built into `target/<profile>/examples`.
pub fn example(name: &str, library: bool) -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    let file = if library {
        format!(
            "{}{name}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        )
    } else {
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    };
    let path = profile.join("examples").join(file);
    assert!(path.exists(), "{} is built by `cargo test`", path.display());
    path
}

/// A TOML string for `text`.
pub fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("JSON")
}

/// A directory of the test's own, removed afterwards.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "canoa-host-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `canoa-host serve` on a configuration, with a home of its own.
pub struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    pub urls: BTreeMap<String, String>,
    pub peer: PeerId,
    pub scratch: Scratch,
}

impl Host {
    /// Writes `config` as Canoa.cfg and serves it, waiting for one line per entry.
    pub fn start(config: &str, entries: usize) -> Self {
        Self::start_with(config, entries, &[], &[])
    }

    /// As [`Host::start`], with more arguments to `serve` and more environment, and its input kept open for
    /// [`Host::close_input`].
    pub fn start_with(config: &str, entries: usize, args: &[&str], env: &[(&str, &str)]) -> Self {
        let env: Vec<(&str, std::ffi::OsString)> = env
            .iter()
            .map(|(name, value)| (*name, std::ffi::OsString::from(value)))
            .collect();
        Self::launch(
            Scratch::new(),
            config,
            entries,
            args,
            &env,
            Stdio::inherit(),
        )
    }

    /// As [`Host::start_in`], with the host's log written to `host.log` in `scratch` rather than the test's own.
    pub fn start_logged(
        scratch: Scratch,
        config: &str,
        entries: usize,
        env: &[(&str, std::ffi::OsString)],
    ) -> (Self, PathBuf) {
        let log = scratch.0.join("host.log");
        let file = std::fs::File::create(&log).expect("a log file");
        (
            Self::launch(scratch, config, entries, &[], env, Stdio::from(file)),
            log,
        )
    }

    /// As [`Host::start_logged`], with more arguments to `serve`.
    pub fn start_logged_with(
        scratch: Scratch,
        config: &str,
        entries: usize,
        args: &[&str],
    ) -> (Self, PathBuf) {
        let log = scratch.0.join("host.log");
        let file = std::fs::File::create(&log).expect("a log file");
        (
            Self::launch(scratch, config, entries, args, &[], Stdio::from(file)),
            log,
        )
    }

    /// As [`Host::start`], in `scratch`, with `env` added to the host's environment.
    pub fn start_in(
        scratch: Scratch,
        config: &str,
        entries: usize,
        env: &[(&str, std::ffi::OsString)],
    ) -> Self {
        Self::launch(scratch, config, entries, &[], env, Stdio::inherit())
    }

    fn launch(
        scratch: Scratch,
        config: &str,
        entries: usize,
        args: &[&str],
        env: &[(&str, std::ffi::OsString)],
        stderr: Stdio,
    ) -> Self {
        let path = scratch.0.join("Canoa.cfg");
        std::fs::write(&path, config).expect("Canoa.cfg is written");
        let mut child = Command::new(host_binary())
            .arg("serve")
            .arg("--config")
            .arg(&path)
            .args(args)
            .args(["--method", method().name()])
            .envs(env.iter().map(|(name, value)| (name, value)))
            .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
            .env("CANOA_LOG", "debug")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .expect("the host starts");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("its output");
        let (lines, received) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if lines.send(line).is_err() {
                    return;
                }
            }
        });
        let mut urls = BTreeMap::new();
        let mut peer = None;
        for _ in 0..entries {
            let line = received
                .recv_timeout(WAIT)
                .expect("the host says where each entry listens");
            let value: Value = serde_json::from_str(&line).expect("one JSON object per line");
            assert_eq!(value["method"], method().name(), "the method it proves by");
            urls.insert(
                value["name"].as_str().expect("a name").to_owned(),
                value["url"].as_str().expect("a url").to_owned(),
            );
            peer = Some(
                value["peer"]
                    .as_str()
                    .expect("a peer")
                    .parse()
                    .expect("a peer id"),
            );
        }
        Self {
            child,
            stdin,
            urls,
            peer: peer.expect("at least one entry"),
            scratch,
        }
    }

    /// Ends the host process at once, as a crash would.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Closes the host's standard input, as its parent ending does.
    pub fn close_input(&mut self) {
        drop(self.stdin.take());
    }

    /// The host's exit status, once it has ended within `wait`.
    pub fn ended_within(&mut self, wait: Duration) -> Option<std::process::ExitStatus> {
        let until = Instant::now() + wait;
        loop {
            if let Some(status) = self.child.try_wait().expect("the host's status") {
                return Some(status);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn url(&self, entry: &str) -> &str {
        &self.urls[entry]
    }

    pub fn home(&self) -> PathBuf {
        self.scratch.0.join("home")
    }

    /// Another `canoa-host` command on this host's home.
    pub fn command(&self, args: &[&str]) -> Output {
        let named = ["--method", method().name()];
        let takes_method = matches!(args.first(), Some(&"id" | &"pair"));
        Command::new(host_binary())
            .args(args)
            .args(if takes_method { &named[..] } else { &[] })
            .env("COQUI_AGENTIC_PEER_HOME", self.home())
            .output()
            .expect("the command runs")
    }

    /// Pairs `identity` as `canoa-host accept` would, without the round of being turned away first.
    pub fn pair(&self, identity: &Identity) {
        let store = TrustStore::open(&self.home().join("peers")).expect("the trust store opens");
        store
            .accept_key(&identity.key_material())
            .expect("the key is accepted");
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The host's log once it holds every one of `expected`, or as it is after [`WAIT`]: what a provider printed reaches it
/// from a thread of the host's own, a moment after the turn.
pub fn log_holding(log: &Path, expected: &[&str]) -> String {
    let until = Instant::now() + WAIT;
    loop {
        let written = std::fs::read_to_string(log).unwrap_or_default();
        if Instant::now() >= until || expected.iter().all(|text| written.contains(text)) {
            return written;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// An ACP conversation inside a sealed session.
pub struct Rpc<'a, C: Channel> {
    channel: &'a Sealed<'a, C>,
    next: std::cell::Cell<u64>,
    pub updates: std::cell::RefCell<Vec<Value>>,
}

impl<C: Channel> Rpc<'_, C> {
    pub fn send(&self, message: &Value) -> Result<(), String> {
        self.channel
            .send(message.to_string().as_bytes())
            .map_err(|failure| failure.to_string())
    }

    /// The next message, or why there is none.
    pub fn next_message(&self, wait: Duration) -> Result<Value, String> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("nothing arrived in time".to_owned());
            }
            match self.channel.receive(Some(left)) {
                Ok(Some(message)) => {
                    return serde_json::from_slice(&message).map_err(|error| error.to_string());
                }
                Ok(None) => return Err("the host closed the link".to_owned()),
                Err(coqui_web::Failure::Timeout) => {}
                Err(failure) => return Err(failure.to_string()),
            }
        }
    }

    /// Sends a request and waits for its answer: `Ok(result)` or `Err(error)`, keeping notifications.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next.get();
        self.next.set(id + 1);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(Value::String)?;
        loop {
            let message = self.next_message(WAIT).map_err(Value::String)?;
            if message.get("method").is_some() {
                self.updates.borrow_mut().push(message);
                continue;
            }
            if message["id"] == id {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    pub fn initialize(&self) -> Result<Value, Value> {
        self.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
    }

    /// The text the agent sent during one prompt.
    pub fn prompt(&self, session: &str, text: &str) -> Result<String, Value> {
        self.updates.borrow_mut().clear();
        let answer = self.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
        )?;
        assert_eq!(answer["stopReason"], "end_turn", "{answer}");
        Ok(self
            .updates
            .borrow()
            .iter()
            .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
            .collect())
    }
}

/// Dials `url`, proves `identity` to a host pinned as `host`, and runs `body` in the sealed session.
pub fn with_link<R>(
    url: &str,
    identity: &Identity,
    host: PeerId,
    body: impl FnOnce(&Rpc<'_, websocket::Channel>) -> R,
) -> Result<R, String> {
    let url = url::Url::parse(url).map_err(|error| error.to_string())?;
    let channel = websocket::Dialer::new()
        .with_frames(Frames::Binary)
        .with_limits(Limits::default().with_max_message(method().record_length(MAX)))
        .dial(Kind::WebSocket, &url, WAIT)
        .map_err(|failure| failure.to_string())?;
    let proof = Proof::new(identity.clone()).with_responders(Trust::pinned([host]));
    let established = auth::prove_out(&proof, &channel, WAIT, method().record_length(MAX))
        .map_err(|error| error.to_string())?;
    let sealed = Sealed::new(&channel, established.session);
    let rpc = Rpc {
        channel: &sealed,
        next: std::cell::Cell::new(0),
        updates: std::cell::RefCell::new(Vec::new()),
    };
    let result = body(&rpc);
    sealed.close();
    channel.close();
    Ok(result)
}

/// Whether a process `pid` is running, asked of the system's own tools.
pub fn running(pid: u32) -> bool {
    if cfg!(windows) {
        let listed = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&listed.stdout).contains(&format!("\"{pid}\""))
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .expect("kill runs")
            .success()
    }
}

/// A directory to work in, absolute, as ACP wants.
pub fn cwd(scratch: &Scratch) -> String {
    let dir = scratch.0.join("work");
    std::fs::create_dir_all(&dir).expect("a working directory");
    dir.to_string_lossy().into_owned()
}
