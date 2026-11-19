//! The proxy environment variables: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` and
//! their kin, read from a snapshot of the environment.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::bypass::{BypassRules, Dialect};
use crate::proxy::{Proxy, Route, Target};

/// A snapshot of environment variables.
///
/// [`Environment::process`] takes the process's; [`Environment::from_pairs`] builds one by
/// hand, so a test or a host that keeps its own settings never touches the process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    variables: BTreeMap<String, String>,
    case_insensitive: bool,
}

impl Environment {
    /// The process's environment, as it is now.
    pub fn process() -> Self {
        let variables = std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        Self {
            variables,
            case_insensitive: cfg!(windows),
        }
    }

    /// An environment of exactly `pairs`, with case-sensitive names.
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            variables: pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
            case_insensitive: false,
        }
    }

    /// The variable `name`, exactly as written.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.variables.get(name).map(String::as_str)
    }

    fn contains(&self, name: &str) -> bool {
        self.variables.contains_key(name)
    }

    /// The proxy variables in this environment; `None` when none of them is set.
    ///
    /// Any non-empty `<scheme>_proxy` or `all_proxy` counts. A lowercase name wins over its
    /// uppercase form, as in curl and the Go standard library. Inside a CGI request
    /// (`REQUEST_METHOD` set) an uppercase `HTTP_PROXY` is ignored, because a client's `Proxy:`
    /// header can set it; where names are case-insensitive `http_proxy` is then ignored too.
    pub fn proxy_variables(&self) -> Option<ProxyVariables> {
        let cgi = self.contains("REQUEST_METHOD");
        let mut proxies: BTreeMap<String, String> = BTreeMap::new();
        let mut no_proxy = None;
        // Uppercase names first, so the lowercase pass overwrites them.
        for lowercase_pass in [false, true] {
            for (name, value) in &self.variables {
                if (*name == name.to_ascii_lowercase()) != lowercase_pass {
                    continue;
                }
                let key = name.to_ascii_lowercase();
                let Some(scheme) = key.strip_suffix("_proxy") else {
                    continue;
                };
                if scheme == "no" {
                    if !value.trim().is_empty() {
                        no_proxy = Some(value.clone());
                    }
                    continue;
                }
                if cgi && scheme == "http" && (!lowercase_pass || self.case_insensitive) {
                    continue;
                }
                if !value.trim().is_empty() {
                    proxies.insert(scheme.to_string(), value.trim().to_string());
                }
            }
        }
        if proxies.is_empty() && no_proxy.is_none() {
            return None;
        }
        Some(ProxyVariables { proxies, no_proxy })
    }
}

/// What an environment's proxy variables say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyVariables {
    proxies: BTreeMap<String, String>,
    no_proxy: Option<String>,
}

/// What a proxy variable names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A proxy URL, or a PAC result such as `PROXY a:3128; DIRECT`.
    Routes(Vec<Route>),
    /// A PAC file's location: a `file:` URL or an `http(s)` URL with a path.
    PacLocation(String),
    /// The text of a PAC script.
    PacScript(String),
}

impl Source {
    /// Reads a setting that is a proxy URL, a PAC result, a PAC location or a script.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.contains("FindProxyForURL") {
            return Some(Self::PacScript(value.to_string()));
        }
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("file:") {
            return Some(Self::PacLocation(value.to_string()));
        }
        if let Some(rest) = lower
            .strip_prefix("http://")
            .or_else(|| lower.strip_prefix("https://"))
            && rest
                .split_once('/')
                .is_some_and(|(_, path)| !path.is_empty())
        {
            return Some(Self::PacLocation(value.to_string()));
        }
        if value.contains("://") {
            return Proxy::parse(value)
                .ok()
                .map(|proxy| Self::Routes(alloc::vec![Route::Proxy(proxy)]));
        }
        if value.contains([';', ' ', '\t']) || lower == "direct" {
            return Some(Self::Routes(Route::parse_pac_result(value)));
        }
        Proxy::parse(value)
            .ok()
            .map(|proxy| Self::Routes(alloc::vec![Route::Proxy(proxy)]))
    }
}

impl ProxyVariables {
    /// `NO_PROXY` as a bypass list.
    pub fn no_proxy(&self) -> BypassRules {
        self.no_proxy
            .as_deref()
            .map(|text| BypassRules::parse(text, Dialect::NoProxy))
            .unwrap_or_default()
    }

