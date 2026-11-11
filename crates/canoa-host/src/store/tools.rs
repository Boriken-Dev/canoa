//! [`LocalTools`]: the host's [`InstallTools`], on this machine. A store never opens a connection or writes a file by
//! itself; it asks for each here, and each is checked on the way: the origin of every URL and redirect, the digest of every
//! download as it is written, and every path of an archive before a byte of it is.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and sockets are std's"
)]

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use canoa::store::{
    Check, FetchRequest, Fetched, FindRequest, Found, InstallTools, UnpackRequest, Unpacked,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use sha2::{Digest, Sha256, Sha512};

/// Limits a host puts on what a store fetches and unpacks. Each has a default and no floor.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ToolsLimits {
    /// How many redirects one fetch follows.
    pub redirects: u32,
    /// How long a request waits for the publisher's answer.
    pub response_timeout: Duration,
    /// How long a body may take, whole.
    pub body_timeout: Duration,
    /// The most files an unpack writes unless the request asks for less.
    pub unpack_files: u64,
    /// The most bytes an unpack writes unless the request asks for less.
    pub unpack_bytes: u64,
}

impl Default for ToolsLimits {
    fn default() -> Self {
        Self {
            redirects: 5,
            response_timeout: Duration::from_secs(60),
            body_timeout: Duration::from_secs(30 * 60),
            unpack_files: 100_000,
            unpack_bytes: 4 << 30,
        }
    }
}

/// One answer to a request, from a [`Transport`].
#[non_exhaustive]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The `Location` header of a redirect.
    pub location: Option<String>,
    /// The body.
    pub body: Box<dyn Read + Send>,
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reply")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Fetches one URL, without following a redirect: [`LocalTools`] follows each hop itself, so it can check its origin.
pub trait Transport: Send + Sync + fmt::Debug {
    /// Asks for `url`.
    ///
    /// # Errors
    ///
    /// The reason the request did not get an answer.
    fn get(&self, url: &str, limits: &ToolsLimits) -> Result<Reply, String>;
}

/// Whose certificates an HTTPS fetch trusts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Roots {
    /// The operating system's verifier and store, so a proxy that intercepts TLS and that the system trusts works.
    #[default]
    Platform,
    /// The roots webpki ships, whatever the system trusts.
    Webpki,
}

/// [`Transport`] over HTTPS with `ureq`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpTransport {
    roots: Roots,
}

impl HttpTransport {
    /// Trusts `roots`.
    #[must_use]
    pub const fn with_roots(roots: Roots) -> Self {
        Self { roots }
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str, limits: &ToolsLimits) -> Result<Reply, String> {
        let root_certs = match self.roots {
            Roots::Platform => ureq::tls::RootCerts::PlatformVerifier,
            _ => ureq::tls::RootCerts::WebPki,
        };
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(root_certs)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_recv_response(Some(limits.response_timeout))
            .timeout_recv_body(Some(limits.body_timeout))
            .build()
            .into();
        let response = agent.get(url).call().map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok(Reply {
            status,
            location,
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

/// How `find` and `~` read the environment of the program the store is for, which is not always this process's.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    /// The `PATH` to search; only its absolute entries are read.
    pub path: OsString,
    /// What `~` means.
    pub home: Option<PathBuf>,
    /// The variables `%NAME%` and `$NAME` open a directory with.
    pub variables: BTreeMap<String, String>,
}

impl Lookup {
    /// This process's own `PATH`, home and variables.
    #[must_use]
    pub fn process() -> Self {
        Self {
            path: std::env::var_os("PATH").unwrap_or_default(),
            home: std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from),
            variables: std::env::vars().collect(),
        }
    }
}

/// A file a fetch wrote: where from, where to and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FetchRecord {
    /// The URL.
    pub url: String,
    /// The path inside the granted folder.
    pub path: String,
    /// Its SHA-256.
    pub sha256: String,
}

/// The host's [`InstallTools`] on this machine.
#[must_use]
pub struct LocalTools {
    fetched: Arc<Mutex<Vec<FetchRecord>>>,
    urls: Arc<Mutex<Vec<String>>>,
    transport: Arc<dyn Transport>,
    endpoints: Vec<String>,
    granted: Option<PathBuf>,
    lookup: Lookup,
    limits: ToolsLimits,
    loopback_http: bool,
    allow_unverified: bool,
    cancel: Arc<AtomicBool>,
    deadline: Option<std::time::Instant>,
}

impl fmt::Debug for LocalTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTools")
            .field("endpoints", &self.endpoints)
            .field("granted", &self.granted)
            .finish_non_exhaustive()
    }
}

