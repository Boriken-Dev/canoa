//! Windows Internet Options as [`SystemSettings`].

use alloc::string::String;
use alloc::vec::Vec;

use super::{SettingsSource, SettingsState, SystemSettings, parse_proxy_list};
use crate::bypass::{BypassRules, Dialect};
use crate::proxy::{Proxy, ProxyScheme, Route};

/// `WINHTTP_CURRENT_USER_IE_PROXY_CONFIG`, as text.
#[derive(Debug, Default)]
pub(crate) struct InternetOptions {
    pub auto_detect: bool,
    pub auto_config_url: Option<String>,
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
}

/// `ProxyServer` is `host:port` for every scheme or `http=a:80;https=b:443;socks=c:1080`, where
/// `socks=` is SOCKS 4, as Chromium reads it; `ProxyOverride` is a WinINet bypass list. Windows
/// always has settings, so nothing switched on is [`SettingsState::Direct`].
pub(crate) fn settings_from(options: &InternetOptions) -> SystemSettings {
    let proxies = parse_proxy_list(options.proxy.as_deref().unwrap_or(""), ProxyScheme::Socks4);
    let pac_url = options
        .auto_config_url
        .clone()
        .filter(|url| !url.is_empty());
    let configured = options.auto_detect || pac_url.is_some() || !proxies.is_empty();
    SystemSettings {
        source: SettingsSource::Windows,
        state: if configured {
            SettingsState::Configured
        } else {
            SettingsState::Direct
        },
        auto_detect: options.auto_detect,
        pac_url,
        pac_script: None,
        proxies,
        bypass: BypassRules::parse(
            options.proxy_bypass.as_deref().unwrap_or(""),
            Dialect::WinInet,
        ),
    }
}

/// Every proxy of a WinHTTP proxy list that applies to a request of `scheme`, in order.
///
/// The list is `[<scheme>=]<server>[:<port>]` entries separated by `;` or whitespace
/// (`WINHTTP_PROXY_INFO`). Entries for this scheme or for no scheme apply; with none of those,
/// the `http` entries and then the `https` ones are used.
pub(crate) fn routes_from_winhttp_list(scheme: &str, list: &str) -> Vec<Route> {
    let entries: Vec<(String, &str)> = list
        .split([';', ' ', '\t'])
        .filter(|token| !token.is_empty())
        .map(|token| match token.split_once('=') {
            Some((protocol, server)) => (protocol.trim().to_ascii_lowercase(), server),
            None => (String::new(), token),
        })
        .filter(|(_, server)| !server.is_empty())
        .collect();
    for wanted in [[scheme, ""], ["http", "http"], ["https", "https"]] {
        let selected: Vec<Route> = entries
            .iter()
            .filter(|(protocol, _)| wanted.contains(&protocol.as_str()))
            .filter_map(|(_, server)| Proxy::parse(server).ok())
            .map(Route::Proxy)
            .collect();
        if !selected.is_empty() {
            return selected;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_winhttp_list_picks_the_entries_for_the_scheme_then_http_then_https() {
        let hosts = |scheme: &str, list: &str| -> Vec<String> {
            routes_from_winhttp_list(scheme, list)
                .into_iter()
                .filter_map(|route| match route {
                    Route::Proxy(proxy) => Some(proxy.host().to_string()),
                    Route::Direct => None,
                })
                .collect()
        };
        assert_eq!(hosts("https", "http=a:80;https=b:443"), ["b"]);
        assert_eq!(hosts("ftp", "http=a:80;https=b:443"), ["a"]);
        assert_eq!(hosts("ftp", "https=b:443"), ["b"]);
        assert_eq!(hosts("http", "x:1 y:2"), ["x", "y"]);
        assert!(hosts("http", "socks=s:1").is_empty());
    }

    #[test]
    fn nothing_switched_on_is_direct() {
        let settings = settings_from(&InternetOptions::default());
        assert_eq!(settings.state, SettingsState::Direct);
    }

    #[test]
    fn a_proxy_server_and_override_list_are_read_in_wininet_syntax() {
        let settings = settings_from(&InternetOptions {
            proxy: Some("http=a:80;socks=s:1080".into()),
            proxy_bypass: Some("*.corp;<local>".into()),
            ..InternetOptions::default()
        });
        assert_eq!(settings.state, SettingsState::Configured);
        assert_eq!(settings.proxies["socks"].scheme(), ProxyScheme::Socks4);
        assert!(settings.bypass.matches("http://x.corp/"));
        assert!(settings.bypass.matches("http://intranet/"));
    }

    #[test]
    fn auto_detection_and_a_pac_url_make_the_settings_configured() {
        let settings = settings_from(&InternetOptions {
            auto_detect: true,
            auto_config_url: Some("http://wpad/wpad.dat".into()),
            ..InternetOptions::default()
        });
        assert!(settings.auto_detect);
        assert_eq!(settings.pac_url.as_deref(), Some("http://wpad/wpad.dat"));
        assert_eq!(settings.state, SettingsState::Configured);
    }
}
