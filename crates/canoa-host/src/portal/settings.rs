//! The portal's settings: a TOML file of its own, every one with a default.

use core::net::{IpAddr, SocketAddr};
use core::time::Duration;
use std::path::PathBuf;

use serde::Deserialize;

/// How the portal runs. `Settings::default()` is what runs with no file; each field is the file's key of the same name
/// (a duration as `NAME_seconds`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// `socket`: the Unix socket a proxy reaches the portal at. Only a connection on it is believed when it says, in
    /// `client_address_header`, who the browser is. Its folder must be reachable by this service's account and the proxy's
    /// group alone.
    pub socket: Option<PathBuf>,
    /// `socket_group`: the group that may connect to `socket` (the proxy's), a name or a number; the portal's account must be
    /// in it.
    pub socket_group: Option<String>,
    /// `listen`: a loopback TCP address, for development. Any local user can connect to it or bind it first, so what is
    /// sent there is never believed about the browser's address and passwords should not cross it.
    pub listen: Option<SocketAddr>,
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
    /// `min_uid`: the lowest user id that may sign in. Root never may.
    pub min_uid: u32,
    /// `groups`: the groups an account must be in. Name them: with none, every account on the machine that has a password may
    /// ask for a host.
    pub groups: Vec<String>,
    /// `hosts`: names a request may carry in `Host`, besides `localhost` and addresses; an `Origin` must match `Host`.
    pub hosts: Vec<String>,
    /// `trusted_proxies`: addresses in `client_address_header` that are proxies of ours and so not the browser's.
    pub trusted_proxies: Vec<IpAddr>,
    /// `client_address_header`: the header the proxy puts the browser's address in; on `socket`, the last address in it that
    /// is not a trusted proxy is the browser's.
    pub client_address_header: String,
    /// `session_seconds`: how long a sign-in lasts.
    pub session: Duration,
    /// `idle_seconds`: how long a host nobody uses runs on.
    pub idle: Duration,
    /// `start_seconds`: how long a started host has to answer.
    pub start: Duration,
    /// `pair_seconds`: how long an introduced key is waited for.
    pub pair: Duration,
    /// `max_failures_per_pair`: failed sign-ins one address may make for one account in the window. Keep it, with the others,
    /// below the directory's own lockout threshold: they are the directory's only protection from a guessing browser.
    pub max_failures_per_pair: usize,
    /// `max_failures_per_address`: failed sign-ins one address (an IPv6 one by its /64) may make for any accounts.
    pub max_failures_per_address: usize,
    /// `slow_after_user_failures`: failures for one account, from anywhere, after which each further try waits.
    pub slow_after_user_failures: usize,
    /// `slow_seconds`: how long each failure past that adds to a try's wait, up to thirty times it.
    pub slow: Duration,
    /// `failure_window_seconds`: the window.
    pub failure_window: Duration,
    /// `failure_floor_seconds`: a failed sign-in is answered no sooner than this after its request began, whatever it failed
    /// for, so its time says nothing of whether the account exists.
    pub failure_floor: Duration,
    /// `max_connections`: browsers served at once.
    pub max_connections: usize,
    /// `max_sign_ins`: password checks under way at once.
    pub max_sign_ins: usize,
    /// `max_sessions`: sign-ins kept at once.
    pub max_sessions: usize,
    /// `max_sessions_per_user`: sign-ins one account keeps; the oldest goes when another comes.
    pub max_sessions_per_user: usize,
    /// `max_hosts`: hosts running at once.
    pub max_hosts: usize,
    /// `max_relays_per_user`: channels one user may hold open.
    pub max_relays_per_user: usize,
    /// `max_message_bytes`: the largest message a channel carries either way.
    pub max_message: usize,
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
            socket: Some(PathBuf::from("/run/canoa-portal/portal.sock")),
            socket_group: None,
            listen: None,
            site: None,
            socket_dir: PathBuf::from("/run/canoa"),
            unit: "canoa@{user}.service".to_owned(),
            systemctl: PathBuf::from("systemctl"),
            elevate: Vec::new(),
            pam_service: "canoa".to_owned(),
            min_uid: 1000,
            groups: Vec::new(),
            hosts: Vec::new(),
            trusted_proxies: Vec::new(),
            client_address_header: "x-forwarded-for".to_owned(),
            session: Duration::from_secs(8 * 3600),
            idle: Duration::from_secs(30 * 60),
            start: Duration::from_secs(20),
            pair: Duration::from_secs(8),
            max_failures_per_pair: 3,
            max_failures_per_address: 10,
            slow_after_user_failures: 4,
            slow: Duration::from_secs(2),
            failure_window: Duration::from_secs(15 * 60),
            failure_floor: Duration::from_secs(2),
            max_connections: 256,
            max_sign_ins: 8,
            max_sessions: 1024,
            max_sessions_per_user: 8,
            max_hosts: 64,
            max_relays_per_user: 16,
            max_message: 4 * 1024 * 1024,
            request: Duration::from_secs(10),
            keep_alive: Duration::from_secs(30),
            max_requests: 1000,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    socket: Option<PathBuf>,
    socket_group: Option<String>,
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
    max_failures_per_pair: Option<usize>,
    max_failures_per_address: Option<usize>,
    slow_after_user_failures: Option<usize>,
    slow_seconds: Option<u64>,
    failure_window_seconds: Option<u64>,
    failure_floor_seconds: Option<u64>,
    max_connections: Option<usize>,
    max_sign_ins: Option<usize>,
    max_sessions: Option<usize>,
    max_sessions_per_user: Option<usize>,
    max_hosts: Option<usize>,
    max_relays_per_user: Option<usize>,
    max_message_bytes: Option<usize>,
    request_seconds: Option<u64>,
    keep_alive_seconds: Option<u64>,
    max_requests: Option<usize>,
}

