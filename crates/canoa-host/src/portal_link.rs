//! A host run for a portal (`serve --portal DIR --portal-user NAME`): every entry and the control channel listen on a socket in
//! `DIR`, a socket of the portal's own introduces a browser's key, and a connection is let in only when the system says its
//! process runs as the portal's user.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;

use coqui_peer_webcrypto::TrustStore;
use coqui_transport_pipe::{Listener, PipeName, Stream};

use crate::config::{Config, ControlConfig, Listen};
use crate::peers;
use crate::serve::ServeError;

/// The folder of the entries' sockets under the portal's directory.
pub const ENTRIES: &str = "entries";
/// The control channel's socket in the portal's directory.
pub const CONTROL: &str = "control.sock";
/// The introduction socket in the portal's directory.
pub const INTRODUCTION: &str = "portal.sock";
/// The longest request the introduction socket reads.
const MAX_REQUEST: usize = 4096;
/// How long a request may take to arrive.
const REQUEST_TIME: Duration = Duration::from_secs(5);
/// How long an idle accept waits before looking at whether the host stopped.
const REST: Duration = Duration::from_millis(50);

/// Where a host run for a portal listens, and who may connect.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortalSockets {
    /// The directory the sockets are made in. The portal's service makes it for this user, under a `/run/canoa` only root
    /// writes, so only this user and root can put a socket in it.
    pub dir: PathBuf,
    /// The user id of the portal. The sockets' mode lets any local user connect; a connection whose peer the system says is
    /// another user is closed before a byte is read, so this is the whole of who may reach the host.
    pub uid: u32,
}

/// The user id of `user`: a number, or a name in `/etc/passwd`.
///
/// # Errors
///
/// When the account is not in that file, or the file cannot be read.
pub fn uid_of(user: &str) -> std::io::Result<u32> {
    if let Ok(uid) = user.parse::<u32>() {
        return Ok(uid);
    }
    let passwd = std::fs::read_to_string("/etc/passwd")?;
    passwd
        .lines()
        .find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?;
            (name == user).then(|| uid.parse::<u32>().ok()).flatten()
        })
        .ok_or_else(|| std::io::Error::other("no such account in /etc/passwd"))
}

/// Refuses to run a host for a portal as an account the portal must never start one for: root, a user id below `min_uid`, an
/// account whose name (`USER`, which systemd sets for `User=`) is not the last part of `dir`, or a `dir` this account does not
/// own. The unit's `User=%i` and `/run/canoa/%i` are what make the last two agree; a portal asked to start a host for
/// another account's name would otherwise run it as that account in someone else's folder.
///
/// # Errors
///
/// A message saying which rule was broken.
pub fn check_account(dir: &Path, min_uid: u32, groups: &[String]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let uid = rustix::process::geteuid().as_raw();
        let owner = std::fs::metadata(dir).map(|meta| meta.uid()).ok();
        let name = std::env::var("USER").ok();
        let own = if groups.is_empty() {
            Vec::new()
        } else {
            own_groups()?
        };
        check_account_of(uid, name.as_deref(), dir, owner, min_uid, (&own, groups))
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, min_uid, groups);
        Err("--portal is Unix only".to_owned())
    }
}

/// Whether `uid` is one a person never has: the machine's service and dynamic-user ranges, `nobody` and the top of the range.
/// People of a directory have large ids of their own, below these, so this is not a lowest-id rule.
#[must_use]
pub const fn system_uid(uid: u32) -> bool {
    uid == 0 || (uid >= 61184 && uid <= 65535) || uid == u32::MAX
}

/// The names of the groups this process is in, as the system says.
#[cfg(unix)]
fn own_groups() -> Result<Vec<String>, String> {
    let output = crate::spawn::Command::new("/usr/bin/id")
        .arg("-Gn")
        .env("LC_ALL", "C")
        .stdin(crate::spawn::Stdio::Null)
        .stderr(crate::spawn::Stdio::Null)
        .output()
        .map_err(|error| format!("--portal: the account's groups could not be read: {error}"))?;
    if !output.status.success() {
        return Err("--portal: the account's groups could not be read".to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect())
}

#[cfg(unix)]
fn check_account_of(
    uid: u32,
    name: Option<&str>,
    dir: &Path,
    owner: Option<u32>,
    min_uid: u32,
    (own, allowed): (&[String], &[String]),
) -> Result<(), String> {
    if uid == 0 {
        return Err("--portal: this host must not run as root".to_owned());
    }
    if system_uid(uid) {
        return Err(format!(
            "--portal: user id {uid} is a system or dynamic one"
        ));
    }
    if uid < min_uid {
        return Err(format!(
            "--portal: user id {uid} is below --portal-min-uid {min_uid}"
        ));
    }
    if allowed.is_empty() {
        return Err(
            "--portal: name the groups that may have a host with --portal-group".to_owned(),
        );
    }
    if !allowed.iter().any(|group| own.contains(group)) {
        return Err("--portal: this account is in none of the --portal-group groups".to_owned());
    }
    let base = dir.file_name().and_then(|part| part.to_str());
    if name.is_none() || name != base {
        return Err("--portal: the account's name (USER) is not the last part of DIR".to_owned());
    }
    if owner != Some(uid) {
        return Err("--portal: DIR is not there or is not owned by this account".to_owned());
    }
    Ok(())
}

/// Whether the system says `stream`'s peer runs as user `uid`.
pub(crate) fn from_user(stream: &Stream, uid: u32) -> bool {
    stream
        .peer()
        .ok()
        .and_then(|peer| peer.user)
        .and_then(|user| user.parse::<u32>().ok())
        == Some(uid)
}

impl PortalSockets {
    /// Sockets in `dir`, for the portal running as user id `uid`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, uid: u32) -> Self {
        Self {
            dir: dir.into(),
            uid,
        }
    }

    /// The socket of the entry `name`.
    pub(crate) fn entry(&self, name: &str) -> Result<PipeName, ServeError> {
        PipeName::at(entry_socket(&self.dir, name)).map_err(|error| ServeError::Pipe {
            entry: name.to_owned(),
            error,
        })
    }

    /// A socket in this directory that any local user may connect to; [`from_user`] turns the others away.
    pub(crate) fn bind(&self, name: &PipeName) -> Result<Listener, coqui_transport_pipe::Error> {
        Listener::builder()
            .name(name)
            .socket_mode(0o666)
            .build()
            .map_err(|error| match error {
                coqui_transport_pipe::BuildError::Failed(error) => error,
                _ => coqui_transport_pipe::Error::InvalidOptions,
            })
    }
}

