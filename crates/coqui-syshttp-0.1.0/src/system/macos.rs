//! macOS System Configuration, read with `scutil --proxy`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::{SettingsSource, SettingsState, SystemSettings, proxy_at};
use crate::bypass::{BypassRules, Dialect};
use crate::proxy::ProxyScheme;

/// One value of scutil's dump.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Text(String),
    Dict(BTreeMap<String, Value>),
    List(Vec<String>),
}

/// Parses the `{ ... }` block that follows `lines[*at]`; arrays become lists of their text values.
fn parse_block(lines: &[&str], at: &mut usize) -> BTreeMap<String, Value> {
    let mut block = BTreeMap::new();
    while *at < lines.len() {
        let line = lines[*at].trim();
        *at += 1;
        if line == "}" {
            break;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let (key, rest) = (key.trim().to_string(), rest.trim());
        match rest {
            "<dictionary> {" => {
                let inner = parse_block(lines, at);
                block.insert(key, Value::Dict(inner));
            }
            "<array> {" => {
                let inner = parse_block(lines, at);
                let mut indexed: Vec<(usize, String)> = inner
                    .into_iter()
                    .filter_map(|(index, value)| match value {
                        Value::Text(text) => Some((index.parse().ok()?, text)),
                        _ => None,
                    })
                    .collect();
                indexed.sort();
                block.insert(
                    key,
                    Value::List(indexed.into_iter().map(|(_, text)| text).collect()),
                );
            }
            _ => {
                block.insert(key, Value::Text(rest.to_string()));
            }
        }
    }
    block
}

/// `scutil --proxy` as a map; empty when it has no dictionary.
pub(crate) fn parse_scutil(output: &str) -> BTreeMap<String, Value> {
    let lines: Vec<&str> = output.lines().map(str::trim_end).collect();
    let Some(start) = lines
        .iter()
        .position(|line| line.trim().ends_with("<dictionary> {"))
    else {
        return BTreeMap::new();
    };
    let mut at = start + 1;
    parse_block(&lines, &mut at)
}

/// The HTTP, HTTPS and FTP entries are HTTP proxies for those schemes; SOCKS is version 5, as
/// Chromium reads it. `ExceptionsList` is a bypass list; `ExcludeSimpleHostnames` adds dotless
/// hosts. `None` for no settings; any settings with nothing switched on are direct.
pub(crate) fn settings_from(settings: &BTreeMap<String, Value>) -> Option<SystemSettings> {
    if settings.is_empty() {
        return None;
    }
    let text = |key: &str| match settings.get(key) {
        Some(Value::Text(text)) => text.as_str(),
        _ => "",
    };
    let enabled = |key: &str| text(key) == "1";
    let mut proxies = BTreeMap::new();
    for (prefix, key, scheme) in [
        ("HTTP", "http", ProxyScheme::Http),
        ("HTTPS", "https", ProxyScheme::Http),
        ("FTP", "ftp", ProxyScheme::Http),
        ("SOCKS", "socks", ProxyScheme::Socks5),
    ] {
        if enabled(&alloc::format!("{prefix}Enable"))
            && let Some(proxy) = proxy_at(
                text(&alloc::format!("{prefix}Proxy")),
                text(&alloc::format!("{prefix}Port")),
                scheme,
            )
        {
            proxies.insert(key.to_string(), proxy);
        }
    }
    let pac_url = enabled("ProxyAutoConfigEnable")
        .then(|| text("ProxyAutoConfigURLString").to_string())
        .filter(|url| !url.is_empty());
    let auto_detect = enabled("ProxyAutoDiscoveryEnable");
    let mut exceptions: Vec<&str> = match settings.get("ExceptionsList") {
        Some(Value::List(list)) => list.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    if enabled("ExcludeSimpleHostnames") {
        exceptions.push("<local>");
    }
    let configured = auto_detect || pac_url.is_some() || !proxies.is_empty();
    Some(SystemSettings {
        source: SettingsSource::MacOs,
        state: if configured {
            SettingsState::Configured
        } else {
            SettingsState::Direct
        },
        auto_detect,
        pac_url,
        pac_script: None,
        proxies,
        bypass: BypassRules::from_entries(exceptions, Dialect::MacOs),
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn read() -> Option<SystemSettings> {
    settings_from(&parse_scutil(&super::process::run("scutil", &["--proxy"])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  ExcludeSimpleHostnames : 1
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 3128
  HTTPProxy : proxy.corp
  HTTPSEnable : 1
  HTTPSPort : 3128
  HTTPSProxy : proxy.corp
  ProxyAutoConfigEnable : 1
  ProxyAutoConfigURLString : http://pac.corp/proxy.pac
  SOCKSEnable : 0
}
";

    #[test]
    fn scutil_output_becomes_proxies_a_pac_url_and_a_bypass_list() {
        let settings = settings_from(&parse_scutil(DUMP)).unwrap();
        assert_eq!(settings.state, SettingsState::Configured);
        assert_eq!(
            settings.proxies["http"].to_string(),
            "http://proxy.corp:3128"
        );
        assert_eq!(settings.proxies["https"].scheme(), ProxyScheme::Http);
        assert!(!settings.proxies.contains_key("socks"));
        assert_eq!(
            settings.pac_url.as_deref(),
            Some("http://pac.corp/proxy.pac")
        );
        assert!(settings.bypass.matches("http://printer.local/"));
        assert!(settings.bypass.matches("http://169.254.1.1/"));
        assert!(settings.bypass.matches("http://intranet/"));
    }

    #[test]
    fn an_empty_dump_is_no_settings_and_a_dump_with_nothing_on_is_direct() {
        assert!(settings_from(&parse_scutil("")).is_none());
        let settings =
            settings_from(&parse_scutil("<dictionary> {\n  HTTPEnable : 0\n}\n")).unwrap();
        assert_eq!(settings.state, SettingsState::Direct);
    }
}