impl Settings {
    /// The settings a file's text says, the rest at their defaults.
    ///
    /// # Errors
    ///
    /// A message naming the key, never its value, for text that is not TOML, a key the portal does not know, a `listen`
    /// beyond loopback, a unit with no `{user}`, or a limit of zero.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let raw: Raw = toml::from_str(text).map_err(|error| error.message().to_owned())?;
        let mut settings = Self::default();
        settings.socket = raw.socket.or(settings.socket);
        settings.socket_group = raw.socket_group.or(settings.socket_group);
        if let Some(listen) = raw.listen {
            settings.listen = Some(
                listen
                    .parse()
                    .map_err(|_| "listen: not an ADDRESS:PORT".to_owned())?,
            );
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
        settings.slow = seconds(raw.slow_seconds, settings.slow);
        settings.failure_window = seconds(raw.failure_window_seconds, settings.failure_window);
        settings.failure_floor = seconds(raw.failure_floor_seconds, settings.failure_floor);
        settings.request = seconds(raw.request_seconds, settings.request);
        settings.keep_alive = seconds(raw.keep_alive_seconds, settings.keep_alive);
        let count = |value: Option<usize>, now: usize| value.unwrap_or(now);
        settings.max_failures_per_pair =
            count(raw.max_failures_per_pair, settings.max_failures_per_pair);
        settings.max_failures_per_address = count(
            raw.max_failures_per_address,
            settings.max_failures_per_address,
        );
        settings.slow_after_user_failures = count(
            raw.slow_after_user_failures,
            settings.slow_after_user_failures,
        );
        settings.max_connections = count(raw.max_connections, settings.max_connections);
        settings.max_sign_ins = count(raw.max_sign_ins, settings.max_sign_ins);
        settings.max_sessions = count(raw.max_sessions, settings.max_sessions);
        settings.max_sessions_per_user =
            count(raw.max_sessions_per_user, settings.max_sessions_per_user);
        settings.max_hosts = count(raw.max_hosts, settings.max_hosts);
        settings.max_relays_per_user = count(raw.max_relays_per_user, settings.max_relays_per_user);
        settings.max_message = count(raw.max_message_bytes, settings.max_message);
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
        if self.socket.is_none() && self.listen.is_none() {
            return Err("socket: the portal listens nowhere".to_owned());
        }
        if self.listen.is_some_and(|listen| !listen.ip().is_loopback()) {
            return Err("listen: only loopback; put a proxy on `socket` in front".to_owned());
        }
        if self
            .socket
            .as_ref()
            .is_some_and(|socket| !socket.is_absolute())
        {
            return Err("socket: an absolute path".to_owned());
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
        if self.min_uid == 0 {
            return Err("min_uid: at least 1; root never signs in".to_owned());
        }
        for (name, zero) in [
            ("max_connections", self.max_connections == 0),
            ("max_sign_ins", self.max_sign_ins == 0),
            ("max_sessions", self.max_sessions == 0),
            ("max_sessions_per_user", self.max_sessions_per_user == 0),
            ("max_hosts", self.max_hosts == 0),
            ("max_relays_per_user", self.max_relays_per_user == 0),
            ("max_message_bytes", self.max_message == 0),
            ("max_requests", self.max_requests == 0),
            ("max_failures_per_pair", self.max_failures_per_pair == 0),
            (
                "max_failures_per_address",
                self.max_failures_per_address == 0,
            ),
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
            "idle_seconds = 5\nhosts = [\"Canoa.Example\"]\nlisten = \"127.0.0.1:0\"\nmax_hosts = 3",
        )
        .unwrap();
        assert_eq!(settings.idle.as_secs(), 5);
        assert_eq!(settings.hosts, ["canoa.example"]);
        assert_eq!(settings.max_hosts, 3);
        assert_eq!(settings.max_failures_per_pair, 3);
        assert!(settings.socket.is_some());
    }

    #[test]
    fn a_listen_beyond_loopback_an_unknown_key_and_a_zero_limit_are_refused() {
        for text in [
            "listen = \"0.0.0.0:7890\"",
            "pasword = 1",
            "max_sessions = 0",
            "max_hosts = 0",
            "max_failures_per_pair = 0",
            "unit = \"canoa.service\"",
            "pam_service = \"../x\"",
            "idle_seconds = 0",
            "min_uid = 0",
            "socket = \"relative.sock\"",
        ] {
            assert!(Settings::from_toml(text).is_err(), "{text}");
        }
    }
}
