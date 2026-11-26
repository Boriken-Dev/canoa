//! The `system` route: what the system says, as a browser here would follow it.

use canoa::network_route::{
    Checked, LeaseStatus, NetworkRoute, OpenRequest, Reach, RouteInfo, RouteLease,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;

use super::shared::{Lease, judge, tls};

/// What the `system` route is set with. Every key overrides the system's own.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(deny_unknown_fields)]
#[form(
    section(id = "route", label = "Route"),
    section(id = "tls", label = "Certificates")
)]
pub struct SystemSettings {
    /// The hosts that go direct, in `NO_PROXY` form, replacing the system's list: names (a name covers its subdomains),
    /// `*.suffix`, networks (`10.0.0.0/8`), `host:port`, `<-loopback>` to send loopback through the proxy too.
    #[schema(section = "route", title = "Hosts that go direct")]
    pub no_proxy: Option<Vec<String>>,
    /// Whether a proxy that asks for Negotiate or NTLM is answered with the logon session (default true where this build can).
    #[schema(section = "route", title = "Answer a proxy's Negotiate login")]
    pub negotiate: Option<bool>,
    /// `system` (what a browser here would send, from the operating system's keystores), `none`, or the path of a PEM file
    /// holding a certificate chain and its unencrypted private key.
    #[schema(advanced, section = "tls", title = "Client certificate")]
    pub client_certificate: Option<String>,
    /// Trust exactly the certificates of this PEM file, replacing the system's trust.
    #[schema(advanced, section = "tls", title = "Trust only this file")]
    pub ca_file: Option<String>,
    /// `system` (Schannel on Windows, so git uses the system's certificate store) or `git` (git's own and the person's git
    /// configuration).
    #[schema(advanced, section = "tls", title = "Git's TLS library")]
    pub git_tls: Option<String>,
}

/// The system's way out: the proxy variables of the environment, else the operating system's settings, PAC and WPAD included.
#[derive(Provider)]
#[provider(NetworkRoute, id = "system", config = SystemSettings, form = SystemSettings)]
pub struct SystemRoute {
    settings: SystemSettings,
}

impl From<SystemSettings> for SystemRoute {
    fn from(settings: SystemSettings) -> Self {
        Self { settings }
    }
}

impl SystemRoute {
    fn reach(&self, base: Option<&str>) -> Reach {
        let mut reach = Reach::system();
        reach.no_proxy.clone_from(&self.settings.no_proxy);
        reach.negotiate = self.settings.negotiate;
        tls(
            &mut reach,
            base,
            self.settings.client_certificate.as_deref(),
            self.settings.ca_file.as_deref(),
            self.settings.git_tls.as_deref(),
        );
        reach
    }
}

impl NetworkRoute for SystemRoute {
    fn describe(&self) -> Result<RouteInfo, ProviderError> {
        let mut info = RouteInfo::new("System");
        info.description = Some(
            "What the system says: the proxy variables, else the operating system's proxy settings, PAC and WPAD included."
                .to_owned(),
        );
        Ok(info)
    }

    fn check(&self, request: OpenRequest) -> Result<Checked, ProviderError> {
        Ok(judge(&self.reach(request.base_dir.as_deref()), Vec::new()))
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn RouteLease>, ProviderError> {
        let base = request.base_dir.clone();
        let settings = self.settings.clone();
        Ok(Lease::new(
            move || SystemRoute::from(settings.clone()).reach(base.as_deref()),
            LeaseStatus::new("following the system"),
            (),
        )
        .into_object())
    }
}
