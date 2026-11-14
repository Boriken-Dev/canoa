//! The portal's settings: a TOML file of its own, every one with a default.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;
use std::path::PathBuf;

use serde::Deserialize;

/// How the portal runs. `Settings::default()` is what runs with no file; each field is the file's key of the same name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// `listen`: where the portal takes browsers, loopback only: a proxy that speaks TLS stands in front of it, since the
    /// session cookie is `Secure`.
    pub listen: SocketAddr,
    /// `site`: the web app's build, the files a signed-in browser is given. Required.
    pub site: Option<PathBuf>,
    /// `socket_dir`: where each user's host keeps its sockets, `DIR/USER`.
    pub socket_dir: PathBuf,
    /// `unit`: the template unit of a user's host, `{user}` standing for the account.
    pub unit: String,
    /// `systemctl`: the program that starts and stops it.
    pub systemctl: PathBuf,
    /// `elevate`: a command put before `systemctl` to start and stop, for a machine with no polkit rule.
    pub elevate: Vec<String>,
    /// `pam_service`: the PAM service that checks a password.
    pub pam_service: String,
    /// `min_uid`: the lowest user id that may sign in.
    pub min_uid: u32,
    /// `groups`: when any are named, the groups an account must be in.
    pub groups: Vec<String>,
    /// `hosts`: names a request may carry in `Host`, besides `localhost` and addresses; an `Origin` must match `Host`.
    pub hosts: Vec<String>,
    /// `trusted_proxies`: the addresses whose `client_address_header` is believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// `client_address_header`: the header the proxy puts the browser's address in; the last address in it that is not a
    /// trusted proxy is the browser's.
    pub client_address_header: String,
    /// `session_seconds`: how long a sign-in lasts.
    pub session: Duration,
    /// `idle_seconds`: how long a host nobody uses runs on.
    pub idle: Duration,
    /// `start_seconds`: how long a started host has to answer.
    pub start: Duration,
    /// `pair_seconds`: how long an introduced key is waited for.
    pub pair: Duration,
    /// `max_failures_per_address`: failed sign-ins an address may make in the window.
    pub max_failures_per_address: usize,
    /// `max_failures_per_user`: failed sign-ins for one name in the window.
    pub max_failures_per_user: usize,
    /// `failure_window_seconds`: the window.
    pub failure_window: Duration,
    /// `max_connections`: browsers served at once.
    pub max_connections: usize,
    /// `max_sign_ins`: password checks under way at once.
    pub max_sign_ins: usize,
    /// `max_sessions`: sign-ins kept at once.
    pub max_sessions: usize,
    /// `max_relays_per_user`: channels one user may hold open.
    pub max_relays_per_user: usize,
    /// `request_seconds`: how long a request has to arrive.
    pub request: Duration,
    /// `keep_alive_seconds`: how long an idle connection is kept.
    pub keep_alive: Duration,
    /// `max_requests`: requests one connection serves.
    pub max_requests: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7890),
            site: None,
            socket_dir: PathBuf::from("/run/canoa"),
            unit: "canoa@{user}.service".to_owned(),
            systemctl: PathBuf::from("systemctl"),
            elevate: Vec::new(),
            pam_service: "canoa".to_owned(),
            min_uid: 1000,
            groups: Vec::new(),
            hosts: Vec::new(),
            trusted_proxies: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
            client_address_header: "x-forwarded-for".to_owned(),
            session: Duration::from_secs(8 * 3600),
            idle: Duration::from_secs(30 * 60),
            start: Duration::from_secs(20),
            pair: Duration::from_secs(8),
            max_failures_per_address: 5,
            max_failures_per_user: 10,
            failure_window: Duration::from_secs(15 * 60),
            max_connections: 256,
            max_sign_ins: 8,
            max_sessions: 1024,
            max_relays_per_user: 16,
            request: Duration::from_secs(10),
            keep_alive: Duration::from_secs(30),
            max_requests: 1000,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: Option<String>,
    site: Option<PathBuf>,
    socket_dir: Option<PathBuf>,
    unit: Option<String>,
    systemctl: Option<PathBuf>,
    elevate: Option<Vec<String>>,
    pam_service: Option<String>,
    min_uid: Option<u32>,
    groups: Option<Vec<String>>,
    hosts: Option<Vec<String>>,
    trusted_proxies: Option<Vec<String>>,
    client_address_header: Option<String>,
    session_seconds: Option<u64>,
    idle_seconds: Option<u64>,
    start_seconds: Option<u64>,
    pair_seconds: Option<u64>,
    max_failures_per_address: Option<usize>,
    max_failures_per_user: Option<usize>,
    failure_window_seconds: Option<u64>,
    max_connections: Option<usize>,
    max_sign_ins: Option<usize>,
    max_sessions: Option<usize>,
    max_relays_per_user: Option<usize>,
    request_seconds: Option<u64>,
    keep_alive_seconds: Option<u64>,
    max_requests: Option<usize>,
}

