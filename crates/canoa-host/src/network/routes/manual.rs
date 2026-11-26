//! The `manual` route: the proxy the person types, as `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` say it.

use canoa::network_route::{
    Checked, LeaseStatus, NetworkRoute, OpenRequest, Reach, RouteInfo, RouteLease, Via,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Secret, ToValue};
use guatiao_intake::Form;

use super::shared::{Lease, judge, tls};

/// What the `manual` route is set with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(deny_unknown_fields)]
#[form(
    section(id = "proxy", label = "Proxy"),
    section(id = "login", label = "Login"),
    section(id = "tls", label = "Certificates")
)]
pub struct ManualSettings {
    /// The proxy of `http` addresses: `[scheme://][user[:password]@]host[:port]`.
    #[schema(section = "proxy", title = "HTTP proxy")]
    #[form(placeholder = "proxy.example.com:8080")]
    pub http_proxy: Option<String>,
    /// The proxy of `https` addresses.
    #[schema(section = "proxy", title = "HTTPS proxy")]
    #[form(placeholder = "proxy.example.com:8080")]
    pub https_proxy: Option<String>,
    /// The proxy of every address the two above do not cover: an HTTP proxy, or `socks5://host:port`.
    #[schema(section = "proxy", title = "Any other proxy")]
    #[form(placeholder = "socks5://proxy.example.com:1080")]
    pub all_proxy: Option<String>,
    /// The hosts that go direct, in `NO_PROXY` form: names (a name covers its subdomains), `*.suffix`, networks (`10.0.0.0/8`),
    /// `host:port`, `<-loopback>` to send loopback through the proxy too.
    #[map(default)]
    #[schema(section = "proxy", title = "Hosts that go direct")]
    pub no_proxy: Vec<String>,
    /// A login for every proxy that names none of its own.
    #[schema(section = "login", title = "User name")]
    pub user: Option<String>,
    /// The name of the host variable that holds that login's password: the password itself is never in canoa.cfg.
    #[schema(section = "login", title = "Password variable")]
    #[form(placeholder = "PROXY_PASSWORD")]
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

/// A proxy the person typed.
#[derive(Provider)]
#[provider(NetworkRoute, id = "manual", config = ManualSettings, form = ManualSettings)]
pub struct ManualRoute {
    settings: ManualSettings,
}

impl From<ManualSettings> for ManualRoute {
    fn from(settings: ManualSettings) -> Self {
        Self { settings }
    }
}

impl ManualRoute {
    /// The reach these settings make; `password` is what `password_env` names, read by the host.
    fn reach(&self, base: Option<&str>, password: Option<String>) -> Reach {
        let settings = &self.settings;
        let mut reach = Reach::default();
        reach.via = Via::Proxy;
        reach.http_proxy.clone_from(&settings.http_proxy);
        reach.https_proxy.clone_from(&settings.https_proxy);
        reach.all_proxy.clone_from(&settings.all_proxy);
        reach.no_proxy = Some(settings.no_proxy.clone());
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

    /// What is wrong with the settings and the variables `request` carries.
    fn problems(&self, request: &OpenRequest) -> Vec<String> {
        let settings = &self.settings;
        let mut problems = Vec::new();
        if settings.http_proxy.is_none()
            && settings.https_proxy.is_none()
            && settings.all_proxy.is_none()
        {
            problems.push("name a proxy: http_proxy, https_proxy or all_proxy".to_owned());
        }
        if settings.user.as_deref().is_some_and(str::is_empty) {
            problems.push("user is empty".to_owned());
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

impl NetworkRoute for ManualRoute {
    fn describe(&self) -> Result<RouteInfo, ProviderError> {
        let mut info = RouteInfo::new("Manual proxy");
        info.description = Some(
            "The proxy you type, as HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY say it."
                .to_owned(),
        );
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
            move || ManualRoute::from(settings.clone()).reach(base.as_deref(), password.clone()),
            LeaseStatus::new("through the proxy it names"),
            (),
        )
        .into_object())
    }
}
