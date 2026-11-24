//! The WPAD URL a DHCP server handed out (option 252), from the lease data Unix systems keep.
//!
//! Reading the option from the network would mean broadcasting `DHCPINFORM` from port 68, which
//! the system's own DHCP client holds, so the lease the client recorded is read instead:
//! NetworkManager's `nmcli` and the lease files of `dhclient`. Windows asks WinHTTP, which does
//! its own DHCP discovery.

#![cfg_attr(
    not(any(unix, test)),
    allow(dead_code, reason = "only the Unix readers parse lease data")
)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The names the lease data gives option 252.
const NAMES: &[&str] = &[
    "wpad",
    "wpad-url",
    "wpad_url",
    "option-252",
    "option_252",
    "private_252",
    "proxy-autodiscovery",
    "proxy_auto_discovery",
];

fn is_option_252(name: &str) -> bool {
    NAMES.iter().any(|known| name.eq_ignore_ascii_case(known))
}

fn is_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && !text.contains(char::is_whitespace)
}

/// The URLs in `nmcli -t -f DHCP4,DHCP6 device show` output: lines like
/// `DHCP4.OPTION[7]:wpad = http://wpad.corp.example/wpad.dat`.
pub(crate) fn from_nmcli(output: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for line in output.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if !key.contains(".OPTION[") {
            continue;
        }
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_end_matches('\0');
        if is_option_252(name.trim()) && is_url(value) && !urls.contains(&value.to_string()) {
            urls.push(value.to_string());
        }
    }
    urls
}

/// The URLs in an ISC `dhclient` lease file, the last lease's first: lines like
/// `option wpad "http://wpad.corp.example/wpad.dat";`.
pub(crate) fn from_dhclient(contents: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for line in contents.lines().rev() {
        let Some(rest) = line.trim().strip_prefix("option ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim().trim_end_matches(';').trim().trim_matches('"');
        if is_option_252(name) && is_url(value) && !urls.contains(&value.to_string()) {
            urls.push(value.to_string());
        }
    }
    urls
}

/// The WPAD URLs this machine's DHCP client recorded, the most recent first.
#[cfg(unix)]
pub(crate) fn urls(reader: &crate::settings::ReaderSettings) -> Vec<String> {
    use std::time::SystemTime;

    let mut urls = Vec::new();
    if let Some(output) = crate::system::process::run(
        "nmcli",
        &["-t", "-f", "DHCP4,DHCP6", "device", "show"],
        reader.command_timeout,
    ) {
        urls.extend(from_nmcli(&output));
    }
    let mut files: Vec<(SystemTime, String)> = Vec::new();
    for directory in &reader.dhcp_lease_paths {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.contains("lease") {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
                continue;
            };
            if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                files.push((modified, contents));
            }
        }
    }
    files.sort_by_key(|file| core::cmp::Reverse(file.0));
    for (_, contents) in files {
        for url in from_dhclient(&contents) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

/// Windows asks WinHTTP, which does its own DHCP discovery.
#[cfg(not(unix))]
pub(crate) fn urls(_reader: &crate::settings::ReaderSettings) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_lists_option_252_under_several_names() {
        let output = "DHCP4.OPTION[1]:dhcp_lease_time = 86400\n\
                      DHCP4.OPTION[2]:wpad = http://wpad.corp.example/wpad.dat\n\
                      DHCP4.OPTION[3]:domain_name = corp.example\n\
                      DHCP6.OPTION[1]:private_252 = https://proxy.corp.example/proxy.pac\n\
                      IP4.ADDRESS[1]:10.0.0.5/24\n";
        assert_eq!(
            from_nmcli(output),
            [
                "http://wpad.corp.example/wpad.dat",
                "https://proxy.corp.example/proxy.pac"
            ]
        );
    }

    #[test]
    fn a_value_that_is_not_a_url_is_not_a_wpad_location() {
        assert!(from_nmcli("DHCP4.OPTION[2]:wpad = \n").is_empty());
        assert!(from_nmcli("DHCP4.OPTION[2]:wpad = not a url\n").is_empty());
        assert!(from_nmcli("DHCP4.OPTION[2]:domain_name = http://x/\n").is_empty());
    }

    #[test]
    fn dhclient_leases_give_the_last_lease_first() {
        let lease = "lease {\n  interface \"eth0\";\n  option wpad \"http://old.corp.example/wpad.dat\";\n}\n\
                     lease {\n  interface \"eth0\";\n  option option-252 \"http://new.corp.example/wpad.dat\";\n  option domain-name \"corp.example\";\n}\n";
        assert_eq!(
            from_dhclient(lease),
            [
                "http://new.corp.example/wpad.dat",
                "http://old.corp.example/wpad.dat"
            ]
        );
    }
}
