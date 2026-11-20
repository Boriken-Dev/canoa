//! A package registry on loopback and the Node.js and npm to use it with, for the tests that install an overlay's npm
//! dependencies. Nothing leaves this computer: the registry serves the few packages a test names, packed by the real `npm pack`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;

use super::needs::Finder;

/// The Node.js the tests install with (`CANOA_TEST_NODE`, a program or a name on the `PATH`), or none. Its folder must hold `npm`
/// too.
pub(crate) fn node() -> Option<PathBuf> {
    let named = std::env::var_os("CANOA_TEST_NODE")?;
    let path = PathBuf::from(&named);
    if path.is_absolute() {
        return Some(path);
    }
    Finder::new(&|name| std::env::var(name).ok(), &[]).find(&named.to_string_lossy())
}

/// A finder whose path holds only the folder of `node`.
pub(crate) fn finder_for(node: &Path) -> Finder {
    let folder = node
        .parent()
        .expect("a folder")
        .to_string_lossy()
        .into_owned();
    let lookup = move |name: &str| match name {
        "PATH" => Some(folder.clone()),
        other => std::env::var(other).ok(),
    };
    Finder::new(&lookup, &[])
}

/// What a process that runs npm gets: `node`'s folder first on the `PATH`, an npm cache and configuration of the test's own in
/// `scratch` (the person's are never read), and `registry` as the registry when one is given.
pub(crate) fn npm_env(
    node: &Path,
    scratch: &Path,
    registry: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let folder = node.parent().expect("a folder").to_path_buf();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(folder).chain(std::env::split_paths(&inherited)))
            .expect("a path");
    std::fs::create_dir_all(scratch).expect("a folder");
    let (user, global) = (scratch.join("npmrc"), scratch.join("npmrc-global"));
    for file in [&user, &global] {
        std::fs::write(file, "").expect("an empty configuration");
    }
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), path),
        ("NPM_CONFIG_CACHE".into(), scratch.join("cache").into()),
        ("NPM_CONFIG_USERCONFIG".into(), user.into()),
        ("NPM_CONFIG_GLOBALCONFIG".into(), global.into()),
        ("NPM_CONFIG_UPDATE_NOTIFIER".into(), "false".into()),
        ("NPM_CONFIG_NOPROXY".into(), "127.0.0.1".into()),
    ];
    if let Some(registry) = registry {
        env.push(("NPM_CONFIG_REGISTRY".into(), registry.into()));
    }
    env
}

/// Packs the package `name` at `version`, whose module exports its name and version, as `npm pack` makes it.
fn pack(node: &Path, scratch: &Path, name: &str, version: &str) -> Vec<u8> {
    let source = scratch.join("source").join(format!("{name}-{version}"));
    std::fs::create_dir_all(&source).expect("a folder");
    std::fs::write(
        source.join("package.json"),
        format!("{{\"name\":\"{name}\",\"version\":\"{version}\",\"main\":\"index.js\"}}\n"),
    )
    .expect("package.json");
    std::fs::write(
        source.join("index.js"),
        format!("module.exports = {{ name: \"{name}\", version: \"{version}\" }};\n"),
    )
    .expect("index.js");
    let packed = scratch.join("packed");
    std::fs::create_dir_all(&packed).expect("a folder");
    let npm = finder_for(node).find("npm").expect("npm beside node");
    let done = crate::spawn::Command::new(&npm)
        .arg("pack")
        .arg(&source)
        .arg("--pack-destination")
        .arg(&packed)
        .args(["--ignore-scripts", "--silent"])
        .envs(npm_env(node, &scratch.join("pack-npm"), None))
        .output()
        .expect("npm pack starts");
    assert!(
        done.status.success(),
        "{npm:?} pack {}: {}{}",
        done.status,
        String::from_utf8_lossy(&done.stdout),
        String::from_utf8_lossy(&done.stderr)
    );
    std::fs::read(packed.join(format!("{name}-{version}.tgz"))).expect("the tarball")
}

/// The packages a registry serves, each name's versions in order, and their tarballs.
type Packages = BTreeMap<String, Vec<(String, Vec<u8>)>>;

/// A registry on `127.0.0.1` that serves some packages and answers 404 to the rest, until it is dropped.
pub(crate) struct Registry {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Registry {
    /// Serves the `(name, version)` packages, packed with `node`'s npm in folders under `scratch`.
    pub(crate) fn start(node: &Path, scratch: &Path, wanted: &[(&str, &str)]) -> Self {
        let mut packages = Packages::new();
        for (name, version) in wanted {
            let tarball = pack(node, scratch, name, version);
            packages
                .entry((*name).to_owned())
                .or_default()
                .push(((*version).to_owned(), tarball));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
        listener.set_nonblocking(true).expect("non-blocking");
        let port = listener.local_addr().expect("an address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let packages = Arc::new(packages);
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let packages = Arc::clone(&packages);
                            std::thread::spawn(move || answer(stream, &packages, port));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })
        };
        Self {
            port,
            stop,
            thread: Some(thread),
        }
    }

    /// The registry's address, as `NPM_CONFIG_REGISTRY` takes it.
    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Where the tarball of `name` at `version` is, as a lock file's `resolved` names it.
    pub(crate) fn tarball(&self, name: &str, version: &str) -> String {
        tarball_url(self.port, name, version)
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn tarball_url(port: u16, name: &str, version: &str) -> String {
    format!("http://127.0.0.1:{port}/{name}/-/{name}-{version}.tgz")
}

/// Answers one request: a package's metadata, a tarball, or 404.
fn answer(mut stream: TcpStream, packages: &Packages, port: u16) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") && head.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let target = head.split_whitespace().nth(1).unwrap_or("/");
    let path = target
        .split('?')
        .next()
        .unwrap_or("/")
        .trim_start_matches('/');
    let (status, kind, body) = route(packages, port, path);
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

fn route(packages: &Packages, port: u16, path: &str) -> (&'static str, &'static str, Vec<u8>) {
    if let Some(versions) = packages.get(path) {
        let table: serde_json::Map<String, serde_json::Value> = versions
            .iter()
            .map(|(version, _)| {
                (
                    version.clone(),
                    json!({
                        "name": path,
                        "version": version,
                        "dist": {"tarball": tarball_url(port, path, version)},
                    }),
                )
            })
            .collect();
        let latest = versions.last().map(|(version, _)| version.as_str());
        let metadata = json!({"name": path, "dist-tags": {"latest": latest}, "versions": table});
        return (
            "200 OK",
            "application/json",
            metadata.to_string().into_bytes(),
        );
    }
    for (name, versions) in packages {
        for (version, tarball) in versions {
            if path == format!("{name}/-/{name}-{version}.tgz") {
                return ("200 OK", "application/octet-stream", tarball.clone());
            }
        }
    }
    (
        "404 Not Found",
        "application/json",
        br#"{"error":"Not found"}"#.to_vec(),
    )
}
