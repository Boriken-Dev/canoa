//! Choosing a client certificate without asking: Chrome's and Edge's `AutoSelectCertificateForUrls`
//! policy, and rules of the same shape.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::identity::CertificateInfo;

/// What a rule requires of a certificate's issuer or subject.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum NameFilter {
    /// Anything.
    #[default]
    Any,
    /// A string found, without regard to case, inside the whole RFC 4514 name.
    Contains(String),
    /// Attributes (`CN`, `OU`, ...) that must each equal one of the name's values exactly.
    Attributes(Vec<(String, String)>),
}

impl NameFilter {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Contains(text) => name.to_lowercase().contains(&text.to_lowercase()),
            Self::Attributes(wanted) => {
                let attributes = split_name(name);
                wanted.iter().all(|(attribute, value)| {
                    attributes.iter().any(|(found, found_value)| {
                        found.eq_ignore_ascii_case(attribute) && found_value == value
                    })
                })
            }
        }
    }
}

/// `(attribute, value)` pairs of an RFC 4514 name, escapes resolved.
fn split_name(name: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let (mut current, mut escaped) = (String::new(), false);
    for character in name.chars().chain(core::iter::once(',')) {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == ',' || character == '+' {
            if let Some((attribute, value)) = current.split_once('=')
                && !attribute.trim().is_empty()
            {
                parts.push((
                    attribute.trim().to_ascii_uppercase(),
                    value.trim().to_string(),
                ));
            }
            current.clear();
        } else {
            current.push(character);
        }
    }
    parts
}

/// A rule that picks a certificate for a host without asking, as the `AutoSelectCertificateForUrls`
/// policy of Chrome and Edge does.
///
/// The host pattern is `example.com` (that host), `[*.]example.com` (it and every subdomain),
/// `*.example.com` (the subdomains) or `*` (any host). A scheme, port or path in it is accepted
/// and ignored: a handshake knows only the server name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoSelectRule {
    pattern: String,
    issuer: NameFilter,
    subject: NameFilter,
}

/// Builds an [`AutoSelectRule`].
#[derive(Debug, Clone)]
#[must_use]
pub struct AutoSelectRuleBuilder {
    rule: AutoSelectRule,
}

impl AutoSelectRuleBuilder {
    /// What the certificate's issuer must look like.
    pub fn issuer(mut self, filter: NameFilter) -> Self {
        self.rule.issuer = filter;
        self
    }

    /// What the certificate's subject must look like.
    pub fn subject(mut self, filter: NameFilter) -> Self {
        self.rule.subject = filter;
        self
    }

    /// The rule.
    pub fn build(self) -> AutoSelectRule {
        self.rule
    }
}

impl AutoSelectRule {
    /// Starts a rule for hosts matching `pattern`.
    pub fn builder(pattern: impl Into<String>) -> AutoSelectRuleBuilder {
        AutoSelectRuleBuilder {
            rule: Self {
                pattern: pattern.into(),
                issuer: NameFilter::Any,
                subject: NameFilter::Any,
            },
        }
    }

    /// The host pattern as given.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// What the certificate's issuer must look like.
    pub fn issuer(&self) -> &NameFilter {
        &self.issuer
    }

    /// What the certificate's subject must look like.
    pub fn subject(&self) -> &NameFilter {
        &self.subject
    }

    /// Whether the rule covers `host`; with no host only `*` does.
    pub fn matches_host(&self, host: Option<&str>) -> bool {
        let mut pattern = self.pattern.trim();
        if let Some((_, rest)) = pattern.split_once("://") {
            pattern = rest;
        }
        pattern = pattern.split('/').next().unwrap_or("");
        let any_subdomain = pattern.starts_with("[*.]");
        if any_subdomain {
            pattern = &pattern[4..];
        }
        if let Some(rest) = pattern.strip_prefix('[') {
            // An IPv6 literal, with or without a port.
            pattern = rest.split(']').next().unwrap_or("");
        } else if pattern.matches(':').count() == 1 {
            pattern = pattern.split(':').next().unwrap_or("");
        }
        let pattern = pattern.trim_end_matches('.').to_lowercase();
        if pattern == "*" {
            return true;
        }
        let Some(host) = host else { return false };
        let host = host.trim_end_matches('.').to_lowercase();
        if let Some(suffix) = pattern.strip_prefix("*.") {
            return host.ends_with(&alloc::format!(".{suffix}"));
        }
        host == pattern || (any_subdomain && host.ends_with(&alloc::format!(".{pattern}")))
    }

    /// Whether `certificate`'s issuer and subject satisfy the rule's filters.
    pub fn matches(&self, certificate: &CertificateInfo) -> bool {
        self.issuer.matches(certificate.issuer()) && self.subject.matches(certificate.subject())
    }
}

