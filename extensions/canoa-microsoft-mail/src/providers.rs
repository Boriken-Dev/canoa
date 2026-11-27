//! The Guatiao `mail_backend` providers exported by `canoa-microsoft-mail`: `ews`, `graph`, and `outlook`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "network and process timeouts use std::time::Duration"
)]

use core::time::Duration;
use std::sync::Arc;

use canoa::network::{Network, Secret as NetworkSecret};
use canoa_mail::mail_backend::{
    BackendInfo, MailBackend, MailError, Mailbox, OpenMailbox, PreparedAccount, ValidateRequest,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Secret, ToValue};
use guatiao_intake::Form;
use serde_json::{Map, Value};
use url::Url;

use crate::ews::{Auth as EwsAuth, Ews, EwsSettings};
use crate::graph::{DEFAULT_SERVER, Graph, GraphSettings, Wam};
use crate::outlook::Outlook;

/// Exchange sign-in scheme declared by `[host.mail.accounts.NAME] auth = ...`.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum EwsAuthMode {
    /// HTTP Basic authentication using `user` and a stored secret or `password_env`.
    Basic,
    /// The signed-in Windows logon session (Negotiate / NTLM).
    Windows,
}

/// Settings schema and form for the Exchange Server (`ews`) mail backend.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[allow(dead_code, reason = "read via schema/form generation and JSON parsing")]
struct EwsFields {
    #[schema(title = "Server")]
    #[form(placeholder = "https://mail.example.com/EWS/Exchange.asmx")]
    server: Option<String>,
    #[schema(title = "Email address")]
    #[form(placeholder = "ana@example.com")]
    email: Option<String>,
    #[schema(title = "User name")]
    #[form(placeholder = "EXAMPLE\\ana")]
    user: Option<String>,
    #[schema(title = "Password")]
    #[form(widget = "password")]
    password: Option<Secret<String>>,
    #[schema(advanced, title = "Autodiscover address")]
    #[form(placeholder = "https://autodiscover.example.com/autodiscover/autodiscover.xml")]
    autodiscover: Option<String>,
    #[schema(advanced, title = "Sign-in")]
    auth: Option<EwsAuthMode>,
}

/// Settings schema and form for the Microsoft Graph (`graph`) mail backend.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[allow(dead_code, reason = "read via schema/form generation and JSON parsing")]
struct GraphFields {
    #[schema(title = "Application (client) ID")]
    #[form(placeholder = "11111111-2222-3333-4444-555555555555")]
    client_id: String,
    #[schema(title = "Tenant")]
    #[form(placeholder = "example.com or organizations")]
    tenant: Option<String>,
    #[schema(title = "Email address")]
    #[form(placeholder = "ana@example.com")]
    email: Option<String>,
    #[schema(
        title = "Allow sending",
        description = "Also ask Microsoft for the permission to send, so the Mail notifier can send from this account. Agents' tools never send."
    )]
    send: Option<bool>,
    #[schema(advanced, title = "Graph service address")]
    #[form(placeholder = "https://graph.microsoft.com/v1.0")]
    server: Option<String>,
    #[schema(advanced, title = "Sign-in helper command")]
    command: Option<Vec<String>>,
}

/// Settings schema and form for the classic Outlook (`outlook`) mail backend.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[allow(dead_code, reason = "read via schema/form generation and JSON parsing")]
struct OutlookFields {
    #[schema(title = "Outlook profile")]
    #[form(placeholder = "current Outlook profile when empty")]
    profile: Option<String>,
    #[schema(advanced, title = "Outlook helper command")]
    command: Option<Vec<String>>,
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn check_http_url(label: &str, url: &str) -> Result<String, MailError> {
    let trimmed = url.trim();
    let parsed = Url::parse(trimmed)
        .map_err(|_| MailError::Bad(format!("{label} must be an http or https address")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(MailError::Bad(format!(
            "{label} must be an http or https address"
        )));
    }
    Ok(trimmed.to_owned())
}

fn parse_map(json_text: &str) -> Result<Map<String, Value>, ProviderError> {
    if json_text.trim().is_empty() {
        return Ok(Map::new());
    }
    serde_json::from_str::<Value>(json_text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| {
            MailError::Bad("mail account settings must be a JSON object".to_owned()).into()
        })
}

fn unknown_field_hint(key: &str) -> String {
    match key {
        "profile" => "profile is only for outlook".to_owned(),
        "client_id" | "tenant" => format!("{key} is only for graph"),
        "server" | "email" | "user" | "auth" | "autodiscover" => {
            format!("{key} is not used by this mail backend")
        }
        other => format!("{other} is not a setting of this mail backend"),
    }
}

fn reject_unknown(map: &Map<String, Value>, allowed: &[&str]) -> Result<(), ProviderError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(MailError::Bad(unknown_field_hint(key)).into());
        }
    }
    Ok(())
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>, ProviderError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(non_empty(Some(text))),
        Some(_) => Err(MailError::Bad(format!("{key} must be text")).into()),
    }
}