impl LocalTools {
    /// Tools that fetch from `endpoints` (origins such as `https://downloads.example`) over `transport`.
    pub fn new(transport: Arc<dyn Transport>, endpoints: Vec<String>) -> Self {
        Self {
            fetched: Arc::new(Mutex::new(Vec::new())),
            urls: Arc::new(Mutex::new(Vec::new())),
            transport,
            endpoints,
            granted: None,
            lookup: Lookup::process(),
            limits: ToolsLimits::default(),
            loopback_http: false,
            allow_unverified: false,
            cancel: Arc::new(AtomicBool::new(false)),
            deadline: None,
        }
    }

    /// Makes every call answer `GUATIAO_ERR_GONE` with `install timed out` once `deadline` has passed.
    pub const fn deadline(mut self, deadline: std::time::Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Every file fetched so far, kept by a handle that outlives the tools.
    #[must_use]
    pub fn fetch_log(&self) -> Arc<Mutex<Vec<FetchRecord>>> {
        Arc::clone(&self.fetched)
    }

    /// Every URL asked for so far, redirects included, kept by a handle that outlives the tools.
    #[must_use]
    pub fn url_log(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.urls)
    }

    /// The folder `fetch` and `unpack` may write in, which must exist.
    pub fn granted(mut self, dir: impl Into<PathBuf>) -> Self {
        self.granted = Some(dir.into());
        self
    }

    /// What `find` reads instead of this process's environment.
    pub fn lookup(mut self, lookup: Lookup) -> Self {
        self.lookup = lookup;
        self
    }

    /// The limits.
    pub fn limits(mut self, limits: ToolsLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Lets `http` reach a loopback address. A test's setting: a download over it is never safe.
    pub const fn loopback_http(mut self, allowed: bool) -> Self {
        self.loopback_http = allowed;
        self
    }

    /// Lets `fetch` take a download nothing checks (`Check::Unverified`). Off by default: the person allows it, knowing the plan
    /// said which downloads those are.
    pub const fn allow_unverified(mut self, allowed: bool) -> Self {
        self.allow_unverified = allowed;
        self
    }

    /// A flag that, once set, makes every call answer `GUATIAO_ERR_GONE`.
    pub fn cancelled_by(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = cancel;
        self
    }

    fn gone(&self) -> Result<(), ProviderError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(refused(Status::GUATIAO_ERR_GONE, "install cancelled"));
        }
        if self
            .deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
        {
            return Err(refused(Status::GUATIAO_ERR_GONE, "install timed out"));
        }
        Ok(())
    }

    fn within_granted(&self, relative: &str) -> Result<PathBuf, ProviderError> {
        let root = self.granted.as_ref().ok_or_else(|| {
            refused(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "this host granted no folder",
            )
        })?;
        Ok(root.join(relative_path(relative)?))
    }

    /// Opens `url`, following redirects while each hop stays in an endpoint.
    fn open(&self, url: &str) -> Result<Reply, ProviderError> {
        let mut current = url.to_owned();
        for _ in 0..=self.limits.redirects {
            self.gone()?;
            self.allowed(&current)?;
            if let Ok(mut urls) = self.urls.lock() {
                urls.push(current.clone());
            }
            let reply = self.transport.get(&current, &self.limits).map_err(|why| {
                refused(
                    Status::GUATIAO_ERR_INTERNAL,
                    &format!("download failed: {why}"),
                )
            })?;
            match reply.status {
                200 => return Ok(reply),
                301 | 302 | 303 | 307 | 308 => {
                    let next = reply.location.ok_or_else(|| {
                        refused(Status::GUATIAO_ERR_INTERNAL, "a redirect with no location")
                    })?;
                    current = join_url(&current, &next);
                }
                other => {
                    return Err(refused(
                        Status::GUATIAO_ERR_INTERNAL,
                        &format!("download failed: the server answered {other}"),
                    ));
                }
            }
        }
        Err(refused(Status::GUATIAO_ERR_INTERNAL, "too many redirects"))
    }

    fn allowed(&self, url: &str) -> Result<(), ProviderError> {
        let (scheme, host_port) = split_origin(url)
            .ok_or_else(|| refused(Status::GUATIAO_ERR_BAD_VALUE, "a URL this host cannot read"))?;
        let origin = format!("{scheme}://{host_port}");
        let host = host_of(&host_port);
        let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
        if scheme != "https" && !(scheme == "http" && self.loopback_http && loopback) {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                "only https is fetched",
            ));
        }
        let reaches = |endpoint: &String| {
            let after_scheme = endpoint.split_once("://").map_or("", |(_, rest)| rest);
            if after_scheme.contains('/') {
                // An endpoint with a path is a prefix: the bucket, not the whole host.
                url.starts_with(endpoint.as_str()) && !url[endpoint.len()..].contains("..")
            } else {
                endpoint.eq_ignore_ascii_case(&origin)
            }
        };
        if !self.endpoints.iter().any(reaches) {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("{origin} is not one of the store's endpoints"),
            ));
        }
        Ok(())
    }
}

