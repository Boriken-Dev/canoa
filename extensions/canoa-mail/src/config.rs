//! The service's own table of canoa.cfg, `[host.services.mail]`, read from the JSON text the host gives.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the file's tables are std's"
)]

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The id of the service: the key of its table in canoa.cfg and the namespace of its control methods.
pub(crate) const SERVICE: &str = "mail";

/// `[host.services.mail]`: the mailboxes the `canoa-mail` tools reach. Nothing is offered until an account is named.
#[derive(Clone, Debug, Default)]
pub(crate) struct MailConfig {
    /// `tools`: whether the tools are offered at all (default true).
    pub(crate) tools: Option<bool>,
    /// `accounts.NAME`, in the file's order.
    pub(crate) accounts: IndexMap<String, MailAccount>,
}

/// One `[host.services.mail.accounts.NAME]`. It holds no secret: `password_env` names the variable that does, and `id` names the
/// entry of the host's account store that does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MailAccount {
    /// Which `mail_backend` provider opens this mailbox (for example `ews`, `graph`, `outlook`, or an extension's).
    pub(crate) backend: String,
    /// The id the secret rests under in the host's account store.
    pub(crate) id: Option<String>,
    /// The variable that holds the secret, read before `id`.
    pub(crate) password_env: Option<String>,
    /// How long one call may take, in seconds (default 60, no ceiling).
    pub(crate) timeout_secs: u64,
    /// The backend's own settings, checked by the backend.
    pub(crate) settings: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMail {
    tools: Option<bool>,
    #[serde(default)]
    accounts: IndexMap<String, RawAccount>,
}

#[derive(Deserialize)]
struct RawAccount {
    backend: Option<String>,
    password_env: Option<String>,
    id: Option<String>,
    timeout_secs: Option<u64>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// Whether `name` is an account's name: 1 to 32 lower-case letters, digits, `-` and `_`.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl MailConfig {
    /// The table `text` (JSON of an object) is, or why it is not one this service reads.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let raw: RawMail =
            serde_json::from_str(text).map_err(|error| format!("[host.services.mail] {error}"))?;
        let trim = |value: Option<String>| -> Option<String> {
            value.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
        };
        let mut accounts = IndexMap::new();
        for (name, raw) in raw.accounts {
            let invalid = |what: &str| format!("[host.services.mail.accounts.{name}] {what}");
            if !valid_name(&name) {
                return Err(invalid(
                    "the name is 1 to 32 lower-case letters, digits, - and _",
                ));
            }
            let backend = raw
                .backend
                .as_deref()
                .map(str::trim)
                .filter(|s| canoa::mail_backend::is_valid_id(s))
                .ok_or_else(|| invalid("backend must be 1 to 32 lower-case letters, digits or _"))?
                .to_owned();
            if raw.rest.contains_key("password") {
                return Err(invalid(
                    "password is a setting this host does not keep in canoa.cfg; use password_env or add the account through the host",
                ));
            }
            let timeout_secs = raw.timeout_secs.unwrap_or(60);
            if timeout_secs == 0 {
                return Err(invalid(
                    "timeout_secs is a whole number of seconds, at least 1",
                ));
            }
            let id = match raw.id {
                Some(id) if id.trim().is_empty() => {
                    return Err(invalid("id is empty; set it or add the account again"));
                }
                other => trim(other),
            };
            let password_env = match raw.password_env {
                Some(env) if env.trim().is_empty() => return Err(invalid("password_env is empty")),
                other => trim(other),
            };
            accounts.insert(
                name,
                MailAccount {
                    backend,
                    password_env,
                    id,
                    timeout_secs,
                    settings: raw.rest,
                },
            );
        }
        Ok(Self {
            tools: raw.tools,
            accounts,
        })
    }

    /// Whether the tools are on (the default) and an account is named.
    pub(crate) fn offers_tools(&self) -> bool {
        self.tools != Some(false) && !self.accounts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_names_accounts_that_hold_no_password_and_refuses_what_would_send_one_in_the_clear() {
        let none = MailConfig::parse("{}").expect("parses");
        assert!(none.accounts.is_empty() && none.tools.is_none() && !none.offers_tools());
        let both = MailConfig::parse(
            r#"{"tools": true,
                "accounts": {
                  "work": {"backend": "ews", "server": "https://mail.example.com/EWS/Exchange.asmx", "user": "u", "password_env": "WORK_PW"},
                  "desktop": {"backend": "outlook", "profile": "Work", "timeout_secs": 90}
                }}"#,
        )
        .expect("parses");
        let names: Vec<&str> = both.accounts.keys().map(String::as_str).collect();
        assert_eq!(names, ["work", "desktop"], "the file's order");
        assert!(both.offers_tools());
        assert_eq!(
            both.accounts["work"].password_env.as_deref(),
            Some("WORK_PW")
        );
        assert_eq!(both.accounts["work"].timeout_secs, 60);
        assert_eq!(both.accounts["desktop"].timeout_secs, 90);
        assert_eq!(both.accounts["desktop"].settings["profile"], "Work");
        let off = MailConfig::parse(r#"{"tools": false, "accounts": {"a": {"backend": "ews"}}}"#)
            .expect("parses");
        assert!(!off.offers_tools());
        let custom = MailConfig::parse(
            r#"{"accounts": {"fastmail": {"backend": "imap", "host": "imap.fastmail.com", "id": "fm-1"}}}"#,
        )
        .expect("any custom mail_backend id can be configured");
        assert_eq!(custom.accounts["fastmail"].backend, "imap");
        let slow = MailConfig::parse(
            r#"{"accounts": {"slow": {"backend": "ews", "timeout_secs": 86400}}}"#,
        )
        .expect("no ceiling");
        assert_eq!(slow.accounts["slow"].timeout_secs, 86400);
    }

    #[test]
    fn a_table_that_would_misname_or_leak_is_refused() {
        for (bad, why) in [
            (
                r#"{"accounts": {"a": {"backend": "ews", "password": "x"}}}"#,
                "add the account",
            ),
            (
                r#"{"accounts": {"a": {"backend": "ews", "timeout_secs": 0}}}"#,
                "at least 1",
            ),
            (
                r#"{"accounts": {"a": {"backend": "ews", "id": " "}}}"#,
                "id is empty",
            ),
            (
                r#"{"accounts": {"a": {"backend": "ews", "password_env": ""}}}"#,
                "password_env is empty",
            ),
            (r#"{"accounts": {"a": {}}}"#, "backend must be"),
            (
                r#"{"accounts": {"a": {"backend": "Bad Id"}}}"#,
                "backend must be",
            ),
            (
                r#"{"accounts": {"Bad_Name": {"backend": "outlook"}}}"#,
                "the name is",
            ),
        ] {
            let error = MailConfig::parse(bad).expect_err(bad);
            assert!(error.contains(why), "{bad}: {error}");
        }
        assert!(MailConfig::parse(r#"{"tools": "no"}"#).is_err());
        assert!(MailConfig::parse(r#"{"surprise": 1}"#).is_err());
    }
}
