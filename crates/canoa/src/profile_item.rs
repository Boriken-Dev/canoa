//! The `profile_item` kind: one thing a profile may set, and what it does to a session.
//!
//! A profile is a list of values the person chose for the items the host offers. Which items there are is not Canoa's to say: each
//! is a provider of this kind, the ones Canoa carries (a model, an effort, the access, starting instructions) and the ones an
//! extension adds (a proxy, an identity, anything else) alike. A provider describes itself (its id, its name, the harnesses it
//! applies to and the form it asks for) and, when a session starts, turns the values the profile holds for it into what the session
//! gets: environment variables, program arguments, values of the session's own settings, the network its programs use and notes
//! for the report.
//!
//! The models are plain data, the same in every language.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue, Value};
use guatiao_intake::{Form, Screen};

/// The name of the `profile_item` kind.
pub const KIND: &str = "profile_item";

/// The most characters of a value or a note an effect may carry, so a provider cannot make a session's setup large.
pub const MAX_TEXT: usize = 64 * 1024;

/// What a profile item is, from [`ProfileItem::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ItemInfo {
    /// The id the profile keeps its values under: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying what it sets.
    pub help: Option<String>,
    /// The ids of the harnesses it applies to, as the standard registry of harnesses names them (`claude-code`, the value of
    /// `AGENTS_HARNESS`); none means every harness.
    #[map(default)]
    pub harnesses: Vec<String>,
    /// The values it takes, as a guatiao schema of an object whose members are what its form asks for; none for an item that takes
    /// no values. A member the schema marks sensitive is never shown in clear.
    pub schema: Option<Value>,
    /// How the schema is shown, a guatiao-intake form document (sections, widgets, `visibleWhen`); none shows the form the schema
    /// implies.
    pub form: Option<Value>,
}

impl ItemInfo {
    /// An item applying to every harness, with no fields.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            harnesses: Vec::new(),
            schema: None,
            form: None,
        }
    }

    /// This item taking the values `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Whether it applies to the harness `harness`.
    #[must_use]
    pub fn applies_to(&self, harness: &str) -> bool {
        self.harnesses.is_empty() || self.harnesses.iter().any(|each| each == harness)
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.id.is_empty()
            && self.id.len() <= 32
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(alloc::format!(
                "the item id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the item {:?} has no title", self.id));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// What a session is applied for.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ApplyRequest {
    /// The id of the harness the session runs, as [`ItemInfo::harnesses`] names it; empty when it has none, so an item may differ by
    /// harness.
    pub harness: String,
    /// What the profile holds for the item, as an object keyed by the schema's member names; a member left empty is absent.
    pub values: Value,
}

impl ApplyRequest {
    /// A session on `harness` whose profile holds `values`.
    #[must_use]
    pub fn new(harness: impl Into<String>, values: Value) -> Self {
        Self {
            harness: harness.into(),
            values,
        }
    }
}

/// An environment variable the session's programs are given.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
}

impl Variable {
    /// The variable `name` set to `value`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A value of the session's own settings an item sets: the model, the effort, the mode, the starting instructions (`context`) and
/// the access's parts (`access.sandbox`, `access.network`, `access.approvals`).
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Setting {
    /// The setting's key.
    pub key: String,
    /// Its value, as text (`true` or `false` for a yes or no).
    pub value: String,
}

impl Setting {
    /// The setting `key` set to `value`.
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// One route a session's programs leave by, and the destinations it is for.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RouteRule {
    /// The name of a route the host has: `system`, `direct` or one of `[network.routes]`. A route the host lacks, or one that does
    /// not open, refuses the session's start.
    pub route: String,
    /// The destinations it is for, each a host name, `*.name`, an address or a regular expression ([`HostPattern`]); none means every
    /// destination no other rule names.
    #[map(default)]
    pub hosts: Vec<String>,
}

impl RouteRule {
    /// `route` for every destination no other rule names.
    #[must_use]
    pub fn new(route: impl Into<String>) -> Self {
        Self {
            route: route.into(),
            hosts: Vec::new(),
        }
    }

