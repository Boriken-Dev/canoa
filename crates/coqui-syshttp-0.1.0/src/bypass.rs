//! Which URLs go direct: bypass lists in each operating system's own syntax.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::IpAddr;
use core::str::FromStr;

use coqui_net_acl::IpNet;

use crate::netutil::{split_host_port, wildcard_match};
use crate::proxy::Target;

/// The syntax a bypass list is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dialect {
    /// `NO_PROXY` (curl): entries separated by commas or spaces; a bare name covers its subdomains.
    NoProxy,
    /// Windows `ProxyOverride` (WinINet): entries separated by `;`; `<local>` is a dotless host.
    WinInet,
    /// macOS `ExceptionsList`.
    MacOs,
    /// GNOME and MATE `ignore-hosts`.
    GSettings,
    /// KDE `NoProxyFor`; a bare name covers its subdomains.
    Kde,
    /// KDE `NoProxyFor` with `ReversedException=true`: the list names the hosts that use the proxy.
    KdeReversed,
}

impl Dialect {
    fn separates(self, character: char) -> bool {
        character.is_whitespace() || character == if self == Self::WinInet { ';' } else { ',' }
    }

    fn covers_subdomains(self) -> bool {
        matches!(self, Self::NoProxy | Self::Kde | Self::KdeReversed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Rule {
    Everything,
    Local,
    Subnet,
    Exact(String, Option<u16>),
    Suffix(String, Option<u16>),
    Glob(String, Option<u16>),
    Network(IpNet, Option<u16>),
    Address(IpAddr, Option<u16>),
}

impl Rule {
    fn matches(&self, target: &Target) -> bool {
        let port_ok = |wanted: &Option<u16>| wanted.is_none_or(|port| port == target.port);
        let host = target.host.as_str();
        match self {
            Self::Everything => true,
            Self::Local => !host.contains('.') && target.address().is_none(),
            Self::Subnet => on_local_network(target),
            Self::Exact(name, port) => host == name && port_ok(port),
            Self::Suffix(domain, port) => {
                (host == domain
                    || host
                        .strip_suffix(domain.as_str())
                        .is_some_and(|front| front.ends_with('.')))
                    && port_ok(port)
            }
            Self::Glob(pattern, port) => wildcard_match(pattern, host, true) && port_ok(port),
            Self::Network(network, port) => {
                target.address().is_some_and(|a| network.contains(a)) && port_ok(port)
            }
            Self::Address(address, port) => target.address() == Some(*address) && port_ok(port),
        }
    }
}

/// A list of hosts, domains, networks and ports that bypass the proxy.
///
/// Entries are:
///
/// - a host name: exact, or, in [`Dialect::NoProxy`] and [`Dialect::Kde`] and for any entry
///   written with a leading `.` or `*.`, the name and every subdomain;
/// - a glob with `*` and `?`, matched without regard to case;
/// - an IP network in CIDR form (`10.0.0.0/8`, abbreviated `169.254/16`, `2001:db8::/32`),
///   matched against IP-literal hosts only: no name is resolved;
/// - an optional `:port` after a host, compared with the URL's port (its scheme's default
///   when it has none);
/// - `<local>`: a name with no dot;
/// - `<subnet>`: a host that resolves to a link-local address or to one on a network a local
///   interface is on (the name is resolved by the system's resolver, so a lookup waits for DNS);
/// - `<-loopback>`: turns the implicit loopback bypass off; `*` as the whole list: every host.
///
/// `localhost`, `*.localhost`, `127.0.0.0/8` and `::1` always bypass unless `<-loopback>` is
/// listed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BypassRules {
    rules: Vec<Rule>,
    entries: Vec<String>,
    skipped: Vec<String>,
    loopback: bool,
    reversed: bool,
}

impl Default for BypassRules {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            entries: Vec::new(),
            skipped: Vec::new(),
            loopback: true,
            reversed: false,
        }
    }
}

impl BypassRules {
    /// No entries: only loopback bypasses.
    pub fn none() -> Self {
        Self::default()
    }

    /// Reads a list written in `dialect`, as one string. An entry that cannot be read is
    /// skipped and listed in [`BypassRules::skipped`].
    pub fn parse(text: &str, dialect: Dialect) -> Self {
        let entries = text
            .split(|character| dialect.separates(character))
            .filter(|entry| !entry.is_empty());
        Self::from_entries(entries, dialect)
    }