    /// Whether any `<scheme>_proxy` is set (`NO_PROXY` alone does not make the variables decide).
    pub fn has_proxies(&self) -> bool {
        !self.proxies.is_empty()
    }

    /// The setting for `target`'s scheme (`ws` falls back to `http`, `wss` to `https`), else `all`.
    pub(crate) fn source_for(&self, target: &Target) -> Option<Source> {
        let fallback = match target.scheme.as_str() {
            "ws" => Some("http"),
            "wss" => Some("https"),
            _ => None,
        };
        [Some(target.scheme.as_str()), fallback, Some("all")]
            .into_iter()
            .flatten()
            .find_map(|key| self.proxies.get(key))
            .and_then(|value| Source::parse(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(pairs: &[(&str, &str)]) -> Option<ProxyVariables> {
        Environment::from_pairs(pairs.iter().copied()).proxy_variables()
    }

    #[test]
    fn nothing_is_read_from_an_environment_without_proxy_variables() {
        assert!(variables(&[("PATH", "/bin"), ("HTTP_PROXYX", "x")]).is_none());
        assert!(variables(&[("HTTP_PROXY", "  ")]).is_none());
    }

    #[test]
    fn a_lowercase_variable_wins_over_its_uppercase_form() {
        let vars = variables(&[
            ("HTTP_PROXY", "http://upper:1"),
            ("http_proxy", "http://lower:2"),
        ])
        .unwrap();
        let target = Target::parse("http://example.com/").unwrap();
        let Source::Routes(routes) = vars.source_for(&target).unwrap() else {
            panic!("not routes");
        };
        assert!(matches!(&routes[0], Route::Proxy(p) if p.host() == "lower"));
    }

    #[test]
    fn a_websocket_scheme_falls_back_to_http_and_everything_to_all() {
        let vars = variables(&[
            ("https_proxy", "http://secure:1"),
            ("all_proxy", "http://any:2"),
        ])
        .unwrap();
        let wss = Target::parse("wss://example.com/").unwrap();
        let ftp = Target::parse("ftp://example.com/").unwrap();
        let host = |target: &Target| match vars.source_for(target) {
            Some(Source::Routes(routes)) => match &routes[0] {
                Route::Proxy(p) => p.host().to_string(),
                Route::Direct => "direct".into(),
            },
            _ => "none".into(),
        };
        assert_eq!(host(&wss), "secure");
        assert_eq!(host(&ftp), "any");
    }

    #[test]
    fn a_value_may_be_a_pac_result_a_pac_location_or_a_proxy_url() {
        assert!(
            matches!(Source::parse("PROXY a:1; DIRECT"), Some(Source::Routes(r)) if r.len() == 2)
        );
        assert!(matches!(Source::parse("direct"), Some(Source::Routes(r)) if r == [Route::Direct]));
        assert!(matches!(
            Source::parse("http://wpad.corp/wpad.dat"),
            Some(Source::PacLocation(_))
        ));
        assert!(matches!(
            Source::parse("file:///etc/proxy.pac"),
            Some(Source::PacLocation(_))
        ));
        assert!(matches!(
            Source::parse("http://proxy:3128"),
            Some(Source::Routes(_))
        ));
        assert!(matches!(
            Source::parse("proxy:3128"),
            Some(Source::Routes(_))
        ));
        assert!(matches!(
            Source::parse("function FindProxyForURL(u,h){return 'DIRECT';}"),
            Some(Source::PacScript(_))
        ));
    }

    #[test]
    fn inside_a_cgi_request_an_uppercase_http_proxy_is_ignored() {
        let cgi = [("REQUEST_METHOD", "GET"), ("HTTP_PROXY", "http://evil:1")];
        assert!(variables(&cgi).is_none());
        let with_lowercase = [
            ("REQUEST_METHOD", "GET"),
            ("HTTP_PROXY", "http://evil:1"),
            ("http_proxy", "http://ok:2"),
        ];
        assert!(variables(&with_lowercase).unwrap().has_proxies());
    }

    #[test]
    fn no_proxy_alone_makes_a_list_but_no_proxies() {
        let vars = variables(&[("NO_PROXY", "corp.example")]).unwrap();
        assert!(!vars.has_proxies());
        assert!(vars.no_proxy().matches("http://a.corp.example/"));
    }
}