fn optional_command(
    map: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, ProviderError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let text = item
                    .as_str()
                    .ok_or_else(|| MailError::Bad(format!("{key} items must be text")))?;
                out.push(text.to_owned());
            }
            if out.is_empty() {
                return Err(MailError::Bad(format!("{key} cannot be empty")).into());
            }
            Ok(Some(out))
        }
        Some(_) => Err(MailError::Bad(format!("{key} must be a list of arguments")).into()),
    }
}

/// Exchange Server (EWS) mail backend provider.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(MailBackend, id = "ews", crate = ::coqui_acp_api::guatiao)]
pub struct EwsBackend;

struct ParsedEws {
    server: Option<String>,
    email: Option<String>,
    user: Option<String>,
    autodiscover: Option<String>,
    password: Option<String>,
    auth: EwsAuthMode,
}

impl EwsBackend {
    fn parse_fields(map: &Map<String, Value>) -> Result<ParsedEws, ProviderError> {
        reject_unknown(
            map,
            &[
                "server",
                "email",
                "user",
                "password",
                "autodiscover",
                "auth",
            ],
        )?;
        let server = optional_string(map, "server")?;
        let email = optional_string(map, "email")?;
        let user = optional_string(map, "user")?;
        let autodiscover = optional_string(map, "autodiscover")?;
        let password = optional_string(map, "password")?;
        let explicit_auth = match map.get("auth") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
                "basic" => Some(EwsAuthMode::Basic),
                "windows" => Some(EwsAuthMode::Windows),
                other => {
                    return Err(MailError::Bad(format!(
                        "ews auth {other:?} must be \"basic\" or \"windows\""
                    ))
                    .into());
                }
            },
            Some(_) => {
                return Err(
                    MailError::Bad("auth must be \"basic\" or \"windows\"".to_owned()).into(),
                );
            }
        };
        let auth = match explicit_auth {
            Some(mode) => mode,
            None => {
                if user.is_some() || password.is_some() {
                    EwsAuthMode::Basic
                } else {
                    EwsAuthMode::Windows
                }
            }
        };
        Ok(ParsedEws {
            server,
            email,
            user,
            autodiscover,
            password,
            auth,
        })
    }
}

impl MailBackend for EwsBackend {
    fn describe(&self) -> Result<BackendInfo, ProviderError> {
        let mut info = BackendInfo::new("ews", "Exchange Server");
        info.description = Some(
            "Connect to Exchange Web Services (EWS) with Basic sign-in or the Windows logon."
                .to_owned(),
        );
        info.no_secret_state = Some("signs in with the Windows logon".to_owned());
        info.taking::<EwsFields>()
    }

    fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, ProviderError> {
        let map = parse_map(&request.settings)?;
        let ParsedEws {
            server,
            email,
            user,
            autodiscover,
            password: inline_password,
            auth,
        } = Self::parse_fields(&map)?;
        let has_secret = request.has_secret || inline_password.is_some();

        let mut check_urls = Vec::new();
        let server = match server {
            Some(url) => {
                let checked = check_http_url("server", &url)?;
                check_urls.push(checked.clone());
                Some(checked)
            }
            None => None,
        };
        let _autodiscover = match autodiscover {
            Some(url) => {
                let checked = check_http_url("autodiscover", &url)?;
                check_urls.push(checked.clone());
                Some(checked)
            }
            None => None,
        };
        if server.is_none() && email.is_none() {
            return Err(MailError::Bad(
                "ews needs a server address or mailbox email address".to_owned(),
            )
            .into());
        }

        let mut prepared = PreparedAccount::new();
        prepared.check_urls = check_urls;
        match auth {
            EwsAuthMode::Windows => {
                if user.is_some() || has_secret {
                    return Err(MailError::Bad(
                        "windows sign-in takes no user, password_env or id".to_owned(),
                    )
                    .into());
                }
                prepared.needs_secret = false;
                prepared.state = Some("signs in with the Windows logon".to_owned());
            }
            EwsAuthMode::Basic => {
                if user.is_none() {
                    return Err(MailError::Bad("basic sign-in needs a user name".to_owned()).into());
                }
                if !has_secret {
                    return Err(MailError::Bad(
                        "basic sign-in needs password_env or an id the host stored when the account was added"
                            .to_owned(),
                    )
                    .into());
                }
                prepared.needs_secret = true;
            }
        }
        if let Some(email) = email {
            prepared.details.push(email);
        }
        if let Some(server) = server {
            prepared.details.push(server);
        }
        if let Some(user) = user {
            prepared.details.push(user);
        }
        Ok(prepared)
    }

    fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError> {
        let map = parse_map(&request.settings)?;
        let ParsedEws {
            server,
            email,
            user,
            autodiscover,
            password: inline_password,
            auth,
        } = Self::parse_fields(&map)?;
        let ews_auth = match auth {
            EwsAuthMode::Windows => EwsAuth::Windows,
            EwsAuthMode::Basic => {
                let user = user
                    .ok_or_else(|| MailError::Bad("basic sign-in needs a user name".to_owned()))?;
                let secret = request
                    .secret
                    .map(|secret| NetworkSecret::new(secret.expose()))
                    .or_else(|| inline_password.map(|raw| NetworkSecret::new(&raw)))
                    .ok_or_else(|| {
                        MailError::Auth("no stored password or environment secret".to_owned())
                    })?;
                EwsAuth::Basic {
                    user,
                    password: secret,
                }
            }
        };
        let ews = Ews::new(
            EwsSettings {
                email,
                server,
                autodiscover,
                auth: ews_auth,
                timeout: Duration::from_secs(request.timeout_secs.max(1)),
            },
            Network::current(),
        );
        Ok(ews.into_object())
    }
}

/// Microsoft Graph mail backend provider.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(MailBackend, id = "graph", crate = ::coqui_acp_api::guatiao)]
pub struct GraphBackend;

struct ParsedGraph {
    client_id: String,
    tenant: Option<String>,
    email: Option<String>,
    server: Option<String>,
    command: Option<Vec<String>>,
    send: bool,
}

impl GraphBackend {
    fn parse_fields(map: &Map<String, Value>) -> Result<ParsedGraph, ProviderError> {
        reject_unknown(
            map,
            &["client_id", "tenant", "email", "server", "command", "send"],
        )?;
        if map.contains_key("autodiscover") {
            return Err(MailError::Bad("graph does not use autodiscover".to_owned()).into());
        }
        let client_id = optional_string(map, "client_id")?.ok_or_else(|| {
            MailError::Bad("graph needs the client_id of an app registration".to_owned())
        })?;
        let tenant = optional_string(map, "tenant")?;
        let email = optional_string(map, "email")?;
        let server = match optional_string(map, "server")? {
            Some(url) => Some(check_http_url("server", &url)?),
            None => None,
        };
        let command = optional_command(map, "command")?;
        let send = match map.get("send") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(send)) => *send,
            Some(_) => return Err(MailError::Bad("send is true or false".to_owned()).into()),
        };
        Ok(ParsedGraph {
            client_id,
            tenant,
            email,
            server,
            command,
            send,
        })
    }
}

impl MailBackend for GraphBackend {
    fn describe(&self) -> Result<BackendInfo, ProviderError> {
        let mut info = BackendInfo::new("graph", "Microsoft Graph");
        info.description = Some(
            "Connect to Microsoft 365 using Microsoft Graph and Windows Web Account Manager."
                .to_owned(),
        );
        info.no_secret_state = Some("signed in by Microsoft".to_owned());
        info.taking::<GraphFields>()
    }

    fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, ProviderError> {
        let map = parse_map(&request.settings)?;
        if let Some(key) = map.keys().find(|k| *k == "autodiscover") {
            return Err(MailError::Bad(format!("graph uses {key} (not autodiscover)")).into());
        }
        let ParsedGraph { email, server, .. } = Self::parse_fields(&map)?;
        if request.has_secret {
            return Err(MailError::Bad(
                "graph signs in by Microsoft and takes no stored password or password_env"
                    .to_owned(),
            )
            .into());
        }
        let mut prepared = PreparedAccount::new();
        prepared.needs_secret = false;
        prepared.state = Some("signed in by Microsoft".to_owned());
        if let Some(url) = &server {
            prepared.check_urls.push(url.clone());
            prepared.details.push(url.clone());
        }
        if let Some(email) = email {
            prepared.details.insert(0, email);
        }
        Ok(prepared)
    }

    fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError> {
        let map = parse_map(&request.settings)?;
        let ParsedGraph {
            client_id,
            tenant,
            email,
            server,
            command,
            send,
        } = Self::parse_fields(&map)?;
        let timeout = Duration::from_secs(request.timeout_secs.max(1));
        let graph = Graph::new(
            GraphSettings {
                server: server.unwrap_or_else(|| DEFAULT_SERVER.to_owned()),
                timeout,
            },
            Arc::new(
                Wam::powershell(
                    command,
                    &client_id,
                    tenant.as_deref().unwrap_or("organizations"),
                    email,
                    timeout,
                )
                .allowing_send(send),
            ),
            Network::current(),
        );
        Ok(graph.into_object())
    }
}

/// Classic Outlook for Windows COM mail backend provider.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(MailBackend, id = "outlook", crate = ::coqui_acp_api::guatiao)]
pub struct OutlookBackend;

impl OutlookBackend {
    fn parse_fields(
        map: &Map<String, Value>,
    ) -> Result<(Option<String>, Option<Vec<String>>), ProviderError> {
        if let Some(bad) = map
            .keys()
            .find(|key| *key != "profile" && *key != "command")
        {
            return Err(MailError::Bad(match bad.as_str() {
                "server" | "email" | "user" | "auth" | "autodiscover" => {
                    format!("outlook signs in as Outlook and takes no {bad}")
                }
                _ => unknown_field_hint(bad),
            })
            .into());
        }
        let profile = optional_string(map, "profile")?;
        let command = optional_command(map, "command")?;
        Ok((profile, command))
    }
}

impl MailBackend for OutlookBackend {
    fn describe(&self) -> Result<BackendInfo, ProviderError> {
        let mut info = BackendInfo::new("outlook", "Outlook");
        info.description =
            Some("Connect to classic Outlook for Windows using your Outlook profile.".to_owned());
        info.no_secret_state = Some("signed in by Outlook".to_owned());
        info.taking::<OutlookFields>()
    }

    fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, ProviderError> {
        let map = parse_map(&request.settings)?;
        let (profile, _command) = Self::parse_fields(&map)?;
        if request.has_secret {
            return Err(MailError::Bad(
                "outlook signs in as Outlook and takes no id or password_env".to_owned(),
            )
            .into());
        }
        let mut prepared = PreparedAccount::new();
        prepared.needs_secret = false;
        prepared.state = Some("signed in by Outlook".to_owned());
        if let Some(profile) = profile {
            prepared.details.push(profile);
        }
        Ok(prepared)
    }

    fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError> {
        let map = parse_map(&request.settings)?;
        let (profile, command) = Self::parse_fields(&map)?;
        let outlook = Outlook::powershell(
            command,
            profile,
            Duration::from_secs(request.timeout_secs.max(1)),
        );
        Ok(outlook.into_object())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-microsoft-mail",
    version = env!("CARGO_PKG_VERSION"),
    providers = [EwsBackend, GraphBackend, OutlookBackend]
);

#[cfg(test)]
mod provider_tests {
    use coqui_acp_api::guatiao::library::Registry;
    use serde_json::json;

