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

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue, Value};
use guatiao_intake::{Form, Screen};

/// The name of the `profile_item` kind.
pub const KIND: &str = "profile_item";

/// The versions of the `profile_item` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// An error with status `GUATIAO_ERR_BAD_VALUE`: the values do not fit the item's form.
#[must_use]
pub fn bad_value_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// An error with status `GUATIAO_ERR_INTERNAL`.
#[must_use]
pub fn internal_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

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
    /// What the profile holds for the item, as a JSON object keyed by the schema's member names; a member left empty is absent.
    pub values: String,
}

impl ApplyRequest {
    /// A session on `harness` whose profile holds `values`.
    #[must_use]
    pub fn new(harness: impl Into<String>, values: impl Into<String>) -> Self {
        Self {
            harness: harness.into(),
            values: values.into(),
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

/// How the session's programs reach the network.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct NetworkEffect {
    /// One of [`NETWORK_MODES`].
    pub mode: String,
    /// The proxy's address for `proxy`, or the PAC script's location for `pac`.
    pub url: Option<String>,
    /// For `command`: the program and its arguments, in which `{listen}` stands for the loopback address `host:port` the host
    /// tells it to listen on.
    #[map(default)]
    pub command: Vec<String>,
    /// Hosts that go direct, in the `NO_PROXY` form; empty keeps what the network chooses.
    #[map(default)]
    pub bypass: Vec<String>,
    /// A user name for the proxy, for `proxy` and `pac`.
    pub user: Option<String>,
    /// The name of the host's environment variable that holds that user's password; the password itself is never in a profile.
    pub password_variable: Option<String>,
    /// The destinations whose TLS the host's proxy for the session ends, so as to present a client certificate to them, in order:
    /// the first entry whose pattern matches a destination says which certificate it gets. Only `system` and `pac`, which run that
    /// proxy, take it; every destination no entry matches stays a tunnel the program's own TLS runs through.
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
    pub fn thumbprint(&self) -> Result<Option<String>, String> {
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
            Err(alloc::format!(
                "{text:?} is neither system nor the SHA-256 thumbprint of a certificate (64 hex digits)"
            ))
        }
    }

    /// The pattern, compiled.
    ///
    /// # Errors
    ///
    /// The pattern is none of a host name, a `*.name`, an address and a regular expression.
    pub fn compile(&self) -> Result<HostPattern, String> {
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
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(String::from("a host pattern is empty"));
        }
        if text.len() > Self::MAX_LEN {
            return Err(alloc::format!(
                "a host pattern is at most {} characters",
                Self::MAX_LEN
            ));
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
            .map_err(|error| {
                alloc::format!(
                    "{text:?} is no host name, *.name or address, and no regular expression: {error}"
                )
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

/// The ways [`NetworkEffect::mode`] says the session reaches the network.
///
/// `system`: the host runs a proxy on its loopback address for this session alone, which sends each connection as the host's own
/// network would (the system's proxy with its PAC, WPAD and login), and the session's programs are given it. `none`: the session's
/// programs are given no proxy, and what they inherit is theirs. `proxy`: the proxy at `url`, given to the programs as it is and
/// used by the host's own requests for the session. `pac`: as `system`, choosing by the PAC script at `url`. `command`: a program
/// the host starts for the session, which listens where it is told and is given to the programs as their proxy.
pub const NETWORK_MODES: [&str; 5] = ["system", "none", "proxy", "pac", "command"];

impl NetworkEffect {
    /// A network of `mode`.
    #[must_use]
    pub fn new(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            url: None,
            command: Vec::new(),
            bypass: Vec::new(),
            user: None,
            password_variable: None,
            intercept: Vec::new(),
        }
    }

    /// Why this effect cannot be applied, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !NETWORK_MODES.contains(&self.mode.as_str()) {
            return Some(alloc::format!(
                "the network mode {:?} is none of {NETWORK_MODES:?}",
                self.mode
            ));
        }
        let needs = |what: &str, given: bool| {
            (!given).then(|| alloc::format!("the network mode {} needs {what}", self.mode))
        };
        if let Some(why) = self.intercept_problem() {
            return Some(why);
        }
        match self.mode.as_str() {
            "proxy" | "pac" => needs(
                "an address",
                self.url.as_deref().is_some_and(|url| !url.is_empty()),
            ),
            "command" => needs(
                "a program",
                self.command.first().is_some_and(|word| !word.is_empty()),
            ),
            _ => None,
        }
    }
}

impl NetworkEffect {
    /// Why [`NetworkEffect::intercept`] cannot be used, or `None`.
    fn intercept_problem(&self) -> Option<String> {
        if self.intercept.is_empty() {
            return None;
        }
        if !matches!(self.mode.as_str(), "system" | "pac") {
            return Some(alloc::format!(
                "a client certificate is carried by the host's own proxy, which the network mode {} does not run: use system or pac",
                self.mode
            ));
        }
        self.intercept
            .iter()
            .find_map(|entry| entry.compile().err().or_else(|| entry.thumbprint().err()))
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
    /// The network its programs use.
    pub network: Option<NetworkEffect>,
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
        self.network.as_ref().and_then(NetworkEffect::problem)
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
        for mode in NETWORK_MODES {
            let mut with = ItemEffect::none();
            let mut network = NetworkEffect::new(mode);
            network.url = Some("http://proxy.example:3128".into());
            network.command = alloc::vec!["proxy".into(), "{listen}".into()];
            with.network = Some(network);
            assert_eq!(with.problem(), None, "{mode}");
        }
        for (mode, why) in [
            ("proxy", "an address"),
            ("pac", "an address"),
            ("command", "a program"),
        ] {
            let mut bare = ItemEffect::none();
            bare.network = Some(NetworkEffect::new(mode));
            assert!(
                bare.problem().is_some_and(|text| text.contains(why)),
                "{mode}"
            );
        }
        let mut other = ItemEffect::none();
        other.network = Some(NetworkEffect::new("tor"));
        assert!(other.problem().is_some());
    }

    #[test]
    fn hosts_that_get_a_client_certificate_are_patterns_and_need_the_proxy_the_host_runs() {
        for mode in ["system", "pac"] {
            let mut network = NetworkEffect::new(mode);
            network.url = Some("proxy.pac".into());
            network.intercept = alloc::vec![
                ClientCertificateHost::new("example.com"),
                ClientCertificateHost::new("*.corp.example"),
                ClientCertificateHost::new("10.0.0.7"),
                ClientCertificateHost::new("::1"),
                ClientCertificateHost::new("(api|auth)[.]example[.]com"),
                ClientCertificateHost::new(".*"),
                ClientCertificateHost::presenting("mtls[.].*", "AB:".repeat(31) + "AB"),
            ];
            assert_eq!(network.problem(), None, "{mode}");
        }
        for mode in ["none", "proxy", "command"] {
            let mut network = NetworkEffect::new(mode);
            network.url = Some("http://proxy.example:3128".into());
            network.command = alloc::vec!["proxy".into()];
            network.intercept = alloc::vec![ClientCertificateHost::new("example.com")];
            assert!(
                network
                    .problem()
                    .is_some_and(|why| why.contains("system or pac")),
                "{mode}"
            );
        }
        for bad in ["", "(", "[a-", "*", "*.", "a{2,1}"] {
            let mut network = NetworkEffect::new("system");
            network.intercept = alloc::vec![ClientCertificateHost::new(bad)];
            assert!(network.problem().is_some(), "{bad:?}");
        }
        for bad in ["me", "abcd", &"zz".repeat(32)] {
            let mut network = NetworkEffect::new("system");
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
