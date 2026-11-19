//! The base environment of a session: the variables every layer's `${...}` and every `from_env` sees before any layer is
//! applied.
//!
//! Two kinds go in. The identity variables name what the session runs on, as dotagents stamps them (`AGENTS_HARNESS`,
//! `AGENTS_VENDOR`, `AGENT`, `AGENTS_MODEL`, `AGENTS_PROJECT_ROOT`, plus `AGENTS_ACCOUNT`, and the session's access as
//! `AGENTS_SANDBOX`, `AGENTS_NETWORK` and `AGENTS_APPROVALS`), so one overlay adapts to every harness; the agent is given them too, and a layer's own value for one replaces them. The host's own variables are
//! those the host sets for the session itself; a layer reads them and may set its own from them, and the host does not
//! hand them on a second time.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use super::Layer;
use super::Scalar;

/// What identifies a session, as far as the host knows it when the environment is made. Each is set only when it has a
/// value.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Who<'a> {
    /// The AOS harness id (`claude-code`, `codex`, `pi`, ...); none for an agent Canoa does not know by name.
    pub(crate) harness: Option<&'a str>,
    /// The model the session starts on: the profile's, else the entry's own.
    pub(crate) model: Option<&'a str>,
    /// The effort the session starts on.
    pub(crate) effort: Option<&'a str>,
    /// The version of the program the Store installed for the harness.
    pub(crate) version: Option<&'a str>,
    /// The label of the account the session runs on, never anything of its credentials.
    pub(crate) account: Option<&'a str>,
    /// The account's kind: `subscription`, `api-key`, `gateway` or `local`.
    pub(crate) account_kind: Option<&'a str>,
    /// The account's provider: `anthropic`, `openai`, `xai`, `google`, or the vendor of what runs a local model.
    pub(crate) account_provider: Option<&'a str>,
    /// The profile the connection runs.
    pub(crate) profile: Option<&'a str>,
    /// The loadout the connection started with.
    pub(crate) loadout: Option<&'a str>,
    /// `delegate` when the session is a task another agent started.
    pub(crate) role: Option<&'a str>,
    /// The folder the session works in, where one is known.
    pub(crate) project: Option<&'a str>,
    /// The access the session runs with, where the profile's layers are known: `AGENTS_SANDBOX` (`none`, `workspace` or
    /// `read-only`), `AGENTS_NETWORK` (`true` or `false`) and `AGENTS_APPROVALS` (`ask`, `edits` or `never`).
    pub(crate) access: Option<coqui_acp_harness::Access>,
}

/// The kind and provider of an account, in the words the identity variables use, from the account kind's id and the standards
/// it provides. `harness` is the harness's AOS id, for the program's own login. A kind this does not recognise has none.
pub(crate) fn account_words(
    kind: &str,
    provides: &[String],
    harness: Option<&str>,
) -> (Option<&'static str>, Option<&'static str>) {
    // The vendor a name belongs to: an agent's id, a standard it owns, its registry id or the vendor's own name.
    let provider_of = |name: &str| crate::vendors::resolve(name).and_then(|known| known.vendor);
    if let Some(vendor) = crate::gateways::vendor_of(kind) {
        return (Some("local"), Some(vendor));
    }
    let by_prefix = crate::vendors::all().into_iter().find_map(|known| {
        let vendor = known.vendor?;
        (kind.starts_with(vendor) || kind.starts_with(known.agent)).then_some(vendor)
    });
    let provider = by_prefix
        .or_else(|| {
            if kind == "harness-login" {
                harness.and_then(provider_of)
            } else {
                None
            }
        })
        .or_else(|| provides.iter().find_map(|standard| provider_of(standard)));
    let category = if kind == "harness-login" || kind.ends_with("-login") {
        Some("subscription")
    } else if kind.contains("api-key") {
        Some("api-key")
    } else if kind.contains("auth-token") || kind.contains("gateway") {
        Some("gateway")
    } else {
        None
    };
    (category, provider)
}

/// The variables a session starts from.
#[derive(Clone, Default)]
pub(crate) struct Session {
    identity: Vec<(String, String)>,
    host: Vec<(String, String)>,
}

