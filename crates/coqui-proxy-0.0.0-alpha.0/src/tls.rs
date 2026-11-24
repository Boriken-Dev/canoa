//! The default TLS configuration for an `https:` proxy or PAC location: verified by the
//! operating system, no client certificate.

use alloc::sync::Arc;

use rustls::ClientConfig;

use crate::error::{Error, Reason};

/// A client configuration whose certificates the platform verifier judges.
pub(crate) fn platform_config() -> Result<Arc<ClientConfig>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
        .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| Error::Tls(Reason::new(error.to_string())))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}
