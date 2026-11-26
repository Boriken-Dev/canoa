//! The `direct` route: straight to the destination.

use canoa::network_route::{
    Checked, LeaseStatus, NetworkRoute, OpenRequest, Reach, RouteInfo, RouteLease,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;

use super::shared::{Lease, judge, tls};

/// What the `direct` route is set with: only the certificates.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(deny_unknown_fields)]
#[form(section(id = "tls", label = "Certificates"))]
pub struct DirectSettings {
    /// `system` (what a browser here would send, from the operating system's keystores), `none`, or the path of a PEM file
    /// holding a certificate chain and its unencrypted private key.
    #[schema(advanced, section = "tls", title = "Client certificate")]
    pub client_certificate: Option<String>,
    /// Trust exactly the certificates of this PEM file, replacing the system's trust.
    #[schema(advanced, section = "tls", title = "Trust only this file")]
    pub ca_file: Option<String>,
    /// `system` or `git`.
    #[schema(advanced, section = "tls", title = "Git's TLS library")]
    pub git_tls: Option<String>,
}

/// No proxy at all.
#[derive(Provider)]
#[provider(NetworkRoute, id = "direct", config = DirectSettings, form = DirectSettings)]
pub struct DirectRoute {
    settings: DirectSettings,
}

impl From<DirectSettings> for DirectRoute {
    fn from(settings: DirectSettings) -> Self {
        Self { settings }
    }
}

impl DirectRoute {
    fn reach(&self, base: Option<&str>) -> Reach {
        let mut reach = Reach::direct();
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

impl NetworkRoute for DirectRoute {
    fn describe(&self) -> Result<RouteInfo, ProviderError> {
        let mut info = RouteInfo::new("Direct");
        info.description =
            Some("No proxy: every connection goes straight to its destination.".to_owned());
        Ok(info)
    }

    fn check(&self, request: OpenRequest) -> Result<Checked, ProviderError> {
        Ok(judge(&self.reach(request.base_dir.as_deref()), Vec::new()))
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn RouteLease>, ProviderError> {
        let base = request.base_dir.clone();
        let settings = self.settings.clone();
        Ok(Lease::new(
            move || DirectRoute::from(settings.clone()).reach(base.as_deref()),
            LeaseStatus::new("with no proxy"),
            (),
        )
        .into_object())
    }
}