fn refused(status: Status, message: &str) -> ProviderError {
    ProviderError::new(status, message)
}

/// `(scheme, host[:port])` of an absolute URL.
fn split_origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), authority.to_ascii_lowercase()))
}

fn host_of(authority: &str) -> String {
    if authority.starts_with('[') {
        return authority
            .find(']')
            .map_or_else(|| authority.to_owned(), |end| authority[..=end].to_owned());
    }
    authority.split(':').next().unwrap_or(authority).to_owned()
}

/// A redirect's target against the URL it came from.
fn join_url(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_owned();
    }
    let Some((scheme, authority)) = split_origin(base) else {
        return location.to_owned();
    };
    if location.starts_with('/') {
        return format!("{scheme}://{authority}{location}");
    }
    let dir = base.rsplit_once('/').map_or(base, |(dir, _)| dir);
    format!("{dir}/{location}")
}

/// A relative path with no part that leaves its folder.
fn relative_path(text: &str) -> Result<PathBuf, ProviderError> {
    let bad = || {
        refused(
            Status::GUATIAO_ERR_BAD_VALUE,
            "a path that leaves its folder",
        )
    };
    if text.is_empty() || text.starts_with('/') || text.contains('\\') {
        return Err(bad());
    }
    let mut out = PathBuf::new();
    for part in text.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if !safe_part(part) {
            return Err(bad());
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err(bad());
    }
    Ok(out)
}

/// Whether `part` is one plain name: not `..`, no separator or drive, not a device name.
fn safe_part(part: &str) -> bool {
    if part == ".." || part.contains(['/', '\\', ':', '\0']) {
        return false;
    }
    let stem = part.split('.').next().unwrap_or(part).to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            matches!(
                chars.next(),
                Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}')
            ) && chars.next().is_none()
        })
    };
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT");
    !device && !part.ends_with(' ') && !part.ends_with('.')
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// What a download is hashed with, as it is written.
#[allow(
    clippy::large_enum_variant,
    reason = "one lives on the stack for one download"
)]
enum Hashes {
    Plain(Sha256),
    Both(Sha256, Sha512),
}

impl Hashes {
    fn for_check(check: &Check) -> Self {
        match check {
            Check::Sha512 { .. } => Self::Both(Sha256::new(), Sha512::new()),
            _ => Self::Plain(Sha256::new()),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Plain(a) => a.update(bytes),
            Self::Both(a, b) => {
                a.update(bytes);
                b.update(bytes);
            }
        }
    }