    /// `route` for the destinations `hosts` name.
    #[must_use]
    pub fn for_hosts(route: impl Into<String>, hosts: Vec<String>) -> Self {
        Self {
            route: route.into(),
            hosts,
        }
    }
}

/// How the session's programs reach the network: the routes of the host that carry what they send, and what the host's proxy for the
/// session does with it. The first rule with hosts that matches a destination decides; then the account's own route for its own
/// addresses; then the rule with no hosts; then the host's route.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RouteEffect {
    /// The routes, in order. Empty leaves the session on the host's route.
    #[map(default)]
    pub routes: Vec<RouteRule>,
    /// Hosts that go direct, in the `NO_PROXY` form; empty keeps what the routes choose.
    #[map(default)]
    pub bypass: Vec<String>,
    /// The destinations whose TLS the host's proxy for the session ends, so as to present a client certificate to them, in order:
    /// the first entry whose pattern matches a destination says which certificate it gets; every destination no entry matches stays a
    /// tunnel the program's own TLS runs through.
    #[map(default)]
    pub intercept: Vec<ClientCertificateHost>,
}

/// What [`ClientCertificateHost::identity`] says when the keystores' own choice is wanted.
pub const SYSTEM_IDENTITY: &str = "system";

fn system_identity() -> String {
    String::from(SYSTEM_IDENTITY)
}

/// One destination pattern of a proxy, and the client certificate presented to the destinations it matches.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ClientCertificateHost {
    /// A host name (`api.example.com`), a name and everything under it (`*.example.com`), an address, or any other regular
    /// expression, matched whole and without regard to case against the destination as the program names it (a name, or an
    /// address): `.*` matches every destination, `(api|auth)[.]example[.]com` two.
    #[schema(title = "Hosts")]
    #[form(placeholder = ".*")]
    pub pattern: String,
    /// `system` presents what the operating system's keystores choose by the authorities the server names, as a browser would; or
    /// the thumbprint (SHA-256, in hex) of one certificate the host lists as available.
    #[map(default = system_identity())]
    #[schema(title = "Certificate")]
    #[form(widget = "client_certificate")]
    pub identity: String,
}

/// Why a host pattern or certificate identity could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The host pattern was empty.
    EmptyPattern,
    /// The host pattern exceeded [`HostPattern::MAX_LEN`].
    PatternTooLong,
    /// The host pattern is neither a host name, a `*.name`, an IP address nor a valid regular expression.
    BadPattern {
        /// The pattern text that failed to compile.
        pattern: String,
        /// What the regular expression parser reported.
        reason: String,
    },
    /// The certificate identity is neither `system` nor a 64-hex-digit SHA-256 thumbprint.
    BadThumbprint(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPattern => f.write_str("a host pattern is empty"),
            Self::PatternTooLong => {
                write!(
                    f,
                    "a host pattern is at most {} characters",
                    HostPattern::MAX_LEN
                )
            }
            Self::BadPattern { pattern, reason } => write!(
                f,
                "{pattern:?} is no host name, *.name or address, and no regular expression: {reason}"
            ),
            Self::BadThumbprint(text) => write!(
                f,
                "{text:?} is neither system nor the SHA-256 thumbprint of a certificate (64 hex digits)"
            ),
        }
    }
}

impl core::error::Error for Error {}

impl ClientCertificateHost {
    /// The entry for `pattern`, presenting the keystores' own choice.
    #[must_use]
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            identity: system_identity(),
        }
    }

    /// The entry for `pattern`, presenting the certificate with `thumbprint`.
    #[must_use]
    pub fn presenting(pattern: impl Into<String>, thumbprint: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            identity: thumbprint.into(),
        }
    }

    /// The thumbprint, normalized to lower-case hex, of the certificate this entry presents; `None` for `system`.
    ///
    /// # Errors
    ///
    /// The identity is neither `system` nor a SHA-256 thumbprint.
    pub fn thumbprint(&self) -> Result<Option<String>, Error> {
        let text = self.identity.trim();
        if text.is_empty() || text.eq_ignore_ascii_case(SYSTEM_IDENTITY) {
            return Ok(None);
        }
        let hex: String = text
            .chars()
            .filter(|c| !matches!(c, ':' | ' ' | '-'))
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Some(hex))
        } else {
            Err(Error::BadThumbprint(text.to_owned()))
        }
    }

    /// The pattern, compiled.
    ///
    /// # Errors
    ///
    /// The pattern is none of a host name, a `*.name`, an address and a regular expression.
    pub fn compile(&self) -> Result<HostPattern, Error> {
        HostPattern::parse(&self.pattern)
    }
}

/// A destination pattern, compiled: a plain host, `*.name` or address, which is matched as written, or a regular expression.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HostPattern {
    /// This host name alone, lower case.
    Name(String),
    /// This name and everything under it, lower case, from `*.name`.
    Under(String),
    /// This address alone.
    Address(core::net::IpAddr),
    /// What the regular expression matches, whole and without regard to case.
    Expression(regex::Regex),
}

