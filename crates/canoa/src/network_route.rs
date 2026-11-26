//! The `network_route` kind: how traffic leaves this computer.
//!
//! A route says how a connection is made: by the system's choice, straight to its destination, through a proxy the person types
//! (per scheme, as `HTTP_PROXY` and `HTTPS_PROXY` are), through a PAC script, through a program of the person's, or any way an
//! extension adds. The host names one route as its own, an account may name another for its own calls, and a profile names the
//! routes its sessions' programs leave by. A route is one configuration of one provider: its settings are the schema the library
//! declares (`config = ..., form = ...` on the provider), and a setting that names a host variable (a proxy password) is listed in
//! [`RouteInfo::secrets`], so a page asks for the name and never shows a value.
//!
//! A provider answers what it would do ([`NetworkRoute::check`]) and, when asked to [`open`](NetworkRoute::open), hands the host a
//! [`RouteLease`]: the [`Reach`] the host builds its own HTTP client from, and, for the programs it starts, extra variables and
//! the authorities they must trust. Sharing is the host's: it opens one lease for every user whose route and settings are equal
//! and ends it after the last ([`Sharing`] says whether that is allowed), so a provider carries no pooling code.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, Secret, ToValue};

use crate::network::{
    ClientCertificate, GitTls, Network, NetworkError, NetworkSettings, ProxyChoice,
    Secret as ProxySecret,
};

/// The name of the `network_route` kind.
pub const KIND: &str = "network_route";

/// Whether the host may give one lease to several users.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Sharing {
    /// One lease serves every user whose route and settings are equal, and ends after the last one.
    #[default]
    Shared,
    /// Each session has a lease of its own, which ends with the session.
    PerSession,
}

/// What a route is, from [`NetworkRoute::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct RouteInfo {
    /// Its name for people.
    pub title: String,
    /// One line on what it does.
    pub description: Option<String>,
    /// Whether the host may share a lease between users.
    #[map(default)]
    pub sharing: Sharing,
    /// The settings whose value is the name of a host variable (a password's), by key. The host reads those variables and hands the
    /// values over in [`OpenRequest::variables`]; a page asks for the name.
    #[map(default)]
    pub secrets: Vec<String>,
}

impl RouteInfo {
    /// A shared route called `title`, with no secret.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            sharing: Sharing::Shared,
            secrets: Vec::new(),
        }
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        self.title
            .trim()
            .is_empty()
            .then(|| String::from("a network route has no title"))
    }
}

/// The value of a host variable a route's settings name.
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct HostVariable {
    /// The variable's name.
    pub name: String,
    /// Its value.
    pub value: Secret<String>,
}

impl HostVariable {
    /// The variable `name` holding `value`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: Secret::new(value.into()),
        }
    }
}

/// What the host asks of a route, for [`NetworkRoute::check`] and [`NetworkRoute::open`].
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct OpenRequest {
    /// Who the lease is for: `host`, `account:NAME` or `session:ID`.
    pub scope: String,
    /// The addresses the user will reach, for a route that chooses per destination (a PAC script) or must know where it is going.
    #[map(default)]
    pub reaches: Vec<String>,
    /// The values of the variables [`RouteInfo::secrets`] names that are set. A variable that is not set is absent, which
    /// [`NetworkRoute::check`] reports.
    #[map(default)]
    pub variables: Vec<HostVariable>,
    /// A folder private to the lease, given only when [`RouteLease::trust`] asks the host to write files.
    pub private_dir: Option<String>,
    /// The folder a relative path in the settings is made from (the folder of canoa.cfg), when the configuration has one.
    pub base_dir: Option<String>,
}

impl OpenRequest {
    /// A request for the lease of `scope`, with nothing else.
    #[must_use]
    pub fn new(scope: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            reaches: Vec::new(),
            variables: Vec::new(),
            private_dir: None,
            base_dir: None,
        }
    }

    /// The value of the host variable `name`, when it was handed over.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&str> {
        self.variables
            .iter()
            .find(|each| each.name == name)
            .map(|each| each.value.expose().as_str())
    }
}

/// What judging a route's settings found, from [`NetworkRoute::check`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Checked {
    /// What is wrong, each in words that name the setting and never a value; empty when the route can be opened.
    #[map(default)]
    pub problems: Vec<String>,
}

impl Checked {
    /// A route with nothing wrong.
    #[must_use]
    pub fn ok() -> Self {
        Self::default()
    }

    /// A route with these problems.
    #[must_use]
    pub fn of(problems: Vec<String>) -> Self {
        Self { problems }
    }
}