    fn finish(self, check: &Check) -> (String, bool) {
        match (self, check) {
            (Self::Plain(a), Check::Sha256 { value }) => {
                let sha = hex(&a.finalize());
                let ok = sha.eq_ignore_ascii_case(value);
                (sha, ok)
            }
            (Self::Both(a, b), Check::Sha512 { value }) => {
                let ok = hex(&b.finalize()).eq_ignore_ascii_case(value);
                (hex(&a.finalize()), ok)
            }
            (Self::Plain(a), _) => (hex(&a.finalize()), true),
            (Self::Both(a, _), _) => (hex(&a.finalize()), true),
        }
    }
}

impl InstallTools for LocalTools {
    fn find(&self, request: FindRequest) -> Result<Vec<Found>, ProviderError> {
        self.gone()?;
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&self.lookup.path)
            .filter(|dir| dir.is_absolute())
            .collect();
        for text in &request.dirs {
            if let Some(dir) = expand(text, &self.lookup)
                && dir.is_absolute()
            {
                dirs.push(dir);
            }
        }
        let extensions: &[&str] = if cfg!(windows) {
            &["", ".exe", ".cmd", ".bat", ".com"]
        } else {
            &[""]
        };
        let mut found = Vec::new();
        for dir in dirs {
            for name in &request.names {
                if !safe_part(name) {
                    continue;
                }
                for extension in extensions {
                    let path = dir.join(format!("{name}{extension}"));
                    let Ok(meta) = fs::metadata(&path) else {
                        continue;
                    };
                    if !meta.is_file() {
                        continue;
                    }
                    let text = path.to_string_lossy().into_owned();
                    if found.iter().any(|f: &Found| f.path == text) {
                        continue;
                    }
                    let modified_ms = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
                    found.push(Found::new(text, meta.len(), modified_ms));
                }
            }
        }
        Ok(found)
    }

    fn fetch_text(&self, url: &str, max_bytes: u64) -> Result<String, ProviderError> {
        let reply = self.open(url)?;
        let mut body = Vec::new();
        reply
            .body
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|e| {
                refused(
                    Status::GUATIAO_ERR_INTERNAL,
                    &format!("download failed: {e}"),
                )
            })?;
        if body.len() as u64 > max_bytes {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                "the document is larger than the limit",
            ));
        }
        String::from_utf8(body)
            .map_err(|_| refused(Status::GUATIAO_ERR_BAD_VALUE, "the document is not text"))
    }

    fn fetch(&self, request: FetchRequest) -> Result<Fetched, ProviderError> {
        let target = self.within_granted(&request.path)?;
        if matches!(request.check, Check::Unverified { .. }) && !self.allow_unverified {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                "a download nothing checks is not allowed here",
            ));
        }
        let mut reply = self.open(&request.url)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(&e))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| io_error(&e))?;
        let mut hashes = Hashes::for_check(&request.check);
        let mut written: u64 = 0;
        let mut buffer = vec![0u8; 64 * 1024];
        let outcome: Result<(), ProviderError> = loop {
            if let Err(error) = self.gone() {
                break Err(error);
            }
            let count = match reply.body.read(&mut buffer) {
                Ok(0) => break Ok(()),
                Ok(count) => count,
                Err(error) => {
                    break Err(refused(
                        Status::GUATIAO_ERR_INTERNAL,
                        &format!("download failed: {error}"),
                    ));
                }
            };
            written += count as u64;
            if written > request.max_bytes {
                break Err(refused(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    "the download is larger than the limit",
                ));
            }
            hashes.update(&buffer[..count]);
            if let Err(error) = file.write_all(&buffer[..count]) {
                break Err(io_error(&error));
            }
        };
        drop(file);
        if let Err(error) = outcome {
            let _ = fs::remove_file(&target);
            return Err(error);
        }
        let (sha256, matches) = hashes.finish(&request.check);
        if !matches {
            let _ = fs::remove_file(&target);
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("checksum mismatch: {}", request.url),
            ));
        }
        if request.executable {
            make_executable(&target).map_err(|e| io_error(&e))?;
        }
        if let Ok(mut fetched) = self.fetched.lock() {
            fetched.push(FetchRecord {
                url: request.url.clone(),
                path: request.path.clone(),
                sha256: sha256.clone(),
            });
        }
        Ok(Fetched::new(written, sha256))
    }

    fn unpack(&self, request: UnpackRequest) -> Result<Unpacked, ProviderError> {
        self.gone()?;
        let archive = self.within_granted(&request.archive)?;
        let into = self.within_granted(&request.into)?;
        let mut limits = Limits {
            files: request.max_files.min(self.limits.unpack_files),
            bytes: request.max_bytes.min(self.limits.unpack_bytes),
            used_files: 0,
            used_bytes: 0,
            strip: request.strip as usize,
            skip_links: request.skip_links,
        };
        fs::create_dir_all(&into).map_err(|e| io_error(&e))?;
        match request.format.as_str() {
            "zip" => unpack_zip(&archive, &into, &mut limits, &self.cancel)?,
            "tar" => unpack_tar(
                File::open(&archive).map_err(|e| io_error(&e))?,
                &into,
                &mut limits,
                &self.cancel,
            )?,
            "tar.gz" => unpack_tar(
                flate2::read::GzDecoder::new(File::open(&archive).map_err(|e| io_error(&e))?),
                &into,
                &mut limits,
                &self.cancel,
            )?,
            other => {
                return Err(refused(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    &format!("an archive of format {other}"),
                ));
            }
        }
        if request.remove_archive {
            fs::remove_file(&archive).map_err(|e| io_error(&e))?;
        }
        Ok(Unpacked::new(limits.used_files, limits.used_bytes))
    }
}