    use super::*;

    #[test]
    fn library_registers_ews_graph_and_outlook_with_valid_schemas() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(crate::LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some());

        for (id, expected_title) in [
            ("ews", "Exchange Server"),
            ("graph", "Microsoft Graph"),
            ("outlook", "Outlook"),
        ] {
            let offer = registry
                .offer::<dyn MailBackend>(id)
                .expect("offered")
                .expect("mail_backend");
            let backend = offer.default_instance().expect("instance");
            let info = backend.describe().expect("describe");
            assert_eq!(info.id, id);
            assert_eq!(info.title, expected_title);
            assert_eq!(info.problem(), None, "{id} problem");
            assert!(info.schema.is_some() && info.form.is_some());
        }
    }

    #[test]
    fn ews_graph_and_outlook_validate_settings_and_reject_mismatches() {
        let ok_ews = EwsBackend
            .validate(ValidateRequest::new(
                json!({"server": "https://mail.example.com/EWS/Exchange.asmx", "email": "ana@example.com", "user": "EXAMPLE\\ana"}).to_string(),
                true,
            ))
            .expect("ews basic ok");
        assert!(ok_ews.needs_secret);
        assert_eq!(
            ok_ews.check_urls,
            vec!["https://mail.example.com/EWS/Exchange.asmx"]
        );

        let win_ews = EwsBackend
            .validate(ValidateRequest::new(
                json!({"server": "https://mail.example.com/EWS/Exchange.asmx"}).to_string(),
                false,
            ))
            .expect("ews windows ok");
        assert!(!win_ews.needs_secret);
        assert_eq!(
            win_ews.state.as_deref(),
            Some("signs in with the Windows logon")
        );

        let ok_graph = GraphBackend
            .validate(ValidateRequest::new(
                json!({"client_id": "cid", "tenant": "example.com", "email": "ana@example.com"})
                    .to_string(),
                false,
            ))
            .expect("graph ok");
        assert!(!ok_graph.needs_secret);
        assert_eq!(ok_graph.state.as_deref(), Some("signed in by Microsoft"));

        let ok_outlook = OutlookBackend
            .validate(ValidateRequest::new(
                json!({"profile": "Work"}).to_string(),
                false,
            ))
            .expect("outlook ok");
        assert!(!ok_outlook.needs_secret);
        assert_eq!(ok_outlook.state.as_deref(), Some("signed in by Outlook"));
        assert_eq!(ok_outlook.details, vec!["Work"]);

        for (backend, settings, has_secret, expected) in [
            (
                &EwsBackend as &dyn MailBackend,
                json!({"server": "ftp://mail.example.com/EWS/Exchange.asmx", "user": "u"}),
                true,
                "http or https address",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"user": "u"}),
                true,
                "needs a server",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"email": "a@b", "user": "u"}),
                false,
                "password_env",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"email": "a@b", "auth": "windows", "user": "u"}),
                false,
                "takes no user",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"email": "a@b", "auth": "windows"}),
                true,
                "takes no user",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"email": "a@b", "profile": "p"}),
                false,
                "for outlook",
            ),
            (
                &EwsBackend as &dyn MailBackend,
                json!({"email": "a@b", "client_id": "c"}),
                false,
                "for graph",
            ),
            (
                &GraphBackend as &dyn MailBackend,
                json!({}),
                false,
                "client_id of an app registration",
            ),
            (
                &GraphBackend as &dyn MailBackend,
                json!({"client_id": "c", "server": "graph.example.com/v1.0"}),
                false,
                "http or https address",
            ),
            (
                &OutlookBackend as &dyn MailBackend,
                json!({"server": "https://x"}),
                false,
                "signs in as Outlook",
            ),
            (
                &OutlookBackend as &dyn MailBackend,
                json!({"tenant": "t"}),
                false,
                "for graph",
            ),
        ] {
            let err = backend
                .validate(ValidateRequest::new(settings.to_string(), has_secret))
                .expect_err("should fail");
            assert!(
                err.message.to_string().contains(expected),
                "{settings}: {} (expected {expected:?})",
                err.message
            );
        }
    }
}