    /// Reads a list from its entries.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a str>, dialect: Dialect) -> Self {
        let entries: Vec<&str> = entries
            .into_iter()
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        let whole_star = entries == ["*"];
        let mut rules = Self {
            reversed: dialect == Dialect::KdeReversed,
            ..Self::default()
        };
        for entry in entries {
            let token = entry.to_ascii_lowercase();
            if token == "<-loopback>" {
                rules.loopback = false;
                rules.entries.push(entry.to_string());
                continue;
            }
            if token == "*" && !whole_star && dialect == Dialect::NoProxy {
                // curl and requests ignore a `*` among other entries.
                rules.skipped.push(entry.to_string());
                continue;
            }
            match parse_entry(entry, dialect) {
                Some(rule) => {
                    rules.rules.push(rule);
                    rules.entries.push(entry.to_string());
                }
                None => rules.skipped.push(entry.to_string()),
            }
        }
        rules
    }

    /// The entries that were read.
    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }

    /// The entries that could not be read.
    pub fn skipped(&self) -> impl Iterator<Item = &str> {
        self.skipped.iter().map(String::as_str)
    }

    /// Whether the list has no entry that can match (loopback still bypasses).
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether `url` goes direct. A URL that cannot be read never matches.
    pub fn matches(&self, url: &str) -> bool {
        Target::parse(url).is_ok_and(|target| self.matches_target(&target))
    }

    /// Whether the list names the hosts that use the proxy rather than the ones that bypass it.
    pub(crate) fn is_reversed(&self) -> bool {
        self.reversed
    }

    /// The list as `NO_PROXY` entries, with what that syntax cannot say set apart.
    ///
    /// A bare name in `NO_PROXY` covers its subdomains, so an exact host of another dialect is
    /// written the same way and widens. Loopback is listed when the list bypasses it, since
    /// most programs do not bypass it themselves. A reversed list has no `NO_PROXY` form.
    pub(crate) fn to_no_proxy(&self) -> NoProxyList {
        let mut list = NoProxyList::default();
        if self.reversed {
            list.unexpressed.extend(self.entries.iter().cloned());
            list.unexpressed.extend(self.skipped.iter().cloned());
            return list;
        }
        if self.loopback {
            list.entries
                .extend(["localhost", "127.0.0.1", "::1"].map(String::from));
        }
        let listed = self
            .entries
            .iter()
            .filter(|entry| !entry.eq_ignore_ascii_case("<-loopback>"));
        for (rule, text) in self.rules.iter().zip(listed) {
            let entry = match rule {
                Rule::Everything => Some("*".to_string()),
                Rule::Local | Rule::Subnet | Rule::Glob(..) => None,
                Rule::Exact(host, port) | Rule::Suffix(host, port) => Some(with_port(host, *port)),
                Rule::Network(network, None) => Some(network.to_string()),
                Rule::Network(_, Some(_)) => None,
                Rule::Address(address, port) => Some(with_port(&address.to_string(), *port)),
            };
            match entry {
                Some(entry) => list.entries.push(entry),
                None => list.unexpressed.push(text.clone()),
            }
        }
        list.unexpressed.extend(self.skipped.iter().cloned());
        list
    }

    pub(crate) fn matches_target(&self, target: &Target) -> bool {
        if self.loopback && is_loopback(target) {
            return true;
        }
        let listed = self.rules.iter().any(|rule| rule.matches(target));
        listed != self.reversed
    }
}

/// A bypass list in `NO_PROXY` form.
#[derive(Debug, Default)]
pub(crate) struct NoProxyList {
    pub entries: Vec<String>,
    pub unexpressed: Vec<String>,
}

fn with_port(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => crate::proxy::join_host_port(host, port),
        None => host.to_string(),
    }
}

/// Whether the host resolves to a link-local address or to one on a network of a local interface.
fn on_local_network(target: &Target) -> bool {
    use std::net::ToSocketAddrs as _;
    let addresses: Vec<IpAddr> = match target.address() {
        Some(address) => alloc::vec![address],
        None => (target.host.as_str(), 0)
            .to_socket_addrs()
            .map(|found| found.map(|socket| socket.ip()).collect())
            .unwrap_or_default(),
    };
    if addresses.iter().any(link_local) {
        return true;
    }
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return false;
    };
    interfaces.iter().any(|interface| {
        let (own, prefix) = match &interface.addr {
            if_addrs::IfAddr::V4(v4) => (IpAddr::V4(v4.ip), v4.prefixlen),
            if_addrs::IfAddr::V6(v6) => (IpAddr::V6(v6.ip), v6.prefixlen),
        };
        addresses
            .iter()
            .any(|address| same_network(*address, own, prefix))
    })
}