impl HostPattern {
    /// The longest pattern read, so a pattern cannot make a regular expression whose compiling a session's setup would pay for.
    pub const MAX_LEN: usize = 1024;

    /// Reads `text`: a host name, `*.name` or an address as such, anything else as a regular expression.
    ///
    /// # Errors
    ///
    /// `text` is empty, too long, or no regular expression.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::EmptyPattern);
        }
        if text.len() > Self::MAX_LEN {
            return Err(Error::PatternTooLong);
        }
        let (under, name) = text
            .strip_prefix("*.")
            .map_or((false, text), |name| (true, name));
        let name = name.trim_end_matches('.');
        if let Ok(address) = name.parse::<core::net::IpAddr>() {
            return Ok(Self::Address(address));
        }
        let plain = !name.is_empty()
            && name.len() <= 253
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
            && !name.starts_with(['.', '-'])
            && !name.ends_with('-')
            && !name.contains("..");
        if plain {
            let name = name.to_ascii_lowercase();
            return Ok(if under {
                Self::Under(name)
            } else {
                Self::Name(name)
            });
        }
        regex::RegexBuilder::new(&alloc::format!("^(?:{text})$"))
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
            .map(Self::Expression)
            .map_err(|error| Error::BadPattern {
                pattern: text.to_owned(),
                reason: error.to_string(),
            })
    }

    /// Whether this is no regular expression: a host, a `*.name` or an address, which a name constraint can say.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        !matches!(self, Self::Expression(_))
    }

    /// Whether `host`, a destination as the program names it (a name or an address, a v6 one with or without brackets), matches.
    #[must_use]
    pub fn matches(&self, host: &str) -> bool {
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let address = host.parse::<core::net::IpAddr>().ok();
        match self {
            Self::Name(name) => *name == host,
            Self::Under(name) => {
                host == *name
                    || host
                        .strip_suffix(name.as_str())
                        .is_some_and(|front| front.ends_with('.'))
            }
            Self::Address(wanted) => address == Some(*wanted),
            Self::Expression(expression) => {
                expression.is_match(&host)
                    || address
                        .is_some_and(|address| expression.is_match(&alloc::format!("{address}")))
            }
        }
    }
}

impl RouteEffect {
    /// Routes that say nothing: the host's own.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Why this effect cannot be applied, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let mut general = 0;
        for rule in &self.routes {
            if rule.route.trim().is_empty() {
                return Some(String::from("a route of the network item has no name"));
            }
            if rule.hosts.is_empty() {
                general += 1;
            }
            for host in &rule.hosts {
                if let Err(error) = HostPattern::parse(host) {
                    return Some(alloc::format!("the hosts of route {}: {error}", rule.route));
                }
            }
        }
        if general > 1 {
            return Some(String::from(
                "only one route of the network item may name no hosts: it is the one for every other destination",
            ));
        }
        self.intercept.iter().find_map(|entry| {
            entry
                .compile()
                .err()
                .map(|error| error.to_string())
                .or_else(|| entry.thumbprint().err().map(|error| error.to_string()))
        })
    }
}

/// What a profile item does to one session, from [`ProfileItem::apply`]. Everything is optional: an item that has nothing to do for
/// the harness answers with nothing and a note that says why.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ItemEffect {
    /// Variables the session's programs are given. A variable the profile's own environment already sets keeps that value, and the
    /// report says so.
    #[map(default)]
    pub env: Vec<Variable>,
    /// Arguments added to the command line of a program the host starts.
    #[map(default)]
    pub args: Vec<String>,
    /// Values of the session's own settings.
    #[map(default)]
    pub settings: Vec<Setting>,
    /// The routes its programs' network leaves by.
    pub network: Option<RouteEffect>,
    /// What the report says of it, a line each, in the person's words.
    #[map(default)]
    pub notes: Vec<String>,
}