/// How a connection leaves, in the terms the host's network is made from.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Via {
    /// What the system says: the proxy variables of the environment, else the operating system's settings, PAC and WPAD included.
    #[default]
    System,
    /// Straight to the destination.
    Direct,
    /// Through the proxy of `http_proxy`, `https_proxy` or `all_proxy` that fits the destination's scheme (an HTTP proxy, or
    /// `socks5://`; it must carry CONNECT, since TLS and WebSocket tunnels use it).
    Proxy,
    /// A PAC script chooses: `pac` is a URL, a `file:` URL, a path or the script's text.
    Pac,
}

/// How the host reaches a destination by a lease. What is not set keeps the system's own behaviour.
#[derive(ToValue, FromValue, Schema, Debug, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Reach {
    /// Which way.
    #[map(default)]
    pub via: Via,
    /// The proxy of `http` addresses (`[scheme://][user[:password]@]host[:port]`), for [`Via::Proxy`].
    pub http_proxy: Option<String>,
    /// The proxy of `https` addresses, for [`Via::Proxy`].
    pub https_proxy: Option<String>,
    /// The proxy of every other address, for [`Via::Proxy`].
    pub all_proxy: Option<String>,
    /// The PAC script, for [`Via::Pac`].
    pub pac: Option<String>,
    /// A login for every HTTP proxy that names none of its own.
    pub user: Option<String>,
    /// Its password.
    pub password: Option<Secret<String>>,
    /// The hosts that go direct, in `NO_PROXY` form, replacing the system's list; absent keeps it.
    pub no_proxy: Option<Vec<String>>,
    /// Whether a proxy that asks for Negotiate or NTLM is answered with the logon session; absent is the host's default.
    pub negotiate: Option<bool>,
    /// Which client certificate is sent: `system`, `none` or the path of a PEM file holding a chain and its key.
    pub client_certificate: Option<String>,
    /// Trust exactly the certificates of this PEM file, replacing the system's trust.
    pub ca_file: Option<String>,
    /// Whose TLS library git uses: `system` or `git`.
    pub git_tls: Option<String>,
}

impl Reach {
    /// The system's way.
    #[must_use]
    pub fn system() -> Self {
        Self::default()
    }

    /// Straight to the destination.
    #[must_use]
    pub fn direct() -> Self {
        Self {
            via: Via::Direct,
            ..Self::default()
        }
    }

    /// Through `proxy` for every address.
    #[must_use]
    pub fn proxy(proxy: impl Into<String>) -> Self {
        Self {
            via: Via::Proxy,
            all_proxy: Some(proxy.into()),
            ..Self::default()
        }
    }
}

impl Reach {
    /// The settings a [`Network`] is made from.
    ///
    /// # Errors
    ///
    /// A way that lacks what it needs (a proxy, a script), a `git_tls` that is neither `system` nor `git`, or an empty
    /// `client_certificate` or `ca_file`.
    pub fn settings(&self) -> Result<NetworkSettings, NetworkError> {
        let refused = |what: &str| NetworkError::new(alloc::format!("route: {what}"));
        let mut settings = NetworkSettings::default();
        settings.proxy = match self.via {
            Via::System => ProxyChoice::System,
            Via::Direct => ProxyChoice::None,
            Via::Proxy => ProxyChoice::Each {
                http: self.http_proxy.clone(),
                https: self.https_proxy.clone(),
                all: self.all_proxy.clone(),
            },
            Via::Pac => ProxyChoice::Pac(
                self.pac
                    .clone()
                    .filter(|script| !script.trim().is_empty())
                    .ok_or_else(|| refused("a PAC script names no location"))?,
            ),
        };
        settings.proxy_user.clone_from(&self.user);
        settings.proxy_password = self
            .password
            .as_ref()
            .map(|password| ProxySecret::new(password.expose().clone()));
        settings.no_proxy.clone_from(&self.no_proxy);
        settings.negotiate = self.negotiate;
        settings.client_certificate = match self.client_certificate.as_deref().map(str::trim) {
            None => ClientCertificate::System,
            Some("") => return Err(refused("client_certificate is empty")),
            Some(text) if text.eq_ignore_ascii_case("system") => ClientCertificate::System,
            Some(text) if text.eq_ignore_ascii_case("none") => ClientCertificate::None,
            Some(path) => ClientCertificate::File(path.into()),
        };
        settings.ca_file = match self.ca_file.as_deref() {
            None => None,
            Some("") => return Err(refused("ca_file is empty")),
            Some(path) => Some(path.into()),
        };
        settings.git_tls = match self.git_tls.as_deref() {
            None | Some("system") => GitTls::System,
            Some("git") => GitTls::Git,
            Some(_) => return Err(refused("git_tls is system or git")),
        };
        Ok(settings)
    }
}

impl Network {
    /// The network a lease's [`Reach`] says: its HTTP clients, tunnels and the variables of the programs it serves.
    ///
    /// # Errors
    ///
    /// As [`Reach::settings`], and as [`Network::new`].
    pub fn from_reach(reach: &Reach) -> Result<Self, NetworkError> {
        Self::new(&reach.settings()?)
    }
}