fn link_local(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_unicast_link_local(),
    }
}

/// Whether `a` and `b` share their first `prefix` bits (and are the same family).
fn same_network(a: IpAddr, b: IpAddr, prefix: u8) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = u32::MAX
                .checked_shl(32 - u32::from(prefix.min(32)))
                .unwrap_or(0);
            u32::from(a) & mask == u32::from(b) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let mask = u128::MAX
                .checked_shl(128 - u32::from(prefix.min(128)))
                .unwrap_or(0);
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

fn is_loopback(target: &Target) -> bool {
    let host = target.host.as_str();
    host == "localhost"
        || host.ends_with(".localhost")
        || target.address().is_some_and(|address| match address {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        })
}

fn parse_entry(entry: &str, dialect: Dialect) -> Option<Rule> {
    let token = entry.to_ascii_lowercase();
    match token.as_str() {
        "*" => return Some(Rule::Everything),
        "<local>" => return Some(Rule::Local),
        "<subnet>" => return Some(Rule::Subnet),
        _ => {}
    }
    // WinINet and KDE lists sometimes carry `http://host`.
    let entry = entry
        .split_once("://")
        .map_or(entry, |(_, rest)| rest.trim_end_matches('/'));
    if entry.contains('/') {
        return parse_network(entry).map(|network| Rule::Network(network, None));
    }
    let (host, port) = split_entry(entry)?;
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    if let Some(domain) = host.strip_prefix("*.")
        && !domain.contains(['*', '?'])
    {
        return Some(Rule::Suffix(domain.to_string(), port));
    }
    if host.contains(['*', '?']) {
        return Some(Rule::Glob(host, port));
    }
    if let Some(domain) = host.strip_prefix('.') {
        return Some(Rule::Suffix(
            domain.trim_start_matches('.').to_string(),
            port,
        ));
    }
    if let Ok(address) = host.split('%').next().unwrap_or(&host).parse::<IpAddr>() {
        return Some(Rule::Address(address, port));
    }
    Some(if dialect.covers_subdomains() {
        Rule::Suffix(host, port)
    } else {
        Rule::Exact(host, port)
    })
}

/// `host[:port]`; a host holding glob characters splits on its last colon only when a port follows.
fn split_entry(entry: &str) -> Option<(String, Option<u16>)> {
    if entry.contains(['*', '?']) {
        return Some(match entry.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                (host.to_string(), port.parse().ok())
            }
            _ => (entry.to_string(), None),
        });
    }
    split_host_port(entry)
}