/// The socket of the entry `entry` in the portal's directory `dir`.
#[must_use]
pub fn entry_socket(dir: &Path, entry: &str) -> PathBuf {
    dir.join(ENTRIES).join(format!("{entry}.sock"))
}

/// `config` with every entry and the control channel on a socket in the portal's directory, and no web app: whatever the file
/// says of an address is replaced, so a user's canoa.cfg cannot open a port.
///
/// # Errors
///
/// [`ServeError::Pipe`] when the folder cannot be made or a name cannot make a socket path.
pub(crate) fn adapt(config: &Config, sockets: &PortalSockets) -> Result<Config, ServeError> {
    let pipe_error = |entry: &str, error| ServeError::Pipe {
        entry: entry.to_owned(),
        error,
    };
    make_folder(&sockets.dir.join(ENTRIES))
        .map_err(|error| pipe_error("portal", coqui_transport_pipe::Error::Io(error)))?;
    let mut config = config.clone();
    config.web = None;
    for entry in &mut config.entries {
        if !entry.local {
            entry.listen = Listen::Pipe(sockets.entry(&entry.name)?);
        }
    }
    let listen = PipeName::at(sockets.dir.join(CONTROL))
        .map_err(|error| pipe_error(crate::control::NAME, error))?;
    let mut control = config.control.take().unwrap_or(ControlConfig {
        listen: None,
        grants: indexmap::IndexMap::new(),
        max_records: crate::control::DEFAULT_MAX_RECORDS,
        max_record_bytes: crate::control::DEFAULT_MAX_RECORD_BYTES,
        max_upload_bytes: crate::control::DEFAULT_MAX_UPLOAD_BYTES,
        max_upload_total_bytes: crate::control::DEFAULT_MAX_UPLOAD_TOTAL,
        max_uploads: crate::control::DEFAULT_MAX_UPLOADS,
    });
    control.listen = Some(Listen::Pipe(listen));
    config.control = Some(control);
    Ok(config)
}

/// Makes `folder` with its parents.
fn make_folder(folder: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o755);
    builder.create(folder)
}

/// What the introduction socket answers `index` with: the host's id and method and its entries, each as `/canoa.json` lists
/// it but with no address, which the portal gives.
pub(crate) fn index_of(peer: &str, method: &str, entries: &[crate::serve::Listening]) -> String {
    let entries: Vec<_> = entries
        .iter()
        .map(|entry| {
            let mut line = entry.to_json();
            line.remove("url");
            serde_json::Value::Object(line)
        })
        .collect();
    serde_json::json!({"peer": peer, "method": method, "os": std::env::consts::OS, "entries": entries})
        .to_string()
}

/// Listens on the introduction socket until `stopped`: `{"op":"index"}` answers the host's entries,
/// `{"op":"accept","peer":ID}` accepts a key that waits in the trust store, and `{"op":"revoke","peer":ID}` removes one. A
/// caller is let in only when the system says it runs as the portal's user; the keys it accepts still prove themselves on every
/// connection.
///
/// # Errors
///
/// When the socket cannot be bound.
pub(crate) fn introduce(
    sockets: &PortalSockets,
    store: Arc<TrustStore>,
    index: String,
    stopped: Arc<AtomicBool>,
) -> Result<(), ServeError> {
    let name = PipeName::at(sockets.dir.join(INTRODUCTION)).map_err(|error| ServeError::Pipe {
        entry: "portal".to_owned(),
        error,
    })?;
    let listener = sockets.bind(&name).map_err(|error| ServeError::Pipe {
        entry: "portal".to_owned(),
        error,
    })?;
    let uid = sockets.uid;
    thread::Builder::new()
        .name("canoa-introduction".to_owned())
        .spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept_timeout(Some(REST)) {
                    Ok(stream) if from_user(&stream, uid) => answer(&stream, &store, &index),
                    Ok(_) => tracing::warn!(
                        "a connection to the introduction socket from another user was refused"
                    ),
                    Err(coqui_transport_pipe::Error::TimedOut) => {}
                    Err(error) => {
                        tracing::warn!("the introduction socket failed: {error}");
                        thread::sleep(REST);
                    }
                }
            }
        })
        .map_err(|error| ServeError::Transport {
            entry: "portal".to_owned(),
            what: error.to_string(),
        })?;
    Ok(())
}