/// A variable a lease gives the programs on it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct LeaseVariable {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
    /// Whether the value is secret: a report names the variable and never shows it.
    #[map(default)]
    pub secret: bool,
}

impl LeaseVariable {
    /// A variable `name` set to `value`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>, secret: bool) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            secret,
        }
    }
}

/// The addresses the programs on a lease will reach, for [`RouteLease::variables`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Urls {
    /// The addresses.
    #[map(default)]
    pub urls: Vec<String>,
}

impl Urls {
    /// These addresses.
    #[must_use]
    pub fn of(urls: Vec<String>) -> Self {
        Self { urls }
    }
}

/// Authorities the programs on a lease must trust: a route that ends TLS itself.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Trust {
    /// The certificates, as PEM. The host writes them into the session's private folder and names them by the `SSL_CERT_FILE`
    /// family of variables.
    pub roots_pem: String,
}

impl Trust {
    /// These certificates.
    #[must_use]
    pub fn new(roots_pem: impl Into<String>) -> Self {
        Self {
            roots_pem: roots_pem.into(),
        }
    }
}

/// What a lease says of itself for a page or a report. It carries no secret.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct LeaseStatus {
    /// What it does now, in a line.
    pub description: String,
    /// The address it listens on, when it runs a proxy of its own.
    pub address: Option<String>,
    /// More, for a person who asks.
    pub detail: Option<String>,
}

impl LeaseStatus {
    /// A lease described by `description`.
    #[must_use]
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            address: None,
            detail: None,
        }
    }
}

/// One way for traffic to leave, as configured. An instance is one configuration of one provider; the provider's settings are its
/// schema.
///
/// A provider that runs a program or a listener starts it in [`open`](Self::open) and ends it when the lease is destroyed. A
/// refusal says what the person can act on (a variable that is not set, a file that cannot be read, a program that did not start)
/// in words that hold no password and no query.
#[coqui_acp_api::guatiao::kind(name = "network_route", crate = ::coqui_acp_api::guatiao)]
pub trait NetworkRoute: Send + Sync {
    /// What it is. Cheap, and the same on every call.
    fn describe(&self) -> Result<RouteInfo, ProviderError>;

    /// Judges the settings with nothing started: a variable that is not set (`request.variables` lacks it), a file that is missing,
    /// an address that does not parse.
    fn check(&self, request: OpenRequest) -> Result<Checked, ProviderError>;

    /// A lease for `request.scope`, started when the host holds none that serves it.
    fn open(&self, request: OpenRequest) -> Result<Object<dyn RouteLease>, ProviderError>;
}

/// A route in use. Destroying it ends it.
#[coqui_acp_api::guatiao::kind(object, name = "route_lease", crate = ::coqui_acp_api::guatiao)]
pub trait RouteLease: Send + Sync {
    /// How a connection of the host's own leaves.
    fn reach(&self) -> Result<Reach, ProviderError>;

    /// Environment for a program that reaches `request.urls` on this lease, beyond what [`reach`](Self::reach) implies.
    fn variables(&self, request: Urls) -> Result<Vec<LeaseVariable>, ProviderError>;

    /// The authorities a program on this lease must trust, for a route that ends TLS.
    fn trust(&self) -> Result<Option<Trust>, ProviderError>;

    /// What the lease is doing, for a page.
    fn status(&self) -> Result<LeaseStatus, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_needs_a_title() {
        assert_eq!(RouteInfo::new("Manual").problem(), None);
        assert!(RouteInfo::new("  ").problem().is_some());
    }

    #[test]
    fn a_request_hands_over_the_variables_it_was_given() {
        let mut request = OpenRequest::new("host");
        request.variables.push(HostVariable::new("PW", "hunter2"));
        assert_eq!(request.variable("PW"), Some("hunter2"));
        assert_eq!(request.variable("OTHER"), None);
    }

    #[test]
    fn a_secret_is_never_printed() {
        let mut request = OpenRequest::new("host");
        request.variables.push(HostVariable::new("PW", "hunter2"));
        assert!(!alloc::format!("{request:?}").contains("hunter2"));
        let mut reach = Reach::proxy("proxy.example.test:8080");
        reach.password = Some(Secret::new(String::from("hunter2")));
        assert!(!alloc::format!("{reach:?}").contains("hunter2"));
    }

    #[test]
    fn a_reach_starts_with_the_system() {
        assert_eq!(Reach::system().via, Via::System);
        assert_eq!(Reach::direct().via, Via::Direct);
        assert_eq!(Reach::proxy("p:1").all_proxy.as_deref(), Some("p:1"));
    }
}