impl fmt::Debug for Session {
    /// Names only: a value may be a path of the person's.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field(
                "identity",
                &self
                    .identity
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// `name` is `other`, as the platform compares variable names.
fn same(name: &str, other: &str) -> bool {
    name == other || cfg!(windows) && name.eq_ignore_ascii_case(other)
}

impl Session {
    /// The identity variables of `identity`, and `host`'s variables the host sets for the session itself. `own` reads this
    /// process's environment: a variable it already holds is kept, as dotagents keeps one.
    pub(crate) fn new(
        who: Who<'_>,
        host: Vec<(String, String)>,
        own: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let mut found: Vec<(String, String)> = Vec::new();
        let mut put = |name: &str, value: Option<&str>| {
            let value = own(name)
                .filter(|held| !held.is_empty())
                .or_else(|| value.filter(|value| !value.is_empty()).map(str::to_owned));
            if let Some(value) = value {
                found.push((name.to_owned(), value));
            }
        };
        if let Some(harness) = who.harness {
            let known = crate::vendors::by_aos(harness);
            let vendor = known.as_ref().and_then(|known| known.vendor);
            let from_vendor = known
                .as_ref()
                .and_then(|known| known.model_variable)
                .and_then(own);
            put("AGENTS_HARNESS", Some(harness));
            put("AGENTS_VENDOR", vendor);
            put("AGENT", Some(harness));
            put("AGENTS_MODEL", who.model.or(from_vendor.as_deref()));
        }
        put("AGENTS_HOST", Some("canoa"));
        put("AGENTS_HARNESS_VERSION", who.version);
        put("AGENTS_EFFORT", who.effort);
        put("AGENTS_ACCOUNT", who.account);
        put("AGENTS_ACCOUNT_KIND", who.account_kind);
        put("AGENTS_ACCOUNT_PROVIDER", who.account_provider);
        put("AGENTS_PROFILE", who.profile);
        put("AGENTS_LOADOUT", who.loadout);
        put("AGENTS_ROLE", who.role);
        put("AGENTS_PROJECT_ROOT", who.project);
        if let Some(access) = who.access {
            put("AGENTS_SANDBOX", Some(access.sandbox.id()));
            put(
                "AGENTS_NETWORK",
                Some(if access.network { "true" } else { "false" }),
            );
            put("AGENTS_APPROVALS", Some(access.approvals.id()));
        }
        Self {
            identity: found,
            host,
        }
    }

    /// Every variable this holds, with its value.
    pub(crate) fn variables(&self) -> Vec<(String, String)> {
        self.identity.iter().chain(&self.host).cloned().collect()
    }

    /// The identity variables, which the agent is given.
    pub(crate) fn identity(&self) -> &[(String, String)] {
        &self.identity
    }

    /// `name`'s value here.
    fn get(&self, name: &str) -> Option<&str> {
        self.identity
            .iter()
            .chain(&self.host)
            .find(|(held, _)| same(held, name))
            .map(|(_, value)| value.as_str())
    }

    /// `outer`, with this session's variables first.
    pub(crate) fn lookup<'a>(
        &'a self,
        outer: &'a dyn Fn(&str) -> Option<String>,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| self.get(name).map(str::to_owned).or_else(|| outer(name))
    }
}

