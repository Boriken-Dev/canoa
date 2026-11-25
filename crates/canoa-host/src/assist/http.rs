//! The HTTP the host lends an `sdk_provider`: bound to one address, the credential applied here and nowhere else, through the person's
//! network, redirects refused and the answer's size limited.

use alloc::sync::Arc;
use core::time::Duration;

use canoa::network::Network;
use canoa::sdk::{Auth, HttpReply, HttpRequest, LentHttp};
use coqui_acp_api::guatiao::library::ProviderError;

/// The most bytes read from one answer, when the provider does not say.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

/// Headers a provider may not set: the host applies the credential.
fn is_credential_header(name: &str, auth: &Auth) -> bool {
    name.eq_ignore_ascii_case(&auth.header)
        || ["authorization", "x-api-key", "api-key", "cookie", "host"]
            .iter()
            .any(|held| name.eq_ignore_ascii_case(held))
}

/// HTTP below `address`, with `key` applied the way `auth` says.
pub(super) struct HostHttp {
    address: String,
    auth: Auth,
    key: Option<String>,
    network: Arc<Network>,
    timeout: Duration,
}

impl core::fmt::Debug for HostHttp {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HostHttp")
            .field("address", &self.address)
            .field("key", &self.key.as_ref().map(|_| "set"))
            .finish_non_exhaustive()
    }
}

impl HostHttp {
    pub(super) fn new(
        address: &str,
        auth: Auth,
        key: Option<String>,
        network: Arc<Network>,
        timeout: Duration,
    ) -> Self {
        Self {
            address: address.trim_end_matches('/').to_owned(),
            auth,
            key,
            network,
            timeout,
        }
    }

    /// The URL of `path` below the address, or why it is refused: a path is the part after the address and names nothing else.
    fn url_of(&self, path: &str) -> Result<String, ProviderError> {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.contains("://")
            || path.contains('\\')
        {
            return Err(canoa::status::bad_value(format!(
                "{path:?} is not a path below the address"
            )));
        }
        let url = format!("{}{path}", self.address);
        let (base, joined) = (url::Url::parse(&self.address), url::Url::parse(&url));
        match (base, joined) {
            (Ok(base), Ok(joined))
                if base.scheme() == joined.scheme()
                    && base.host_str() == joined.host_str()
                    && base.port_or_known_default() == joined.port_or_known_default()
                    && joined.path().starts_with(base.path().trim_end_matches('/')) =>
            {
                Ok(url)
            }
            _ => Err(canoa::status::bad_value(format!(
                "{path:?} leaves the address it was lent"
            ))),
        }
    }
}

impl LentHttp for HostHttp {
    fn request(&self, request: HttpRequest) -> Result<HttpReply, ProviderError> {
        let url = self.url_of(&request.path)?;
        let seconds = request.timeout_secs.map_or(self.timeout, |seconds| {
            Duration::from_secs(u64::from(seconds.max(1)))
        });
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(seconds.min(self.timeout.max(Duration::from_secs(1)))))
            .build();
        let agent = self
            .network
            .agent(config)
            .map_err(|error| canoa::status::internal(format!("the network: {error}")))?;
        let failed = |error: ureq::Error| {
            canoa::status::internal(format!("{}{}: {error}", self.address, request.path))
        };
        let headers: Vec<(&str, String)> = request
            .headers
            .iter()
            .filter(|header| !is_credential_header(&header.name, &self.auth))
            .map(|header| (header.name.as_str(), header.value.clone()))
            .collect();
        let credential = self.key.as_ref().map(|key| self.auth.value(key));
        let mut response = match request.method.as_str() {
            "GET" => {
                let mut call = agent.get(&url);
                for (name, value) in &headers {
                    call = call.header(*name, value);
                }
                if let Some(value) = &credential {
                    call = call.header(self.auth.header.as_str(), value);
                }
                call.call().map_err(failed)?
            }
            "POST" => {
                let mut call = agent.post(&url);
                for (name, value) in &headers {
                    call = call.header(*name, value);
                }
                if let Some(value) = &credential {
                    call = call.header(self.auth.header.as_str(), value);
                }
                call.send(request.body.as_deref().unwrap_or(""))
                    .map_err(failed)?
            }
            other => {
                return Err(canoa::status::bad_value(format!(
                    "the method {other:?} is not GET or POST"
                )));
            }
        };
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(request.max_bytes.unwrap_or(MAX_ANSWER))
            .read_to_string()
            .map_err(|error| {
                canoa::status::internal(format!("{}{}: {error}", self.address, request.path))
            })?;
        Ok(HttpReply::new(status, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(address: &str) -> HostHttp {
        HostHttp::new(
            address,
            Auth::bearer(),
            Some("secret".to_owned()),
            Network::current(),
            Duration::from_secs(5),
        )
    }

    #[test]
    fn a_path_stays_below_the_address_it_was_lent() {
        let http = http("https://api.example.test/v1/");
        assert_eq!(
            http.url_of("/models").expect("a path"),
            "https://api.example.test/v1/models"
        );
        for refused in ["https://evil.test/x", "//evil.test/x", "models", "/a\\b"] {
            assert!(http.url_of(refused).is_err(), "{refused}");
        }
        assert!(
            http.url_of("@evil.test/x").is_err(),
            "a path that does not start with a slash is not joined to the host"
        );
    }

    #[test]
    fn the_credential_headers_are_the_hosts_alone() {
        let auth = Auth::header("x-key");
        for name in ["X-Key", "Authorization", "cookie", "Host"] {
            assert!(is_credential_header(name, &auth), "{name}");
        }
        assert!(!is_credential_header("content-type", &auth));
    }

    #[test]
    fn a_debug_print_never_shows_the_key() {
        assert!(!format!("{:?}", http("http://h")).contains("secret"));
    }
}
