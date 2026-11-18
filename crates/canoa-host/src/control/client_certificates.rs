//! `network/client_certificates`: the client certificates this host can present, for a profile's proxy item to offer as a person
//! offers a browser's: which one each pattern of hosts is given. What it lists is what the certificate says of itself and never a
//! key.

use std::sync::Arc;

use canoa::network::Network;
use serde_json::{Value, json};

/// The certificates of the network the host presents them from.
#[derive(Clone, Default)]
pub(crate) struct ClientCertificates {
    /// The network they are listed from; the process's when `None`.
    network: Option<Arc<Network>>,
}

impl core::fmt::Debug for ClientCertificates {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClientCertificates").finish_non_exhaustive()
    }
}

impl ClientCertificates {
    /// Lists `network`'s certificates instead of the process's.
    #[cfg(test)]
    pub(crate) fn on(network: Arc<Network>) -> Self {
        Self {
            network: Some(network),
        }
    }

    /// `{"certificates": [{"thumbprint", "subject", "issuer", "not_after", "source"}]}`, the ones expiring last first: `thumbprint`
    /// is the SHA-256 of the certificate in lower-case hex, which a proxy item names a certificate by; `not_after` is seconds since
    /// the Unix epoch; `source` says where it is held (the operating system's keystore, a PKCS#11 token, or the file
    /// `[host.network] client_certificate` names).
    pub(crate) fn list(&self) -> Value {
        let network = self.network.clone().unwrap_or_else(Network::current);
        let certificates: Vec<Value> = network
            .client_identities()
            .into_iter()
            .map(|identity| {
                json!({
                    "thumbprint": identity.thumbprint,
                    "subject": identity.subject,
                    "issuer": identity.issuer,
                    "not_after": identity.not_after,
                    "source": identity.source,
                })
            })
            .collect();
        json!({ "certificates": certificates })
    }
}
