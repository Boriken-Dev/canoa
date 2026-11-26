//! What the built-in routes share: where a path in a route's settings is made from, the TLS keys every route may carry, and the
//! lease of a route that is only a [`Reach`].

use canoa::network::Network;
use canoa::network_route::{Checked, LeaseStatus, LeaseVariable, Reach, RouteLease, Trust, Urls};
use coqui_acp_api::guatiao::library::ProviderError;

/// `path` as an absolute path: itself when it is one, else below `base` (canoa.cfg's folder) when the request names it.
pub(super) fn absolute(base: Option<&str>, path: &str) -> String {
    let given = std::path::Path::new(path);
    match base {
        Some(base) if !given.is_absolute() && !path.is_empty() => {
            let joined = std::path::Path::new(base).join(given);
            std::path::absolute(&joined)
                .unwrap_or(joined)
                .to_string_lossy()
                .into_owned()
        }
        _ => path.to_owned(),
    }
}

/// The TLS keys every built-in route takes: `client_certificate` and `ca_file` name a path, made absolute from `base`.
pub(super) fn tls(
    reach: &mut Reach,
    base: Option<&str>,
    client_certificate: Option<&str>,
    ca_file: Option<&str>,
    git_tls: Option<&str>,
) {
    reach.client_certificate = client_certificate.map(|value| {
        if value.eq_ignore_ascii_case("system") || value.eq_ignore_ascii_case("none") {
            value.to_owned()
        } else {
            absolute(base, value)
        }
    });
    reach.ca_file = ca_file.map(|path| absolute(base, path));
    reach.git_tls = git_tls.map(str::to_owned);
}

/// What opening `reach` would find wrong, added to `problems`: every file it names is readable and every address parses. Nothing is
/// started and no address is reached.
pub(super) fn judge(reach: &Reach, mut problems: Vec<String>) -> Checked {
    if let Err(error) = Network::from_reach(reach) {
        problems.push(error.to_string());
    }
    Checked::of(problems)
}

/// A lease that is a [`Reach`] and, for a route that runs something, what ends when the lease does.
pub(super) struct Lease<K> {
    reach: Box<dyn Fn() -> Reach + Send + Sync>,
    status: LeaseStatus,
    variables: Vec<LeaseVariable>,
    _keep: K,
}

impl<K: Send + Sync + 'static> Lease<K> {
    pub(super) fn new(
        reach: impl Fn() -> Reach + Send + Sync + 'static,
        status: LeaseStatus,
        keep: K,
    ) -> Self {
        Self {
            reach: Box::new(reach),
            status,
            variables: Vec::new(),
            _keep: keep,
        }
    }
}

impl<K: Send + Sync + 'static> RouteLease for Lease<K> {
    fn reach(&self) -> Result<Reach, ProviderError> {
        Ok((self.reach)())
    }

    fn variables(&self, _request: Urls) -> Result<Vec<LeaseVariable>, ProviderError> {
        Ok(self.variables.clone())
    }

    fn trust(&self) -> Result<Option<Trust>, ProviderError> {
        Ok(None)
    }

    fn status(&self) -> Result<LeaseStatus, ProviderError> {
        Ok(self.status.clone())
    }
}