impl ItemEffect {
    /// An effect that does nothing.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Why this effect cannot be applied, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let long = |text: &str| text.len() > MAX_TEXT;
        if self
            .env
            .iter()
            .any(|variable| long(&variable.value) || long(&variable.name))
            || self.args.iter().any(|arg| long(arg))
            || self.settings.iter().any(|setting| long(&setting.value))
            || self.notes.iter().any(|note| long(note))
        {
            return Some(alloc::format!(
                "an effect holds a value longer than {MAX_TEXT} characters"
            ));
        }
        for variable in &self.env {
            let name = !variable.name.is_empty()
                && variable.name.len() <= 128
                && variable
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && !variable.name.as_bytes()[0].is_ascii_digit();
            if !name || variable.value.contains('\0') {
                return Some(alloc::format!(
                    "the variable {:?} is not a name of letters, digits and _, or its value holds a NUL",
                    variable.name
                ));
            }
        }
        if self.args.iter().any(|arg| arg.contains('\0')) {
            return Some("an argument holds a NUL".into());
        }
        self.network.as_ref().and_then(RouteEffect::problem)
    }
}

/// One thing a profile may set.
///
/// `describe` answers the same on every call with nothing run. `apply` runs when a session starts, with the values the profile holds
/// for the item already read against its schema; it must change nothing outside the effect it answers with.
#[coqui_acp_api::guatiao::kind(name = "profile_item", crate = ::coqui_acp_api::guatiao)]
pub trait ProfileItem: Send + Sync {
    /// What this item is and the form it asks for.
    fn describe(&self) -> Result<ItemInfo, ProviderError>;

    /// What the values do to a session.
    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use guatiao::{FromValue, Schema, ToValue};
    use guatiao_intake::Form;

    /// The values of a stand-in item.
    #[derive(ToValue, FromValue, Schema, Form)]
    #[form(section(id = "net", label = "Network"))]
    #[allow(dead_code, reason = "only its description is read")]
    struct Values {
        /// Where it goes.
        #[schema(section = "net", title = "Address")]
        url: String,
        /// A secret it is given.
        #[schema(section = "net", title = "Token", sensitive)]
        token: Option<String>,
    }

    #[test]
    fn an_item_with_a_word_for_an_id_a_title_and_a_schema_with_its_form_is_usable() {
        let mut info = ItemInfo::new("proxy", "Proxy").taking::<Values>().unwrap();
        assert_eq!(info.problem(), None);
        assert!(info.schema.is_some() && info.form.is_some());
        assert!(info.applies_to("claude"), "no harnesses named is every one");
        info.harnesses = alloc::vec!["codex".into()];
        assert!(!info.applies_to("claude"));
        assert!(info.applies_to("codex"));
        assert_eq!(
            ItemInfo::new("none", "None").problem(),
            None,
            "an item may take nothing"
        );
    }

    #[test]
    fn an_item_that_is_no_word_has_no_title_or_a_form_its_schema_does_not_fit_is_refused() {
        for id in ["", "Proxy", "a b", &"a".repeat(33)] {
            assert!(ItemInfo::new(id, "T").problem().is_some(), "{id:?}");
        }
        assert!(ItemInfo::new("a", " ").problem().is_some());
        let mut info = ItemInfo::new("a", "A");
        info.form = Some(guatiao::Value::from(true));
        assert!(info.problem().is_some_and(|why| why.contains("no schema")));
        let mut info = ItemInfo::new("a", "A").taking::<Values>().unwrap();
        info.schema = Some(guatiao::Value::from(true));
        assert!(info.problem().is_some());
    }

    #[test]
    fn an_effect_names_its_variables_as_a_program_reads_them_and_its_network_as_one_of_the_modes() {
        let mut effect = ItemEffect::none();
        assert_eq!(effect.problem(), None);
        effect.env.push(Variable::new("GIT_AUTHOR_NAME", "Ana"));
        assert_eq!(effect.problem(), None);
        for name in ["", "1A", "A-B", "A B"] {
            let mut bad = ItemEffect::none();
            bad.env.push(Variable::new(name, "x"));
            assert!(bad.problem().is_some(), "{name:?}");
        }
        let mut bad = ItemEffect::none();
        bad.env.push(Variable::new("A", "x\0"));
        assert!(bad.problem().is_some());
        let mut with = ItemEffect::none();
        let mut network = RouteEffect::new();
        network.routes = alloc::vec![
            RouteRule::for_hosts("gateway", alloc::vec!["*.internal.example".into()]),
            RouteRule::new("corp"),
        ];
        with.network = Some(network);
        assert_eq!(with.problem(), None);
        let mut nameless = ItemEffect::none();
        let mut network = RouteEffect::new();
        network.routes = alloc::vec![RouteRule::new(" ")];
        nameless.network = Some(network);
        assert!(
            nameless
                .problem()
                .is_some_and(|text| text.contains("no name"))
        );
        let mut twice = ItemEffect::none();
        let mut network = RouteEffect::new();
        network.routes = alloc::vec![RouteRule::new("a"), RouteRule::new("b")];
        twice.network = Some(network);
        assert!(
            twice
                .problem()
                .is_some_and(|text| text.contains("no hosts"))
        );
        let mut bad = ItemEffect::none();
        let mut network = RouteEffect::new();
        network.routes = alloc::vec![RouteRule::for_hosts("a", alloc::vec!["(".into()])];
        bad.network = Some(network);
        assert!(bad.problem().is_some());
    }