fn io_error(error: &io::Error) -> ProviderError {
    refused(Status::GUATIAO_ERR_INTERNAL, &error.to_string())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// What an unpack may still write.
struct Limits {
    files: u64,
    bytes: u64,
    used_files: u64,
    used_bytes: u64,
    strip: usize,
    skip_links: bool,
}

/// Where the entry `name` goes under `into`, or `None` when stripping leaves nothing.
/// How deep an archive's entry may be nested.
const MAX_DEPTH: usize = 64;

fn entry_target(name: &str, into: &Path, strip: usize) -> Result<Option<PathBuf>, ProviderError> {
    let bad = || {
        refused(
            Status::GUATIAO_ERR_BAD_VALUE,
            "an archive entry that leaves its folder",
        )
    };
    if name.starts_with('/') || name.starts_with('\\') || name.contains('\\') {
        return Err(bad());
    }
    let parts: Vec<&str> = name
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.iter().any(|part| !safe_part(part)) || parts.len() > MAX_DEPTH {
        return Err(bad());
    }
    if parts.len() <= strip {
        return Ok(None);
    }
    let mut path = into.to_path_buf();
    for part in &parts[strip..] {
        path.push(part);
    }
    Ok(Some(path))
}

fn copy_limited(
    from: &mut dyn Read,
    to: &mut File,
    limits: &mut Limits,
    cancel: &AtomicBool,
) -> Result<(), ProviderError> {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(refused(Status::GUATIAO_ERR_GONE, "cancelled"));
        }
        let count = from.read(&mut buffer).map_err(|e| io_error(&e))?;
        if count == 0 {
            return Ok(());
        }
        limits.used_bytes += count as u64;
        if limits.used_bytes > limits.bytes {
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                "the archive unpacks to more than the limit",
            ));
        }
        to.write_all(&buffer[..count]).map_err(|e| io_error(&e))?;
    }
}

fn count_file(limits: &mut Limits) -> Result<(), ProviderError> {
    limits.used_files += 1;
    if limits.used_files > limits.files {
        return Err(refused(
            Status::GUATIAO_ERR_BAD_VALUE,
            "the archive holds more files than the limit",
        ));
    }
    Ok(())
}