/// A CIDR network, an abbreviated IPv4 one (`169.254/16`) included.
fn parse_network(text: &str) -> Option<IpNet> {
    let (address, prefix) = text.split_once('/')?;
    let octets: Vec<&str> = address.split('.').collect();
    let padded;
    let address = if octets.len() < 4
        && octets
            .iter()
            .all(|o| !o.is_empty() && o.bytes().all(|b| b.is_ascii_digit()))
    {
        padded = octets
            .iter()
            .copied()
            .chain(core::iter::repeat_n("0", 4 - octets.len()))
            .collect::<Vec<_>>()
            .join(".");
        padded.as_str()
    } else {
        address
    };
    IpNet::from_str(&alloc::format!("{address}/{prefix}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_proxy(text: &str) -> BypassRules {
        BypassRules::parse(text, Dialect::NoProxy)
    }

    #[test]
    fn a_bare_name_covers_its_subdomains_in_no_proxy_but_not_in_wininet() {
        let list = no_proxy("example.com");
        assert!(list.matches("http://example.com/"));
        assert!(list.matches("http://a.example.com/"));
        assert!(!list.matches("http://badexample.com/"));
        let list = BypassRules::parse("example.com", Dialect::WinInet);
        assert!(list.matches("http://example.com/"));
        assert!(!list.matches("http://a.example.com/"));
        let list = BypassRules::parse("*.example.com", Dialect::WinInet);
        assert!(list.matches("http://a.example.com/"));
        assert!(list.matches("http://example.com/"));
    }

    #[test]
    fn a_cidr_entry_matches_ip_literals_only_and_an_abbreviated_one_is_padded() {
        let list = no_proxy("10.0.0.0/8, 169.254/16, 2001:db8::/32");
        assert!(list.matches("http://10.1.2.3/"));
        assert!(list.matches("http://169.254.9.9/"));
        assert!(list.matches("http://[2001:db8::1]/"));
        assert!(!list.matches("http://11.0.0.1/"));
        assert!(!list.matches("http://ten.example/"));
    }

    #[test]
    fn a_port_in_an_entry_is_compared_with_the_urls_port_or_its_default() {
        let list = no_proxy("host:8443");
        assert!(list.matches("https://host:8443/"));
        assert!(!list.matches("https://host/"));
        let list = no_proxy("host:443");
        assert!(list.matches("https://host/"));
    }

    #[test]
    fn local_matches_a_name_without_a_dot() {
        let list = BypassRules::parse("<local>", Dialect::WinInet);
        assert!(list.matches("http://intranet/"));
        assert!(!list.matches("http://intranet.corp/"));
        assert!(!list.matches("http://10.1.1.1/"));
    }

    #[test]
    fn loopback_bypasses_unless_the_list_turns_it_off() {
        assert!(BypassRules::none().matches("http://localhost:3000/"));
        assert!(BypassRules::none().matches("http://app.localhost/"));
        assert!(BypassRules::none().matches("http://127.9.9.9/"));
        assert!(BypassRules::none().matches("http://[::1]/"));
        assert!(!no_proxy("<-loopback>").matches("http://127.0.0.1/"));
    }

    #[test]
    fn a_star_alone_bypasses_every_host_and_among_others_it_is_ignored_by_no_proxy() {
        assert!(no_proxy("*").matches("http://anywhere.example/"));
        let list = no_proxy("a.example, *");
        assert!(!list.matches("http://b.example/"));
        assert_eq!(list.skipped().collect::<Vec<_>>(), ["*"]);
        let list = BypassRules::parse("a.example;*", Dialect::WinInet);
        assert!(list.matches("http://b.example/"));
    }

    #[test]
    fn a_reversed_list_names_the_hosts_that_use_the_proxy() {
        let list = BypassRules::parse("corp.example", Dialect::KdeReversed);
        assert!(!list.matches("http://corp.example/"));
        assert!(list.matches("http://other.example/"));
        assert!(list.matches("http://localhost/"));
    }

    #[test]
    fn a_glob_is_case_blind_and_an_unreadable_entry_is_reported() {
        let list = no_proxy("*.CORP.example, [bad");
        assert!(list.matches("http://a.corp.example/"));
        assert_eq!(list.skipped().collect::<Vec<_>>(), ["[bad"]);
    }

    #[test]
    fn subnet_matches_an_address_on_a_local_network_or_link_local() {
        let list = BypassRules::parse("<subnet>", Dialect::WinInet);
        assert_eq!(list.skipped().count(), 0);
        assert!(list.matches("http://169.254.7.7/"));
        assert!(list.matches("http://[fe80::1]/"));
        assert!(!list.matches("http://192.0.2.1/"));
        // Whatever network this machine is on, its own address is on it.
        let own = if_addrs::get_if_addrs()
            .unwrap()
            .into_iter()
            .find(|interface| !interface.is_loopback() && interface.ip().is_ipv4())
            .map(|interface| interface.ip());
        if let Some(own) = own {
            assert!(list.matches(&alloc::format!("http://{own}/")));
        }
    }

    #[test]
    fn a_prefix_decides_which_addresses_share_a_network() {
        let a: IpAddr = "10.1.2.3".parse().unwrap();
        assert!(same_network(a, "10.1.200.9".parse().unwrap(), 16));
        assert!(!same_network(a, "10.2.2.3".parse().unwrap(), 16));
        assert!(same_network(a, "99.9.9.9".parse().unwrap(), 0));
        assert!(!same_network(a, "10.1.2.4".parse().unwrap(), 32));
        assert!(!same_network(a, "::1".parse().unwrap(), 8));
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(same_network(v6, "2001:db8::ffff".parse().unwrap(), 64));
    }

    #[test]
    fn a_scheme_in_a_wininet_entry_is_ignored() {
        let list = BypassRules::parse("http://host.example/", Dialect::WinInet);
        assert!(list.matches("https://host.example/"));
    }
}