fn answer(stream: &Stream, store: &TrustStore, index: &str) {
    use coqui_transport_pipe::ReadTimeout;
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(REQUEST_TIME));
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    // One request, a line of JSON.
    while request.len() < MAX_REQUEST {
        match stream.read(&mut byte) {
            Ok(1) if byte[0] != b'\n' => request.push(byte[0]),
            _ => break,
        }
    }
    let reply = respond(&request, store, index);
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.write_all(b"\n");
}

/// The line the introduction socket answers `request` with.
fn respond(request: &[u8], store: &TrustStore, index: &str) -> String {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(request) else {
        return serde_json::json!({"error": "not JSON"}).to_string();
    };
    let peer = request.get("peer").and_then(serde_json::Value::as_str);
    match (request.get("op").and_then(serde_json::Value::as_str), peer) {
        (Some("index"), _) => index.to_owned(),
        (Some("accept"), Some(peer)) => match peers::accept(store, peer) {
            Ok(id) => serde_json::json!({"ok": true, "peer": id.to_string()}).to_string(),
            Err(error) => serde_json::json!({"error": error.to_string()}).to_string(),
        },
        (Some("revoke"), Some(peer)) => match peers::revoke(store, peer) {
            Ok(found) => serde_json::json!({"ok": found}).to_string(),
            Err(error) => serde_json::json!({"error": error.to_string()}).to_string(),
        },
        _ => serde_json::json!({"error": "not an operation"}).to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

    use super::check_account_of;

    #[test]
    fn the_entries_of_a_portal_host_listen_on_its_sockets() {
        let dir = std::env::temp_dir().join(format!("canoa-adapt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("made");
        let text = "[host.agents.Claude]
harness = \"claude\"

[host.agents.Mine]
program = \"agent\"
listen = \"127.0.0.1:0\"
";
        let config = crate::config::Config::from_str_in(text, &dir).expect("parses");
        let sockets = super::PortalSockets::new(dir.clone(), 1000);
        let adapted = super::adapt(&config, &sockets).expect("adapted");
        for name in ["Claude", "Mine"] {
            let entry = adapted
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .expect(name);
            assert!(
                matches!(entry.listen, crate::config::Listen::Pipe(_)),
                "{name} listens on a socket of the portal"
            );
        }
        std::fs::remove_dir_all(&dir).expect("cleaned");
    }

    #[test]
    fn a_host_runs_for_a_portal_only_as_the_account_whose_folder_it_is_and_who_may_have_one() {
        let dir = Path::new("/run/canoa/alice");
        let team = vec!["canoa-users".to_owned()];
        let (mine, other) = (
            vec!["alice".to_owned(), "canoa-users".to_owned()],
            vec!["alice".to_owned()],
        );
        let ok = |uid, name, owner, min, own: &[String], allowed: &[String]| {
            check_account_of(uid, name, dir, owner, min, (own, allowed))
        };
        assert!(ok(1001, Some("alice"), Some(1001), 1000, &mine, &team).is_ok());
        assert!(
            ok(
                486_801_112,
                Some("alice"),
                Some(486_801_112),
                1000,
                &mine,
                &team
            )
            .is_ok(),
            "a directory's large ids are people"
        );
        for (uid, name, owner, min, own, allowed) in [
            (0, Some("alice"), Some(0), 1000, &mine, &team),
            (0, Some("root"), Some(0), 0, &mine, &team),
            (999, Some("alice"), Some(999), 1000, &mine, &team),
            (65534, Some("alice"), Some(65534), 1000, &mine, &team),
            (61184, Some("alice"), Some(61184), 1000, &mine, &team),
            (65535, Some("alice"), Some(65535), 1000, &mine, &team),
            (1001, Some("bob"), Some(1001), 1000, &mine, &team),
            (1001, None, Some(1001), 1000, &mine, &team),
            (1001, Some("alice"), Some(1002), 1000, &mine, &team),
            (1001, Some("alice"), None, 1000, &mine, &team),
            (1001, Some("alice"), Some(1001), 1000, &other, &team),
            (1001, Some("alice"), Some(1001), 1000, &mine, &Vec::new()),
        ] {
            assert!(
                ok(uid, name, owner, min, own, allowed).is_err(),
                "{uid} {name:?} {owner:?}"
            );
        }
        assert!(
            check_account_of(
                1001,
                Some("alice"),
                Path::new("/run/canoa/alice/"),
                Some(1001),
                1000,
                (&mine, &team)
            )
            .is_ok()
        );
    }
}