impl Settings {
    /// The settings a file's text says, the rest at their defaults.
    ///
    /// # Errors
    ///
    /// A message naming the key, never its value, for text that is not TOML, a key the portal does not know, a `listen` beyond
    /// loopback, a unit with no `{user}`, or a limit of zero.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let raw: Raw = toml::from_str(text).map_err(|error| error.message().to_owned())?;
        let mut settings = Self::default();
        if let Some(listen) = raw.listen {
            settings.listen = listen
                .parse()
                .map_err(|_| "listen: not an ADDRESS:PORT".to_owned())?;
        }
        settings.site = raw.site.or(settings.site);
        if let Some(value) = raw.socket_dir {
            settings.socket_dir = value;
        }
        if let Some(value) = raw.unit {
            settings.unit = value;
        }
        if let Some(value) = raw.systemctl {
            settings.systemctl = value;
        }
        settings.elevate = raw.elevate.unwrap_or(settings.elevate);
        if let Some(value) = raw.pam_service {
            settings.pam_service = value;
        }
        if let Some(value) = raw.min_uid {
            settings.min_uid = value;
        }
        settings.groups = raw.groups.unwrap_or(settings.groups);
        settings.hosts = raw
            .hosts
            .unwrap_or(settings.hosts)
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        if let Some(proxies) = raw.trusted_proxies {
            settings.trusted_proxies = proxies
                .iter()
                .map(|text| text.parse())
                .collect::<Result<_, _>>()
                .map_err(|_| "trusted_proxies: not an address".to_owned())?;
        }
        if let Some(value) = raw.client_address_header {
            settings.client_address_header = value.to_ascii_lowercase();
        }
        let seconds = |value: Option<u64>, now: Duration| value.map_or(now, Duration::from_secs);
        settings.session = seconds(raw.session_seconds, settings.session);
        settings.idle = seconds(raw.idle_seconds, settings.idle);
        settings.start = seconds(raw.start_seconds, settings.start);
        settings.pair = seconds(raw.pair_seconds, settings.pair);
        settings.failure_window = seconds(raw.failure_window_seconds, settings.failure_window);
        settings.request = seconds(raw.request_seconds, settings.request);
        settings.keep_alive = seconds(raw.keep_alive_seconds, settings.keep_alive);
        let count = |value: Option<usize>, now: usize| value.unwrap_or(now);
        settings.max_failures_per_address = count(
            raw.max_failures_per_address,
            settings.max_failures_per_address,
        );
        settings.max_failures_per_user =
            count(raw.max_failures_per_user, settings.max_failures_per_user);
        settings.max_connections = count(raw.max_connections, settings.max_connections);
        settings.max_sign_ins = count(raw.max_sign_ins, settings.max_sign_ins);
        settings.max_sessions = count(raw.max_sessions, settings.max_sessions);
        settings.max_relays_per_user = count(raw.max_relays_per_user, settings.max_relays_per_user);
        settings.max_requests = count(raw.max_requests, settings.max_requests);
        settings.check()?;
        Ok(settings)
    }

    /// Refuses what would not work or would be unsafe.
    ///
    /// # Errors
    ///
    /// A message naming the setting.
    pub fn check(&self) -> Result<(), String> {
        if !self.listen.ip().is_loopback() {
            return Err("listen: only loopback; put a proxy that speaks TLS in front".to_owned());
        }
        if !self.unit.contains("{user}") {
            return Err("unit: needs {user}".to_owned());
        }
        if self.pam_service.is_empty() || self.pam_service.contains('/') {
            return Err("pam_service: a service name".to_owned());
        }
        if !self.socket_dir.is_absolute() {
            return Err("socket_dir: an absolute path".to_owned());
        }
        for (name, zero) in [
            ("max_connections", self.max_connections == 0),
            ("max_sign_ins", self.max_sign_ins == 0),
            ("max_sessions", self.max_sessions == 0),
            ("max_relays_per_user", self.max_relays_per_user == 0),
            ("max_requests", self.max_requests == 0),
            ("session_seconds", self.session.is_zero()),
            ("idle_seconds", self.idle.is_zero()),
            ("start_seconds", self.start.is_zero()),
            ("failure_window_seconds", self.failure_window.is_zero()),
            ("request_seconds", self.request.is_zero()),
            ("keep_alive_seconds", self.keep_alive.is_zero()),
        ] {
            if zero {
                return Err(format!("{name}: at least 1"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn what_the_file_says_replaces_the_default_and_the_rest_stays() {
        let settings = Settings::from_toml(
            "idle_seconds = 5\nhosts = [\"Canoa.Example\"]\nlisten = \"127.0.0.1:0\"",
        )
        .unwrap();
        assert_eq!(settings.idle.as_secs(), 5);
        assert_eq!(settings.hosts, ["canoa.example"]);
        assert_eq!(settings.max_failures_per_address, 5);
    }

    #[test]
    fn a_listen_beyond_loopback_an_unknown_key_and_a_zero_limit_are_refused() {
        for text in [
            "listen = \"0.0.0.0:7890\"",
            "pasword = 1",
            "max_sessions = 0",
            "unit = \"canoa.service\"",
            "pam_service = \"../x\"",
            "idle_seconds = 0",
        ] {
            assert!(Settings::from_toml(text).is_err(), "{text}");
        }
    }
}