fn filter_from(value: Option<&serde_json::Value>) -> Option<NameFilter> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Some(NameFilter::Any);
    };
    let object = value.as_object()?;
    if object.is_empty() {
        return Some(NameFilter::Any);
    }
    let mut attributes = Vec::new();
    for (attribute, value) in object {
        let text = match value {
            serde_json::Value::String(text) => text.clone(),
            // Several values per attribute are not supported.
            serde_json::Value::Array(list) if list.len() == 1 => list[0].as_str()?.to_string(),
            _ => return None,
        };
        attributes.push((attribute.to_ascii_uppercase(), text));
    }
    Some(NameFilter::Attributes(attributes))
}

/// Rules from policy entries, each a JSON object such as
/// `{"pattern": "https://[*.]example.com", "filter": {"ISSUER": {"CN": "Corp CA"}}}`. A malformed
/// entry is skipped, as the browsers skip it.
pub fn rules_from_policy<'a>(entries: impl IntoIterator<Item = &'a str>) -> Vec<AutoSelectRule> {
    let values: Vec<serde_json::Value> = entries
        .into_iter()
        .filter_map(|entry| serde_json::from_str(entry).ok())
        .collect();
    rules_from_policy_values(&values)
}

/// Rules from policy entries already parsed, such as the objects of a managed JSON policy file.
/// A malformed entry is skipped.
pub fn rules_from_policy_values<'a>(
    values: impl IntoIterator<Item = &'a serde_json::Value>,
) -> Vec<AutoSelectRule> {
    values
        .into_iter()
        .filter_map(|value| {
            let pattern = value.get("pattern")?.as_str()?;
            let spec = value.get("filter");
            let issuer = filter_from(spec.and_then(|spec| spec.get("ISSUER")))?;
            let subject = filter_from(spec.and_then(|spec| spec.get("SUBJECT")))?;
            Some(
                AutoSelectRule::builder(pattern)
                    .issuer(issuer)
                    .subject(subject)
                    .build(),
            )
        })
        .collect()
}

/// The `AutoSelectCertificateForUrls` entries of a macOS managed-preferences property list (XML
/// or binary): each a JSON string, or a dictionary of the same shape.
///
/// # Errors
///
/// [`crate::Error::Credential`] when the bytes are not a property list.
#[cfg(feature = "system")]
#[cfg_attr(docsrs, doc(cfg(feature = "system")))]
pub fn rules_from_plist(bytes: &[u8]) -> Result<Vec<AutoSelectRule>, crate::Error> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).map_err(|_| {
        crate::Error::Credential(crate::Reason::new("the property list does not parse"))
    })?;
    let entries: Vec<serde_json::Value> = value
        .as_dictionary()
        .and_then(|dictionary| dictionary.get(POLICY))
        .and_then(plist::Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|item| match item {
                    plist::Value::String(text) => serde_json::from_str(text).ok(),
                    plist::Value::Dictionary(_) => serde_json::to_value(item).ok(),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(rules_from_policy_values(&entries))
}

#[cfg(any(feature = "system", all(unix, not(target_os = "macos"))))]
const POLICY: &str = "AutoSelectCertificateForUrls";

/// The `AutoSelectCertificateForUrls` rules set for Edge, Chrome and Chromium on this machine,
/// machine-wide before per-user, in each browser's order. Windows reads the registry and Linux the
/// managed JSON policy files; macOS reads the managed preferences. Without the `system` feature
/// only Linux reads anything. Empty when none is set.
pub fn browser_policies() -> Vec<AutoSelectRule> {
    platform_rules()
}

#[cfg(all(windows, feature = "system"))]
fn platform_rules() -> Vec<AutoSelectRule> {
    rules_from_policy(registry_entries().iter().map(String::as_str))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_rules() -> Vec<AutoSelectRule> {
    rules_from_policy(json_entries().iter().map(String::as_str))
}

#[cfg(all(target_os = "macos", feature = "system"))]
fn platform_rules() -> Vec<AutoSelectRule> {
    macos_rules()
}

#[cfg(not(any(
    all(windows, feature = "system"),
    all(unix, not(target_os = "macos")),
    all(target_os = "macos", feature = "system")
)))]
fn platform_rules() -> Vec<AutoSelectRule> {
    Vec::new()
}

/// The managed preferences of Edge, Chrome and Chromium: machine-wide, then this user's.
#[cfg(all(target_os = "macos", feature = "system"))]
fn macos_rules() -> Vec<AutoSelectRule> {
    let base = std::path::Path::new("/Library/Managed Preferences");
    let user = std::env::home_dir()
        .and_then(|home| home.file_name().map(std::ffi::OsStr::to_owned))
        .unwrap_or_default();
    let mut rules = Vec::new();
    for directory in [base.to_path_buf(), base.join(user)] {
        for domain in [
            "com.microsoft.Edge",
            "com.google.Chrome",
            "org.chromium.Chromium",
        ] {
            if let Ok(bytes) = std::fs::read(directory.join(alloc::format!("{domain}.plist")))
                && let Ok(found) = rules_from_plist(&bytes)
            {
                rules.extend(found);
            }
        }
    }
    rules
}

