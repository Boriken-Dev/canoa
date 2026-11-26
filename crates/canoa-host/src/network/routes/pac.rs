//! The `pac` route: a PAC script chooses the proxy of each destination.

use canoa::network_route::{
    Checked, LeaseStatus, NetworkRoute, OpenRequest, Reach, RouteInfo, RouteLease, Via,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Secret, ToValue};
use guatiao_intake::Form;

use super::shared::{Lease, absolute, judge, tls};

/// What the `pac` route is set with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(deny_unknown_fields)]
#[form(
    section(id = "script", label = "Script"),
    section(id = "login", label = "Login"),
    section(id = "tls", label = "Certificates")
)]
pub struct PacSettings {
    /// Where the script is: a URL, a `file:` URL or a path.
    #[schema(section = "script", title = "Script location")]
    #[form(placeholder = "http://wpad.example.com/proxy.pac")]
    #[map(default)]
    pub location: String,
    /// The hosts that go direct, in `NO_PROXY` form, replacing the system's list.
    #[schema(section = "script", title = "Hosts that go direct")]
    pub no_proxy: Option<Vec<String>>,
    /// A login for every proxy the script names that names none of its own.
    #[schema(section = "login", title = "User name")]
    pub user: Option<String>,
    /// The name of the host variable that holds that login's password.
    #[schema(section = "login", title = "Password variable")]
    pub password_env: Option<String>,
    /// Whether a proxy that asks for Negotiate or NTLM is answered with the logon session.
    #[schema(advanced, section = "login", title = "Answer Negotiate login")]
    pub negotiate: Option<bool>,
    /// `system`, `none`, or the path of a PEM file holding a certificate chain and its unencrypted private key.
    #[schema(advanced, section = "tls", title = "Client certificate")]
    pub client_certificate: Option<String>,
    /// Trust exactly the certificates of this PEM file, replacing the system's trust.
    #[schema(advanced, section = "tls", title = "Trust only this file")]
    pub ca_file: Option<String>,
    /// `system` or `git`.
    #[schema(advanced, section = "tls", title = "Git's TLS library")]
    pub git_tls: Option<String>,
}

/// A PAC script's choice.
#[derive(Provider)]
#[provider(NetworkRoute, id = "pac", config = PacSettings, form = PacSettings)]
pub struct PacRoute {
    settings: PacSettings,
}

impl From<PacSettings> for PacRoute {
    fn from(settings: PacSettings) -> Self {
        Self { settings }
    }
}

impl PacRoute {
    fn reach(&self, base: Option<&str>, password: Option<String>) -> Reach {
        let settings = &self.settings;
        let location = settings.location.trim();
        let mut reach = Reach::default();
        reach.via = Via::Pac;
        // A script's own text and a URL are kept as written; only a path is made absolute.
        reach.pac = Some(
            if location.contains("://")
                || location.starts_with("file:")
                || location.contains("FindProxyForURL")
            {
                location.to_owned()
            } else {
                absolute(base, location)
            },
        );
        reach.no_proxy.clone_from(&settings.no_proxy);
        reach.user.clone_from(&settings.user);
        reach.password = password.map(Secret::new);
        reach.negotiate = settings.negotiate;
        tls(
            &mut reach,
            base,
            settings.client_certificate.as_deref(),
            settings.ca_file.as_deref(),
            settings.git_tls.as_deref(),
        );
        reach
    }

    fn password(&self, request: &OpenRequest) -> Option<String> {
        self.settings
            .password_env
            .as_deref()
            .and_then(|name| request.variable(name))
            .map(str::to_owned)
    }

    fn problems(&self, request: &OpenRequest) -> Vec<String> {
        let settings = &self.settings;
        let mut problems = Vec::new();
        if settings.location.trim().is_empty() {
            problems.push("location names no script".to_owned());
        }
        match (&settings.user, &settings.password_env) {
            (None, Some(_)) => problems.push("password_env goes with user".to_owned()),
            (Some(_), Some(name)) if request.variable(name).is_none() => problems.push(format!(
                "password_env names {name}, a variable that is not set"
            )),
            _ => {}
        }
        problems
    }
}

impl NetworkRoute for PacRoute {
    fn describe(&self) -> Result<RouteInfo, ProviderError> {
        let mut info = RouteInfo::new("PAC script");
        info.description =
            Some("A proxy auto-config script chooses the proxy of each destination.".to_owned());
        info.secrets = vec!["password_env".to_owned()];
        Ok(info)
    }

    fn check(&self, request: OpenRequest) -> Result<Checked, ProviderError> {
        let problems = self.problems(&request);
        if !problems.is_empty() {
            return Ok(Checked::of(problems));
        }
        let reach = self.reach(request.base_dir.as_deref(), self.password(&request));
        Ok(judge(&reach, problems))
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn RouteLease>, ProviderError> {
        let mut problems = self.problems(&request);
        if problems.is_empty() {
            let reach = self.reach(request.base_dir.as_deref(), self.password(&request));
            problems = judge(&reach, problems).problems;
        }
        if let Some(problem) = problems.first() {
            return Err(canoa::status::bad_value(problem));
        }
        let base = request.base_dir.clone();
        let password = self.password(&request);
        let settings = self.settings.clone();
        Ok(Lease::new(
            move || PacRoute::from(settings.clone()).reach(base.as_deref(), password.clone()),
            LeaseStatus::new("chosen by the PAC script"),
            (),
        )
        .into_object())
    }
}
