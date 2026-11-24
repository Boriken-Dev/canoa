//! WPAD (web proxy auto-discovery): the URL the DHCP server handed out (option 252, read from the
//! lease the system's DHCP client recorded), then `http://wpad.<domain>/wpad.dat` for each parent
//! domain of this host's name, most specific first.

use crate::settings::PacSettings;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::PacScript;
use super::fetch;
use crate::dns::resolve;

/// Second-level labels that country-code registries sell names under (`example.co.uk`).
/// Anyone can register `wpad.<label>.<cc>` there, so devolution stops above them.
const CC_SECOND_LEVEL: &[&str] = &[
    "ac", "ad", "asn", "biz", "co", "com", "ed", "edu", "firm", "gen", "go", "gob", "gov", "govt",
    "gr", "id", "idv", "ind", "info", "lg", "ltd", "me", "mil", "muni", "ne", "net", "nhs", "nom",
    "or", "org", "pe", "per", "plc", "police", "re", "res", "sch", "school", "web",
];

fn is_public_suffix(labels: &[&str]) -> bool {
    match labels {
        [] | [_] => true,
        [second, top] => {
            top.len() == 2 && CC_SECOND_LEVEL.contains(&second.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

/// Parent domains of `fqdn`, most specific first, stopping before a public suffix: a bare TLD
/// or a country-code registry level such as `co.uk`.
pub(crate) fn candidate_domains(fqdn: &str) -> Vec<String> {
    let labels: Vec<&str> = fqdn.trim_end_matches('.').split('.').collect();
    let mut domains = Vec::new();
    for start in 1..labels.len() {
        if is_public_suffix(&labels[start..]) {
            break;
        }
        domains.push(labels[start..].join("."));
    }
    domains
}

/// This host's fully qualified name: the operating system's where it says, else the host name
/// with the search domain of `/etc/resolv.conf`.
pub(crate) fn host_fqdn() -> Option<String> {
    #[cfg(windows)]
    if let Some(name) = crate::system::winhttp::computer_fqdn() {
        return Some(name);
    }
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    if name.contains('.') {
        return Some(name);
    }
    #[cfg(unix)]
    if let Ok(resolv) = std::fs::read_to_string("/etc/resolv.conf") {
        let domain = resolv.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            matches!(words.next(), Some("search" | "domain"))
                .then(|| words.next().map(str::to_string))?
        });
        if let Some(domain) = domain {
            return Some(alloc::format!("{name}.{domain}"));
        }
    }
    None
}

/// A script that downloads from `url` and loads.
fn load(url: &str, settings: &PacSettings) -> Option<PacScript> {
    let body = fetch::get(url, settings).ok()?;
    PacScript::builder(String::from_utf8_lossy(&body).into_owned())
        .settings(settings)
        .build()
        .ok()
}

/// The first `wpad.dat` that downloads and loads, or `None`. A location whose script fails is
/// skipped.
pub(crate) fn discover(fqdn: Option<&str>, settings: &PacSettings) -> Option<PacScript> {
    for url in super::dhcp::urls(&settings.reader) {
        if let Some(script) = load(&url, settings) {
            return Some(script);
        }
    }
    let fqdn = match fqdn {
        Some(name) => name.to_string(),
        None => host_fqdn()?,
    };
    for domain in candidate_domains(&fqdn) {
        let host = alloc::format!("wpad.{domain}");
        if resolve(&host, settings.wpad_probe_timeout).is_none() {
            continue;
        }
        if let Some(script) = load(&alloc::format!("http://{host}/wpad.dat"), settings) {
            return Some(script);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devolution_walks_up_to_but_not_into_a_public_suffix() {
        assert_eq!(
            candidate_domains("pc1.eng.corp.example.com"),
            ["eng.corp.example.com", "corp.example.com", "example.com"]
        );
        assert_eq!(candidate_domains("pc1.example.co.uk"), ["example.co.uk"]);
        assert!(candidate_domains("pc1.com").is_empty());
        assert!(candidate_domains("pc1").is_empty());
    }
}