fn unpack_tar<R: Read>(
    reader: R,
    into: &Path,
    limits: &mut Limits,
    cancel: &AtomicBool,
) -> Result<(), ProviderError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| io_error(&e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| io_error(&e))?;
        let kind = entry.header().entry_type();
        let name = entry
            .path()
            .map_err(|e| io_error(&e))?
            .to_string_lossy()
            .into_owned();
        use tar::EntryType as T;
        match kind {
            T::XGlobalHeader | T::XHeader | T::GNULongName | T::GNULongLink => continue,
            T::Symlink | T::Link if limits.skip_links => continue,
            T::Regular | T::Continuous | T::Directory => {}
            _ => {
                return Err(refused(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &format!("an archive entry that is a link or a device: {name}"),
                ));
            }
        }
        let Some(target) = entry_target(&name, into, limits.strip)? else {
            continue;
        };
        if kind == T::Directory {
            count_file(limits)?;
            fs::create_dir_all(&target).map_err(|e| io_error(&e))?;
            continue;
        }
        count_file(limits)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(&e))?;
        }
        let mode = entry.header().mode().unwrap_or(0o644);
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| io_error(&e))?;
        copy_limited(&mut entry, &mut file, limits, cancel)?;
        drop(file);
        if mode & 0o111 != 0 {
            make_executable(&target).map_err(|e| io_error(&e))?;
        }
    }
    Ok(())
}

fn unpack_zip(
    archive: &Path,
    into: &Path,
    limits: &mut Limits,
    cancel: &AtomicBool,
) -> Result<(), ProviderError> {
    let file = File::open(archive).map_err(|e| io_error(&e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        refused(
            Status::GUATIAO_ERR_BAD_VALUE,
            &format!("not a zip archive: {e}"),
        )
    })?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| {
            refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("a bad zip entry: {e}"),
            )
        })?;
        let name = entry.name().to_owned();
        let link = entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000);
        if link {
            if limits.skip_links {
                continue;
            }
            return Err(refused(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("an archive entry that is a link: {name}"),
            ));
        }
        let Some(target) = entry_target(name.trim_end_matches('/'), into, limits.strip)? else {
            continue;
        };
        if entry.is_dir() {
            count_file(limits)?;
            fs::create_dir_all(&target).map_err(|e| io_error(&e))?;
            continue;
        }
        count_file(limits)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(&e))?;
        }
        let mode = entry.unix_mode().unwrap_or(0o644);
        let mut out = File::options()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| io_error(&e))?;
        copy_limited(&mut entry, &mut out, limits, cancel)?;
        drop(out);
        if mode & 0o111 != 0 {
            make_executable(&target).map_err(|e| io_error(&e))?;
        }
    }
    Ok(())
}

/// A directory text that may open with `~` or a variable.
fn expand(text: &str, lookup: &Lookup) -> Option<PathBuf> {
    let mut out = String::new();
    let mut rest = text;
    if let Some(after) = rest.strip_prefix('~') {
        out.push_str(&lookup.home.as_ref()?.to_string_lossy());
        rest = after;
    }
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                let mut name = String::new();
                for n in chars.by_ref() {
                    if n == '%' {
                        break;
                    }
                    name.push(n);
                }
                out.push_str(variable(lookup, &name)?);
            }
            '$' => {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push_str(variable(lookup, &name)?);
            }
            other => out.push(other),
        }
    }
    Some(PathBuf::from(out))
}

fn variable<'a>(lookup: &'a Lookup, name: &str) -> Option<&'a str> {
    lookup
        .variables
        .iter()
        .find(|(key, _)| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name)
            } else {
                key.as_str() == name
            }
        })
        .map(|(_, value)| value.as_str())
}

/// Whether `path` lies inside `root`, by its parts alone (neither need exist).
#[must_use]
pub fn is_inside(path: &Path, root: &Path) -> bool {
    let normal = |p: &Path| -> Option<Vec<String>> {
        let mut parts = Vec::new();
        for component in p.components() {
            match component {
                Component::ParentDir => return None,
                Component::CurDir => {}
                other => parts.push(other.as_os_str().to_string_lossy().to_ascii_lowercase()),
            }
        }
        Some(parts)
    };
    match (normal(path), normal(root)) {
        (Some(path), Some(root)) => path.len() > root.len() && path[..root.len()] == root[..],
        _ => false,
    }
}
