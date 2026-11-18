//! The profile items that ship with the host as extensions: how a session's programs reach the network, and who they say they are.
//! They are providers of the `profile_item` kind in a library of their own, registered the way an extension's library is, so the host
//! treats them as it treats any item an extension adds.

use canoa::profile_item::{
    ApplyRequest, ItemEffect, ItemInfo, NetworkEffect, ProfileItem, Variable, bad_value_error,
};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use super::items::values_of;

fn text(values: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    values
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn words(values: &serde_json::Map<String, Value>, name: &str) -> Vec<String> {
    values
        .get(name)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// How the session's programs reach the network.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Through a proxy the host runs on this computer's loopback address, which follows the system.
    System,
    /// No proxy is given to the session's programs.
    None,
    /// The proxy at the address.
    Proxy,
    /// As system, choosing by the PAC script.
    Pac,
    /// A program of the person's that listens and is given to the session's programs.
    Command,
}

const fn default_mode() -> ProxyMode {
    ProxyMode::System
}

/// What the proxy item holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "only its description is read")]
pub struct ProxyValues {
    /// system: through a proxy the host runs on this computer's loopback address, which follows the system (PAC, login, bypass list). none: no proxy is given to its programs. proxy: the proxy at the address below. pac: as system, choosing by the PAC script below. command: a program of yours that listens and is given to its programs.
    #[map(default = default_mode())]
    #[schema(title = "Reach the network")]
    pub mode: ProxyMode,
    /// proxy: [scheme://][user[:password]@]host[:port]. pac: the script's address or a file.
    #[schema(title = "Address")]
    pub url: Option<String>,
    /// command: the program first, then its arguments; {listen} is replaced by the address host:port it must listen on.
    #[map(default)]
    #[schema(title = "Program and arguments")]
    pub command: Vec<String>,
    /// In the NO_PROXY form. Empty keeps what the network chooses.
    #[map(default)]
    #[schema(title = "Hosts that go direct")]
    pub bypass: Vec<String>,
    /// The user the proxy asks for.
    #[schema(title = "Proxy user name")]
    pub user: Option<String>,
    /// The name of an environment variable of the host that holds the proxy user's password; the password is never kept in the profile.
    #[schema(title = "Variable holding the password")]
    pub password_variable: Option<String>,
    /// system and pac only. A host (example.com), a host and everything under it (*.example.com) or an address, one per line. For these alone the proxy ends the program's TLS with a certificate of this session's, signed by an authority made for this session in memory (never written to disk, limited to exactly these hosts, gone with the session) that the session's programs alone are told to trust, and makes its own TLS to the host, presenting the client certificate your keystores or [host.network] choose. A program that pins the certificates it trusts, or reads no CA file variable (Windows' own tools), fails for them. Empty ends no TLS.
    #[map(default)]
    #[schema(title = "Hosts that get your client certificate")]
    pub client_certificate_hosts: Vec<String>,
}

/// How the session's programs reach the network.
#[derive(Default, Provider)]
#[provider(ProfileItem, id = "proxy_item", crate = ::coqui_acp_api::guatiao)]
pub struct ProxyItem;

impl ProfileItem for ProxyItem {
    fn describe(&self) -> Result<ItemInfo, ProviderError> {
        let mut info = ItemInfo::new("proxy", "Proxy");
        info.help = Some(
            "How this session's programs reach the network. By default the host runs a proxy for the session alone, which sends each connection as this computer's own network would."
                .to_owned(),
        );
        info.taking::<ProxyValues>()
    }

    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
        let values = values_of(&request)?;
        let mode = text(&values, "mode").unwrap_or_else(|| "system".to_owned());
        let mut network = NetworkEffect::new(mode.clone());
        network.url = text(&values, "url");
        network.command = words(&values, "command");
        network.bypass = words(&values, "bypass");
        network.user = text(&values, "user");
        network.password_variable = text(&values, "password_variable");
        network.intercept = words(&values, "client_certificate_hosts");
        if let Some(problem) = network.problem() {
            return Err(bad_value_error(problem));
        }
        let mut effect = ItemEffect::none();
        let ended = network.intercept.join(", ");
        effect.network = Some(network);
        effect.notes.push(
            match mode.as_str() {
                "none" => "gives its programs no proxy",
                "proxy" => "gives its programs the proxy it names and uses it for the host's own requests for the session",
                "pac" => "gives its programs a proxy of the host's that chooses by the PAC script",
                "command" => "runs the program it names and gives it to its programs as their proxy",
                _ => "gives its programs a proxy of the host's that follows the system",
            }
            .to_owned(),
        );
        if !ended.is_empty() {
            effect.notes.push(format!(
                "ends TLS for {ended} to present your client certificate to them"
            ));
        }
        Ok(effect)
    }
}

/// What the identity item holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "only its description is read")]
pub struct IdentityValues {
    /// The name git commits under.
    #[schema(title = "Name")]
    pub name: Option<String>,
    /// The address git commits under.
    #[schema(title = "Email address")]
    pub email: Option<String>,
    /// What AGENTS_ACCOUNT says to the session's programs.
    #[schema(title = "Account label")]
    pub account: Option<String>,
    /// One NAME=value per line.
    #[map(default)]
    #[schema(title = "Other variables")]
    pub variables: Vec<String>,
}

/// Who the session's programs say they are.
#[derive(Default, Provider)]
#[provider(ProfileItem, id = "identity_item", crate = ::coqui_acp_api::guatiao)]
pub struct IdentityItem;

impl ProfileItem for IdentityItem {
    fn describe(&self) -> Result<ItemInfo, ProviderError> {
        let mut info = ItemInfo::new("identity", "Identity");
        info.help = Some(
            "The name and address git commits under, the account label the session reports, and any other variable that says who is working."
                .to_owned(),
        );
        info.taking::<IdentityValues>()
    }

    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
        let values = values_of(&request)?;
        let mut effect = ItemEffect::none();
        if let Some(name) = text(&values, "name") {
            effect
                .env
                .push(Variable::new("GIT_AUTHOR_NAME", name.clone()));
            effect.env.push(Variable::new("GIT_COMMITTER_NAME", name));
        }
        if let Some(email) = text(&values, "email") {
            effect
                .env
                .push(Variable::new("GIT_AUTHOR_EMAIL", email.clone()));
            effect.env.push(Variable::new("GIT_COMMITTER_EMAIL", email));
        }
        if let Some(account) = text(&values, "account") {
            effect.env.push(Variable::new("AGENTS_ACCOUNT", account));
        }
        for line in words(&values, "variables") {
            let Some((name, value)) = line.split_once('=') else {
                return Err(bad_value_error("each other variable is written NAME=value"));
            };
            effect.env.push(Variable::new(name.trim(), value));
        }
        if !effect.env.is_empty() {
            effect
                .notes
                .push("tells its programs who is working".to_owned());
        }
        Ok(effect)
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-profile-extras",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ProxyItem, IdentityItem]
);
