//! The profile items that ship with the host as extensions: how a session's programs reach the network, and who they say they are.
//! They are providers of the `profile_item` kind in a library of their own, registered the way an extension's library is, so the host
//! treats them as it treats any item an extension adds.

use canoa::form::Field;
use canoa::profile_item::{
    ApplyRequest, ItemEffect, ItemInfo, NetworkEffect, ProfileItem, Variable, bad_value_error,
};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;
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
        let mut mode = Field::choice(
            "mode",
            "Reach the network",
            ["system", "none", "proxy", "pac", "command"],
        );
        mode.default = Some("system".to_owned());
        mode.help = Some(
            "system: through a proxy the host runs on this computer's loopback address, which follows the system (PAC, login, bypass list). none: no proxy is given to its programs. proxy: the proxy at the address below. pac: as system, choosing by the PAC script below. command: a program of yours that listens and is given to its programs."
                .to_owned(),
        );
        let mut url = Field::new("url", "Address", "text");
        url.help = Some(
            "proxy: [scheme://][user[:password]@]host[:port]. pac: the script's address or a file."
                .to_owned(),
        );
        let mut command = Field::new("command", "Program and arguments", "list");
        command.help = Some("command: the program first, then its arguments; {listen} is replaced by the address host:port it must listen on.".to_owned());
        let mut bypass = Field::new("bypass", "Hosts that go direct", "list");
        bypass.help =
            Some("In the NO_PROXY form. Empty keeps what the network chooses.".to_owned());
        let user = Field::new("user", "Proxy user name", "text");
        let mut password = Field::new("password_variable", "Variable holding the password", "text");
        password.help = Some("The name of an environment variable of the host that holds the proxy user's password; the password is never kept in the profile.".to_owned());
        let mut certificate = Field::new(
            "client_certificate_hosts",
            "Hosts that get your client certificate",
            "list",
        );
        certificate.help = Some(
            "system and pac only. A host (example.com), a host and everything under it (*.example.com) or an address, one per line. For these alone the proxy ends the program's TLS with a certificate of this session's, signed by an authority made for this session in memory (never written to disk, limited to exactly these hosts, gone with the session) that the session's programs alone are told to trust, and makes its own TLS to the host, presenting the client certificate your keystores or [host.network] choose. A program that pins the certificates it trusts, or reads no CA file variable (Windows' own tools), fails for them. Empty ends no TLS."
                .to_owned(),
        );
        info.fields = vec![mode, url, command, bypass, user, password, certificate];
        Ok(info)
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
        let name = Field::new("name", "Name", "text");
        let email = Field::new("email", "Email address", "text");
        let mut account = Field::new("account", "Account label", "text");
        account.help = Some("What AGENTS_ACCOUNT says to the session's programs.".to_owned());
        let mut variables = Field::new("variables", "Other variables", "list");
        variables.help = Some("One NAME=value per line.".to_owned());
        info.fields = vec![name, email, account, variables];
        Ok(info)
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
