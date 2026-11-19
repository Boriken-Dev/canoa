//! Unix desktops other than macOS: GNOME, MATE, KDE, then NetworkManager.
//!
//! There is no session detection, and a machine often carries another desktop's defaults
//! (GNOME's `mode='none'` under KDE), so the first source with a proxy configured wins; when
//! none has one, the first that has settings answers.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(all(unix, not(target_os = "macos")))]
use super::process::run;
use super::{SettingsSource, SettingsState, SystemSettings, proxy_at, proxy_from};
use crate::bypass::{BypassRules, Dialect};
use crate::proxy::ProxyScheme;

/// `gsettings list-recursively` output: `(schema, key)` to value, string quotes dropped.
pub(crate) fn parse_gsettings(output: &str) -> BTreeMap<(String, String), String> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, char::is_whitespace);
            let schema = parts.next()?;
            let key = parts.next()?;
            let value = parts.next()?.trim().trim_matches('\'');
            Some(((schema.to_string(), key.to_string()), value.to_string()))
        })
        .collect()
}

/// A GVariant string array: `['localhost', '*.internal']`, or `@as []` when empty.
pub(crate) fn string_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value.strip_prefix("@as ").unwrap_or(value);
    value
        .trim_matches(['[', ']'])
        .split(',')
        .map(|item| item.trim().trim_matches(['\'', '"']).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// `mode` is `none`, `manual` or `auto`; `auto` with a blank `autoconfig-url` means
/// auto-detection. `None` when the schema is missing.
pub(crate) fn gsettings_from(
    values: &BTreeMap<(String, String), String>,
    schema: &str,
    source: SettingsSource,
) -> Option<SystemSettings> {
    let get = |schema: &str, key: &str| {
        values
            .get(&(schema.to_string(), key.to_string()))
            .map_or("", String::as_str)
    };
    let mode = values.get(&(schema.to_string(), "mode".to_string()))?;
    match mode.as_str() {
        "none" => Some(SystemSettings {
            source,
            state: SettingsState::Direct,
            ..SystemSettings::default()
        }),
        "auto" => {
            let pac_url =
                Some(get(schema, "autoconfig-url").trim().to_string()).filter(|u| !u.is_empty());
            Some(SystemSettings {
                source,
                state: SettingsState::Configured,
                auto_detect: pac_url.is_none(),
                pac_url,
                ..SystemSettings::default()
            })
        }
        "manual" => {
            let mut proxies = BTreeMap::new();
            for (key, scheme) in [
                ("http", ProxyScheme::Http),
                ("https", ProxyScheme::Http),
                ("ftp", ProxyScheme::Http),
                ("socks", ProxyScheme::Socks5),
            ] {
                let child = alloc::format!("{schema}.{key}");
                if let Some(proxy) = proxy_at(get(&child, "host"), get(&child, "port"), scheme) {
                    proxies.insert(key.to_string(), proxy);
                }
            }
            let ignore = string_list(get(schema, "ignore-hosts"));
            Some(SystemSettings {
                source,
                state: if proxies.is_empty() {
                    SettingsState::Direct
                } else {
                    SettingsState::Configured
                },
                proxies,
                bypass: BypassRules::from_entries(
                    ignore.iter().map(String::as_str),
                    Dialect::GSettings,
                ),
                ..SystemSettings::default()
            })
        }
        _ => None,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_gsettings(schema: &str, source: SettingsSource) -> Option<SystemSettings> {
    let mut values = parse_gsettings(&run("gsettings", &["list-recursively", schema])?);
    // Some gsettings versions leave child schemas out of the recursive listing.
    for key in ["http", "https", "ftp", "socks"] {
        let child = alloc::format!("{schema}.{key}");
        if !values.keys().any(|(name, _)| *name == child)
            && let Some(output) = run("gsettings", &["list-recursively", &child])
        {
            values.extend(parse_gsettings(&output));
        }
    }
    gsettings_from(&values, schema, source)
}

/// KDE's `[Proxy Settings]` section: option names lower-cased, as `kioslaverc` is read.
pub(crate) fn parse_kioslaverc(text: &str) -> Option<BTreeMap<String, String>> {
    let mut inside = false;
    let mut found = false;
    let mut section = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            inside = name == "Proxy Settings";
            found |= inside;
            continue;
        }
        if inside
            && !line.starts_with(['#', ';'])
            && let Some((key, value)) = line.split_once('=')
        {
            section.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    found.then_some(section)
}

/// `ProxyType` 0 none, 1 manual, 2 PAC URL, 3 auto-detect, 4 the environment (read elsewhere,
/// so it reads as no settings). `NoProxyFor` is a bypass list; with `ReversedException=true` it
/// lists the hosts that use the proxy.
pub(crate) fn kde_from(section: &BTreeMap<String, String>) -> Option<SystemSettings> {
    let get = |key: &str| section.get(key).map_or("", String::as_str);
    let source = SettingsSource::Kde;
    match get("proxytype").trim() {
        "" | "0" => Some(SystemSettings {
            source,
            state: SettingsState::Direct,
            ..SystemSettings::default()
        }),
        "3" => Some(SystemSettings {
            source,
            state: SettingsState::Configured,
            auto_detect: true,
            ..SystemSettings::default()
        }),
        "2" => {
            let url = get("proxy config script").trim();
            (!url.is_empty()).then(|| SystemSettings {
                source,
                state: SettingsState::Configured,
                pac_url: Some(url.to_string()),
                ..SystemSettings::default()
            })
        }
        "1" => {
            let mut proxies = BTreeMap::new();
            for (setting, key, scheme) in [
                ("httpproxy", "http", ProxyScheme::Http),
                ("httpsproxy", "https", ProxyScheme::Http),
                ("ftpproxy", "ftp", ProxyScheme::Http),
                ("socksproxy", "socks", ProxyScheme::Socks5),
            ] {
                // KDE writes `scheme://host port`, the port after a space.
                let mut parts = get(setting).split_whitespace();
                let text = match (parts.next(), parts.next()) {
                    (Some(host), Some(port)) => alloc::format!("{host}:{port}"),
                    (Some(host), None) => host.to_string(),
                    _ => continue,
                };
                if let Some(proxy) = proxy_from(&text, scheme) {
                    proxies.insert(key.to_string(), proxy);
                }
            }
            let overrides: Vec<&str> = get("noproxyfor").split(',').map(str::trim).collect();
            let dialect = if get("reversedexception").trim().eq_ignore_ascii_case("true") {
                Dialect::KdeReversed
            } else {
                Dialect::Kde
            };
            Some(SystemSettings {
                source,
                state: if proxies.is_empty() {
                    SettingsState::Direct
                } else {
                    SettingsState::Configured
                },
                proxies,
                bypass: BypassRules::from_entries(overrides, dialect),
                ..SystemSettings::default()
            })
        }
        _ => None,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_kde() -> Option<SystemSettings> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        })?;
    let text = std::fs::read_to_string(home.join("kioslaverc")).ok()?;
    kde_from(&parse_kioslaverc(&text)?)
}

/// `None` unless `method` is `auto`: `none` is every connection's default, not a choice to go
/// direct.
pub(crate) fn network_manager_from(
    method: &str,
    pac_url: &str,
    pac_script: &str,
) -> Option<SystemSettings> {
    if method != "auto" {
        return None;
    }
    let pac_url = Some(pac_url.trim().to_string()).filter(|url| !url.is_empty());
    let pac_script =
        Some(pac_script.trim().to_string()).filter(|script| script.contains("FindProxyForURL"));
    Some(SystemSettings {
        source: SettingsSource::NetworkManager,
        state: SettingsState::Configured,
        auto_detect: pac_url.is_none() && pac_script.is_none(),
        pac_url,
        pac_script,
        ..SystemSettings::default()
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_network_manager() -> Option<SystemSettings> {
    let uuids = run("nmcli", &["-g", "UUID", "connection", "show", "--active"])?;
    for uuid in uuids.lines().map(str::trim).filter(|uuid| !uuid.is_empty()) {
        let Some(fields) = run(
            "nmcli",
            &[
                "-g",
                "proxy.method,proxy.pac-url,proxy.pac-script",
                "connection",
                "show",
                uuid,
            ],
        ) else {
            continue;
        };
        let mut lines = fields.lines();
        let (Some(method), Some(url), Some(script)) = (lines.next(), lines.next(), lines.next())
        else {
            continue;
        };
        // Every active connection is read: the proxy is often set on a VPN connection.
        if let Some(settings) = network_manager_from(method.trim(), url, script) {
            return Some(settings);
        }
    }
    None
}

/// The first desktop with a proxy configured, else the first with settings.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn read() -> Option<SystemSettings> {
    let mut first: Option<SystemSettings> = None;
    let readers: [&dyn Fn() -> Option<SystemSettings>; 4] = [
        &|| read_gsettings("org.gnome.system.proxy", SettingsSource::Gnome),
        &|| read_gsettings("org.mate.system.proxy", SettingsSource::Mate),
        &read_kde,
        &read_network_manager,
    ];
    for reader in readers {
        let Some(settings) = reader() else { continue };
        if settings.state == SettingsState::Configured {
            return Some(settings);
        }
        first.get_or_insert(settings);
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    const GNOME_MANUAL: &str = "org.gnome.system.proxy mode 'manual'
org.gnome.system.proxy ignore-hosts ['localhost', '*.internal', '10.0.0.0/8']
org.gnome.system.proxy autoconfig-url ''
org.gnome.system.proxy.http host 'proxy.corp'
org.gnome.system.proxy.http port 3128
org.gnome.system.proxy.https host 'proxy.corp'
org.gnome.system.proxy.https port 3129
org.gnome.system.proxy.socks host ''
org.gnome.system.proxy.socks port 0
";

    #[test]
    fn gnome_manual_mode_gives_proxies_per_scheme_and_a_bypass_list() {
        let values = parse_gsettings(GNOME_MANUAL);
        let settings =
            gsettings_from(&values, "org.gnome.system.proxy", SettingsSource::Gnome).unwrap();
        assert_eq!(settings.state, SettingsState::Configured);
        assert_eq!(
            settings.proxies["http"].to_string(),
            "http://proxy.corp:3128"
        );
        assert_eq!(settings.proxies["https"].port(), 3129);
        assert!(!settings.proxies.contains_key("socks"));
        assert!(settings.bypass.matches("http://a.internal/"));
        assert!(settings.bypass.matches("http://10.9.9.9/"));
    }

    #[test]
    fn gnome_auto_mode_without_a_url_means_auto_detection_and_none_means_direct() {
        let auto = parse_gsettings("s mode 'auto'\ns autoconfig-url ''\n");
        let settings = gsettings_from(&auto, "s", SettingsSource::Gnome).unwrap();
        assert!(settings.auto_detect);
        let auto = parse_gsettings("s mode 'auto'\ns autoconfig-url 'http://pac/p.pac'\n");
        let settings = gsettings_from(&auto, "s", SettingsSource::Gnome).unwrap();
        assert_eq!(settings.pac_url.as_deref(), Some("http://pac/p.pac"));
        let none = parse_gsettings("s mode 'none'\n");
        assert_eq!(
            gsettings_from(&none, "s", SettingsSource::Gnome)
                .unwrap()
                .state,
            SettingsState::Direct
        );
        assert!(gsettings_from(&BTreeMap::new(), "s", SettingsSource::Gnome).is_none());
    }

    #[test]
    fn an_empty_gvariant_array_has_no_items() {
        assert!(string_list("@as []").is_empty());
        assert_eq!(string_list("['a', 'b']"), ["a", "b"]);
    }

    #[test]
    fn kde_manual_settings_read_the_port_after_a_space_and_a_reversed_list() {
        let text = "[General]\nx=1\n[Proxy Settings]\nProxyType=1\nhttpProxy=http://proxy.corp 3128\n\
                    NoProxyFor=corp.example,10.0.0.0/8\nReversedException=false\n";
        let settings = kde_from(&parse_kioslaverc(text).unwrap()).unwrap();
        assert_eq!(
            settings.proxies["http"].to_string(),
            "http://proxy.corp:3128"
        );
        assert!(settings.bypass.matches("http://a.corp.example/"));
        let reversed = text.replace("ReversedException=false", "ReversedException=true");
        let settings = kde_from(&parse_kioslaverc(&reversed).unwrap()).unwrap();
        assert!(!settings.bypass.matches("http://a.corp.example/"));
        assert!(settings.bypass.matches("http://other.example/"));
    }

    #[test]
    fn kde_defers_to_the_environment_for_type_four_and_reads_a_pac_url_for_type_two() {
        let env = parse_kioslaverc("[Proxy Settings]\nProxyType=4\n").unwrap();
        assert!(kde_from(&env).is_none());
        let pac =
            parse_kioslaverc("[Proxy Settings]\nProxyType=2\nProxy Config Script=http://p/x.pac\n")
                .unwrap();
        assert_eq!(
            kde_from(&pac).unwrap().pac_url.as_deref(),
            Some("http://p/x.pac")
        );
        assert!(parse_kioslaverc("[Other]\nProxyType=1\n").is_none());
    }

    #[test]
    fn network_manager_auto_is_a_choice_and_none_is_only_the_default() {
        assert!(network_manager_from("none", "", "").is_none());
        let settings = network_manager_from("auto", "", "").unwrap();
        assert!(settings.auto_detect);
        let settings = network_manager_from("auto", "http://p/x.pac", "").unwrap();
        assert!(!settings.auto_detect);
        let settings = network_manager_from("auto", "", "no function here").unwrap();
        assert!(settings.auto_detect);
    }
}