    #[test]
    fn hosts_that_get_a_client_certificate_are_patterns() {
        let mut network = RouteEffect::new();
        network.intercept = alloc::vec![
            ClientCertificateHost::new("example.com"),
            ClientCertificateHost::new("*.corp.example"),
            ClientCertificateHost::new("10.0.0.7"),
            ClientCertificateHost::new("::1"),
            ClientCertificateHost::new("(api|auth)[.]example[.]com"),
            ClientCertificateHost::new(".*"),
            ClientCertificateHost::presenting("mtls[.].*", "AB:".repeat(31) + "AB"),
        ];
        assert_eq!(network.problem(), None);
        for bad in ["", "(", "[a-", "*", "*.", "a{2,1}"] {
            let mut network = RouteEffect::new();
            network.intercept = alloc::vec![ClientCertificateHost::new(bad)];
            assert!(network.problem().is_some(), "{bad:?}");
        }
        for bad in ["me", "abcd", &"zz".repeat(32)] {
            let mut network = RouteEffect::new();
            network.intercept = alloc::vec![ClientCertificateHost::presenting(".*", bad)];
            assert!(
                network
                    .problem()
                    .is_some_and(|why| why.contains("thumbprint")),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_pattern_is_a_plain_host_a_star_name_an_address_or_a_regular_expression_matched_whole() {
        let matches = |pattern: &str, host: &str| {
            HostPattern::parse(pattern)
                .expect("a pattern")
                .matches(host)
        };
        for (pattern, plain) in [
            ("Api.Example.com", true),
            ("*.corp.example", true),
            ("10.0.0.7", true),
            ("::1", true),
            (".*", false),
            ("(api|auth)[.]example[.]com", false),
            ("mtls[.].*", false),
        ] {
            assert_eq!(
                HostPattern::parse(pattern).expect("a pattern").is_plain(),
                plain,
                "{pattern}"
            );
        }
        for (pattern, yes, no) in [
            (
                "Api.Example.com",
                &["api.example.com", "API.EXAMPLE.COM."][..],
                &[
                    "xapi.example.com",
                    "api.example.com.evil.test",
                    "apiXexample.com",
                ][..],
            ),
            (
                "*.corp.example",
                &["corp.example", "a.b.corp.example"][..],
                &["evilcorp.example", "corp.example.evil.test"][..],
            ),
            (
                "10.0.0.7",
                &["10.0.0.7"][..],
                &["10.0.0.70", "localhost"][..],
            ),
            (
                "::1",
                &["::1", "0:0:0:0:0:0:0:1", "[::1]"][..],
                &["127.0.0.1"][..],
            ),
            (
                "(api|auth)[.]example[.]com",
                &["api.example.com", "AUTH.example.com"][..],
                &["x.api.example.com", "api.example.com.evil.test"][..],
            ),
            (
                "10[.]0[.]0[.][0-9]+",
                &["10.0.0.7", "10.0.0.123"][..],
                &["10.0.1.7", "x10.0.0.7"][..],
            ),
            (
                ".*",
                &["anything.test", "10.1.2.3", "::1", "localhost"][..],
                &[][..],
            ),
        ] {
            for host in yes {
                assert!(matches(pattern, host), "{pattern} matches {host}");
            }
            for host in no {
                assert!(!matches(pattern, host), "{pattern} does not match {host}");
            }
        }
    }

    #[test]
    fn an_identity_is_the_keystores_choice_or_a_thumbprint_in_any_common_spelling() {
        assert_eq!(ClientCertificateHost::new(".*").thumbprint(), Ok(None));
        assert_eq!(
            ClientCertificateHost::presenting(".*", " SYSTEM ").thumbprint(),
            Ok(None)
        );
        let colons = "AB:CD:".repeat(15) + "AB:CD";
        let wanted = "abcd".repeat(16);
        assert_eq!(
            ClientCertificateHost::presenting(".*", colons).thumbprint(),
            Ok(Some(wanted))
        );
    }
}
