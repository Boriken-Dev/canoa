//! Unix desktops other than macOS: GNOME, MATE, KDE, then NetworkManager.
//!
//! There is no session detection, and a machine often carries another desktop's defaults
//! (GNOME's `mode='none'` under KDE), so the first source with a proxy configured wins; when
//! none has one, the first that has settings answers.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
#[cfg(all(unix, not(target_os = "macos")))]
use core::time::Duration;

#[cfg(all(unix, not(target_os = "macos")))]
use super::process::run;
use super::{SettingsSource, SettingsState, SystemSettings, proxy_at, proxy_from};
use crate::bypass::{BypassRules, Dialect};
use crate::proxy::ProxyScheme;
#[cfg(all(unix, not(target_os = "macos")))]
use crate::settings::ReaderSettings;

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
fn read_gsettings(
    schema: &str,
    source: SettingsSource,
    timeout: Duration,
) -> Option<SystemSettings> {
    let mut values = parse_gsettings(&run("gsettings", &["list-recursively", schema], timeout)?);
    // Some gsettings versions leave child schemas out of the recursive listing.
    for key in ["http", "https", "ftp", "socks"] {
        let child = alloc::format!("{schema}.{key}");
        if !values.keys().any(|(name, _)| *name == child)
            && let Some(output) = run("gsettings", &["list-recursively", &child], timeout)
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
fn read_network_manager(timeout: Duration) -> Option<SystemSettings> {
    read_network_manager_nmcli(timeout).or_else(|| read_network_manager_dbus(timeout))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_network_manager_nmcli(timeout: Duration) -> Option<SystemSettings> {
    let uuids = run(
        "nmcli",
        &["-g", "UUID", "connection", "show", "--active"],
        timeout,
    )?;
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
            timeout,
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

/// The object paths in a `dbus-send --print-reply` answer.
#[cfg_attr(
    all(not(unix), not(test)),
    expect(dead_code, reason = "parsed by tests on every platform, used on Unix")
)]
pub(crate) fn dbus_object_paths(reply: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = reply;
    while let Some(start) = rest.find("object path \"") {
        rest = &rest[start + "object path \"".len()..];
        let Some(end) = rest.find('"') else { break };
        paths.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    paths
}

/// The string values of the `proxy` group of a `Settings.Connection.GetSettings` answer.
/// `dbus-send --print-reply` is a debug printer, not a stable format: anything unexpected reads
/// as no settings.
#[cfg_attr(
    all(not(unix), not(test)),
    expect(dead_code, reason = "parsed by tests on every platform, used on Unix")
)]
pub(crate) fn dbus_proxy_group(reply: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let Some(start) = reply.find("string \"proxy\"") else {
        return values;
    };
    let Some(open) = reply[start..].find('[').map(|at| start + at) else {
        return values;
    };
    let mut depth = 0usize;
    let mut end = reply.len();
    for (at, character) in reply[open..].char_indices() {
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = open + at + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut rest = &reply[open..end];
    while let Some(at) = rest.find("string \"") {
        rest = &rest[at + "string \"".len()..];
        let Some(quote) = rest.find('"') else { break };
        let key = &rest[..quote];
        let after = rest[quote + 1..].trim_start();
        let Some(variant) = after.strip_prefix("variant") else {
            rest = &rest[quote..];
            continue;
        };
        let variant = variant.trim_start();
        if key == "method"
            && let Some(number) = variant.strip_prefix("int32 ")
        {
            // NetworkManager sends the method as an enum: 0 none, 1 auto.
            let word = match number.split_whitespace().next() {
                Some("1") => "auto",
                Some("0") => "none",
                _ => "",
            };
            values.insert(key.to_string(), word.to_string());
            rest = &rest[quote..];
            continue;
        }
        let Some(value) = variant.strip_prefix("string \"") else {
            rest = &rest[quote..];
            continue;
        };
        let Some(close) = value.find('"') else { break };
        values.insert(key.to_string(), value[..close].to_string());
        rest = &value[close..];
    }
    values
}

/// NetworkManager's proxy settings over D-Bus, for a machine without `nmcli`.
#[cfg(all(unix, not(target_os = "macos")))]
fn read_network_manager_dbus(timeout: Duration) -> Option<SystemSettings> {
    const DESTINATION: &str = "org.freedesktop.NetworkManager";
    let send = |args: &[&str]| {
        let destination = alloc::format!("--dest={DESTINATION}");
        let mut all = alloc::vec!["--system", "--print-reply", destination.as_str()];
        all.extend_from_slice(args);
        run("dbus-send", &all, timeout)
    };
    let active = send(&[
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.DBus.Properties.Get",
        &alloc::format!("string:{DESTINATION}"),
        "string:ActiveConnections",
    ])?;
    let mut first = None;
    for path in dbus_object_paths(&active) {
        let Some(connection) = send(&[
            &path,
            "org.freedesktop.DBus.Properties.Get",
            &alloc::format!("string:{DESTINATION}.Connection.Active"),
            "string:Connection",
        ]) else {
            continue;
        };
        let Some(settings_path) = dbus_object_paths(&connection).into_iter().next() else {
            continue;
        };
        let Some(reply) = send(&[
            &settings_path,
            &alloc::format!("{DESTINATION}.Settings.Connection.GetSettings"),
        ]) else {
            continue;
        };
        let group = dbus_proxy_group(&reply);
        if group.is_empty() {
            continue;
        }
        let field = |name: &str| group.get(name).map_or("", String::as_str);
        if let Some(settings) =
            network_manager_from(field("method"), field("pac-url"), field("pac-script"))
        {
            return Some(settings);
        }
        first.get_or_insert(());
    }
    None
}

/// The first desktop with a proxy configured, else the first with settings.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn read(reader: &ReaderSettings) -> Option<SystemSettings> {
    let timeout = reader.command_timeout;
    let mut first: Option<SystemSettings> = None;
    let readers: [&dyn Fn() -> Option<SystemSettings>; 4] = [
        &|| read_gsettings("org.gnome.system.proxy", SettingsSource::Gnome, timeout),
        &|| read_gsettings("org.mate.system.proxy", SettingsSource::Mate, timeout),
        &read_kde,
        &|| read_network_manager(timeout),
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

#[cfg(test)]
mod dbus_tests {
    use super::*;

    const REPLY: &str = r#"method return time=1 sender=:1.5 -> destination=:1.99 serial=3 reply_serial=2
   array [
      dict entry(
         string "connection"
         array [
            dict entry(
               string "id"
               variant                   string "wired"
            )
         ]
      )
      dict entry(
         string "proxy"
         array [
            dict entry(
               string "method"
               variant                   int32 1
            )
            dict entry(
               string "pac-url"
               variant                   string "http://wpad.corp.example/proxy.pac"
            )
            dict entry(
               string "pac-script"
               variant                   string ""
            )
         ]
      )
   ]"#;

    #[test]
    fn object_paths_are_read_from_a_reply() {
        let reply = "   variant       array [\n         object path \"/org/freedesktop/NetworkManager/ActiveConnection/3\"\n         object path \"/org/freedesktop/NetworkManager/ActiveConnection/4\"\n      ]";
        assert_eq!(
            dbus_object_paths(reply),
            [
                "/org/freedesktop/NetworkManager/ActiveConnection/3",
                "/org/freedesktop/NetworkManager/ActiveConnection/4"
            ]
        );
        assert!(dbus_object_paths("nothing here").is_empty());
    }

    #[test]
    fn only_the_string_values_of_the_proxy_group_are_read() {
        let group = dbus_proxy_group(REPLY);
        assert_eq!(
            group.get("pac-url").map(String::as_str),
            Some("http://wpad.corp.example/proxy.pac")
        );
        assert_eq!(group.get("pac-script").map(String::as_str), Some(""));
        // `method` is an integer in the reply, and `id` belongs to another group.
        assert_eq!(group.get("method").map(String::as_str), Some("auto"));
        assert!(!group.contains_key("id"));
        assert!(dbus_proxy_group("garbage [ string").is_empty());
    }
}
