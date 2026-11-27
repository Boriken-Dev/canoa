//! Server configurations: an identity, optional client certificates, ALPN and versions.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use rustls::RootCertStore;
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;

use crate::client::{NonDefault, Trust, protocol_versions, trust_roots};
use crate::error::{Error, Reason};
use crate::identity::Identity;
use crate::keylog::key_log;
use crate::settings::{KeyLogging, TlsSettings, TlsVersion};

/// What a server does about client certificates. [`Trust::System`] accepts a client certificate
/// from any public CA with the client-authentication usage, so it vouches for strangers: it is
/// listed by `non_defaults()`; name the roots of your own CA instead.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ClientAuth {
    /// Asks for none (the default).
    #[default]
    None,
    /// Asks, verifies one that is sent against this trust, and lets a client with none through.
    Optional(Trust),
    /// Asks and refuses a client with none or one this trust does not vouch for.
    Required(Trust),
}

/// Builds a [`ServerTls`].
#[derive(Debug, Clone)]
#[must_use]
pub struct ServerTlsBuilder {
    identity: Identity,
    client_auth: ClientAuth,
    alpn: Vec<Vec<u8>>,
    settings: TlsSettings,
}

impl ServerTlsBuilder {
    /// What to do about client certificates (default: ask for none).
    pub fn client_auth(mut self, client_auth: ClientAuth) -> Self {
        self.client_auth = client_auth;
        self
    }

    /// The application protocols accepted, most preferred first (default: none).
    pub fn alpn<I, P>(mut self, protocols: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<Vec<u8>>,
    {
        self.alpn = protocols.into_iter().map(Into::into).collect();
        self
    }

    /// The oldest protocol version accepted (default TLS 1.2).
    pub fn minimum_version(mut self, version: TlsVersion) -> Self {
        self.settings.minimum_version = version;
        self
    }

    /// The newest protocol version accepted (default TLS 1.3).
    pub fn maximum_version(mut self, version: TlsVersion) -> Self {
        self.settings.maximum_version = version;
        self
    }

    /// Where session secrets are logged for a traffic analyser (default nowhere).
    pub fn key_log(mut self, logging: KeyLogging) -> Self {
        self.settings.key_log = logging;
        self
    }

    /// All the plain settings at once.
    pub fn settings(mut self, settings: TlsSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The settings that are not the default, as data: logging session secrets, and client
    /// certificates verified against the system's roots.
    pub fn non_defaults(&self) -> Vec<NonDefault> {
        non_defaults(&self.settings, &self.client_auth)
    }

    /// The server configuration source.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the versions leave none, a client trust holds no root certificate or is
    /// [`Trust::Disabled`] (a server either verifies client certificates or asks for none);
    /// [`Error::Io`] when a bundle or key log file cannot be read.
    pub fn build(self) -> Result<ServerTls, Error> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let versions = protocol_versions(&self.settings)?;
        let builder = ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&versions)
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
        let builder = match &self.client_auth {
            ClientAuth::None => builder.with_no_client_auth(),
            ClientAuth::Optional(trust) => {
                builder.with_client_cert_verifier(client_verifier(trust, &provider, true)?)
            }
            ClientAuth::Required(trust) => {
                builder.with_client_cert_verifier(client_verifier(trust, &provider, false)?)
            }
        };
        let mut config = builder.with_cert_resolver(Arc::new(OneServerIdentity {
            key: self.identity.certified_key(),
        }));
        config.alpn_protocols.clone_from(&self.alpn);
        if let Some(log) = key_log(&self.settings.key_log)? {
            config.key_log = log;
        }
        Ok(ServerTls {
            config: Arc::new(config),
            client_auth: self.client_auth,
            settings: self.settings,
        })
    }
}

fn client_verifier(
    trust: &Trust,
    provider: &Arc<CryptoProvider>,
    optional: bool,
) -> Result<Arc<dyn rustls::server::danger::ClientCertVerifier>, Error> {
    if matches!(trust, Trust::Disabled) {
        return Err(Error::Tls(Reason::new(
            "client certificates cannot be required and left unverified; ask for none instead",
        )));
    }
    let roots: RootCertStore = trust_roots(trust)?;
    let verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(roots), Arc::clone(provider));
    let verifier = if optional {
        verifier.allow_unauthenticated()
    } else {
        verifier
    };
    verifier
        .build()
        .map_err(|error| Error::Tls(Reason::new(error.to_string())))
}

/// Answers every handshake with one identity, whose key may live in an OS keystore.
struct OneServerIdentity {
    key: Arc<CertifiedKey>,
}

impl fmt::Debug for OneServerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OneServerIdentity")
            .finish_non_exhaustive()
    }
}

impl ResolvesServerCert for OneServerIdentity {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.key))
    }
}

/// A source of `rustls` server configurations: one identity and a policy for client certificates.
pub struct ServerTls {
    config: Arc<ServerConfig>,
    client_auth: ClientAuth,
    settings: TlsSettings,
}

impl fmt::Debug for ServerTls {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerTls")
            .field("client_auth", &self.client_auth)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl ServerTls {
    /// Starts a server configuration serving `identity` (its key may be held by a keystore).
    pub fn builder(identity: Identity) -> ServerTlsBuilder {
        ServerTlsBuilder {
            identity,
            client_auth: ClientAuth::None,
            alpn: Vec::new(),
            settings: TlsSettings::default(),
        }
    }

    /// The configuration to hand to a TLS acceptor.
    pub fn server_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.config)
    }

    /// The plain settings this source was built with.
    pub fn settings(&self) -> &TlsSettings {
        &self.settings
    }

    /// The settings that are not the default, as data: logging session secrets, and client
    /// certificates verified against the system's roots.
    pub fn non_defaults(&self) -> Vec<NonDefault> {
        non_defaults(&self.settings, &self.client_auth)
    }
}

fn non_defaults(settings: &TlsSettings, client_auth: &ClientAuth) -> Vec<NonDefault> {
    let mut found = Vec::new();
    if let ClientAuth::Optional(Trust::System) | ClientAuth::Required(Trust::System) = client_auth {
        found.push(NonDefault {
            setting: "client_auth",
            value: "system roots".into(),
            default: "none, or the roots you name",
        });
    }
    if settings.key_log != KeyLogging::Off {
        found.push(NonDefault {
            setting: "key_log",
            value: "on".into(),
            default: "off",
        });
    }
    found
}