#[cfg(all(windows, feature = "system"))]
fn registry_entries() -> Vec<String> {
    let mut entries = Vec::new();
    for hive in [
        windows_registry::LOCAL_MACHINE,
        windows_registry::CURRENT_USER,
    ] {
        for base in [
            r"SOFTWARE\Policies\Microsoft\Edge",
            r"SOFTWARE\Policies\Google\Chrome",
            r"SOFTWARE\Policies\Chromium",
        ] {
            let Ok(key) = hive.open(alloc::format!("{base}\\{POLICY}")) else {
                continue;
            };
            let Ok(values) = key.values() else { continue };
            let mut found: Vec<(Option<u32>, String, String)> = values
                .filter_map(|(name, value)| {
                    let text = String::try_from(value).ok()?;
                    Some((name.parse().ok(), name, text))
                })
                .collect();
            // Numeric names in numeric order, then the others by name.
            found.sort_by(|a, b| (a.0.is_none(), a.0, &a.1).cmp(&(b.0.is_none(), b.0, &b.1)));
            entries.extend(found.into_iter().map(|(_, _, text)| text));
        }
    }
    entries
}

#[cfg(all(unix, not(target_os = "macos")))]
fn json_entries() -> Vec<String> {
    let mut entries = Vec::new();
    for directory in [
        "/etc/opt/edge/policies/managed",
        "/etc/opt/chrome/policies/managed",
        "/etc/chromium/policies/managed",
    ] {
        let Ok(listing) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut paths: Vec<_> = listing
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(document) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if let Some(list) = document.get(POLICY).and_then(|value| value.as_array()) {
                entries.extend(list.iter().filter_map(|value| match value {
                    serde_json::Value::String(text) => Some(text.clone()),
                    object @ serde_json::Value::Object(_) => Some(object.to_string()),
                    _ => None,
                }));
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pattern: &str) -> AutoSelectRule {
        AutoSelectRule::builder(pattern).build()
    }

    #[test]
    fn a_host_pattern_covers_the_host_its_subdomains_or_all() {
        assert!(rule("example.com").matches_host(Some("Example.com")));
        assert!(!rule("example.com").matches_host(Some("a.example.com")));
        assert!(rule("[*.]example.com").matches_host(Some("example.com")));
        assert!(rule("[*.]example.com").matches_host(Some("a.example.com")));
        assert!(rule("*.example.com").matches_host(Some("a.example.com")));
        assert!(!rule("*.example.com").matches_host(Some("example.com")));
        assert!(rule("*").matches_host(None));
        assert!(!rule("example.com").matches_host(None));
    }

    #[test]
    fn a_scheme_port_or_path_in_a_pattern_is_ignored() {
        assert!(rule("https://example.com:8443/path").matches_host(Some("example.com")));
        assert!(rule("[::1]:8443").matches_host(Some("::1")));
    }

    #[test]
    fn a_name_filter_matches_attributes_exactly_or_a_substring_without_case() {
        let name = "CN=Corp CA,OU=Staff,O=Example\\, Inc";
        let attributes = NameFilter::Attributes(alloc::vec![
            ("cn".into(), "Corp CA".into()),
            ("O".into(), "Example, Inc".into())
        ]);
        assert!(attributes.matches(name));
        assert!(!NameFilter::Attributes(alloc::vec![("CN".into(), "Corp".into())]).matches(name));
        assert!(NameFilter::Contains("corp ca".into()).matches(name));
        assert!(!NameFilter::Contains("other".into()).matches(name));
    }

    #[test]
    fn policy_entries_become_rules_and_a_malformed_one_is_skipped() {
        let rules = rules_from_policy([
            r#"{"pattern": "https://[*.]corp.example", "filter": {"ISSUER": {"CN": "Corp CA"}}}"#,
            "not json",
            r#"{"filter": {}}"#,
            r#"{"pattern": "*", "filter": {"SUBJECT": {"OU": ["Staff"]}}}"#,
            r#"{"pattern": "bad.example", "filter": {"ISSUER": {"CN": ["a", "b"]}}}"#,
        ]);
        assert_eq!(rules.len(), 2);
        assert!(rules[0].matches_host(Some("a.corp.example")));
        assert_eq!(
            rules[0].issuer,
            NameFilter::Attributes(alloc::vec![("CN".into(), "Corp CA".into())])
        );
        assert_eq!(
            rules[1].subject,
            NameFilter::Attributes(alloc::vec![("OU".into(), "Staff".into())])
        );
    }
}