/// What the layers start a session on for `scalar` (its model or effort): the last layer that sets one, unless a later layer
/// unsets it.
pub(crate) fn starting(layers: &[Layer], scalar: Scalar) -> Option<String> {
    let mut found = None;
    for layer in layers {
        if layer.profile.unset.contains(&scalar) {
            found = None;
        }
        let set = match scalar {
            Scalar::Model => &layer.profile.model,
            Scalar::Effort => &layer.profile.effort,
            Scalar::Mode => &layer.profile.mode,
            Scalar::Context => &None,
        };
        if let Some(set) = set {
            found = Some(set.clone());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;

    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn value<'a>(session: &'a Session, name: &str) -> Option<&'a str> {
        session.get(name)
    }

    #[test]
    fn each_harness_gets_dotagents_id_and_vendor() {
        let loaded: Vec<_> = crate::vendors::all()
            .into_iter()
            .filter_map(|known| known.aos.map(|id| (id, known.vendor.unwrap_or_default())))
            .collect();
        assert!(
            loaded.len() >= 5,
            "the bundled agents are described: {loaded:?}"
        );
        for (id, vendor) in loaded {
            let session = Session::new(
                Who {
                    harness: Some(id),
                    ..Who::default()
                },
                Vec::new(),
                &none,
            );
            assert_eq!(value(&session, "AGENTS_HARNESS"), Some(id));
            assert_eq!(value(&session, "AGENT"), Some(id));
            let expected = (!vendor.is_empty()).then_some(vendor);
            assert_eq!(value(&session, "AGENTS_VENDOR"), expected, "{id}");
        }
    }

    #[test]
    fn nothing_is_ever_empty_and_an_unknown_agent_has_no_identity() {
        let session = Session::new(
            Who {
                harness: None,
                model: Some("m"),
                account: Some(""),
                project: Some(""),
                ..Who::default()
            },
            Vec::new(),
            &none,
        );
        assert_eq!(
            session
                .identity()
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["AGENTS_HOST"]
        );
        let session = Session::new(
            Who {
                harness: Some("pi"),
                model: Some(""),
                ..Who::default()
            },
            Vec::new(),
            &none,
        );
        assert_eq!(value(&session, "AGENTS_MODEL"), None);
    }

    #[test]
    fn a_value_the_person_already_has_is_kept_and_the_model_may_come_from_the_vendor() {
        let own = |name: &str| match name {
            "AGENTS_HARNESS" => Some("mine".to_string()),
            "ANTHROPIC_MODEL" => Some("claude-x".to_string()),
            _ => None,
        };
        let session = Session::new(
            Who {
                harness: Some("claude-code"),
                account: Some("work"),
                ..Who::default()
            },
            Vec::new(),
            &own,
        );
        assert_eq!(value(&session, "AGENTS_HARNESS"), Some("mine"));
        assert_eq!(value(&session, "AGENT"), Some("claude-code"));
        assert_eq!(value(&session, "AGENTS_MODEL"), Some("claude-x"));
        assert_eq!(value(&session, "AGENTS_ACCOUNT"), Some("work"));
        assert_eq!(value(&session, "AGENTS_PROJECT_ROOT"), None);
    }

    #[test]
    fn the_access_is_in_the_environment_only_when_the_session_has_one() {
        use coqui_acp_harness::{Access, Approvals, Sandbox};
        let none_known = Session::new(Who::default(), Vec::new(), &none);
        for name in ["AGENTS_SANDBOX", "AGENTS_NETWORK", "AGENTS_APPROVALS"] {
            assert_eq!(value(&none_known, name), None, "{name}");
        }
        let standard = Session::new(
            Who {
                access: Some(Access::STANDARD),
                ..Who::default()
            },
            Vec::new(),
            &none,
        );
        assert_eq!(value(&standard, "AGENTS_SANDBOX"), Some("none"));
        assert_eq!(value(&standard, "AGENTS_NETWORK"), Some("true"));
        assert_eq!(value(&standard, "AGENTS_APPROVALS"), Some("ask"));
        let mut strict = Access::STANDARD;
        strict.sandbox = Sandbox::ReadOnly;
        strict.network = false;
        strict.approvals = Approvals::Never;
        let own = |name: &str| (name == "AGENTS_APPROVALS").then(|| "mine".to_owned());
        let session = Session::new(
            Who {
                access: Some(strict),
                ..Who::default()
            },
            Vec::new(),
            &own,
        );
        assert_eq!(value(&session, "AGENTS_SANDBOX"), Some("read-only"));
        assert_eq!(value(&session, "AGENTS_NETWORK"), Some("false"));
        assert_eq!(
            value(&session, "AGENTS_APPROVALS"),
            Some("mine"),
            "a value the person already has is kept, as for the others"
        );
    }

    #[test]
    fn the_session_is_read_before_the_process() {
        let session = Session::new(
            Who::default(),
            vec![("CODEX_HOME".to_owned(), "/state".to_owned())],
            &none,
        );
        let outer = |name: &str| (name == "OTHER").then(|| "x".to_owned());
        let lookup = session.lookup(&outer);
        assert_eq!(lookup("CODEX_HOME").as_deref(), Some("/state"));
        assert_eq!(lookup("OTHER").as_deref(), Some("x"));
        assert_eq!(lookup("MISSING"), None);
    }
}
