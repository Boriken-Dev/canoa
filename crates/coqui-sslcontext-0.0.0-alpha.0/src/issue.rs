//! A small certificate authority for tests, local servers and TLS-terminating proxies.
//!
//! Meant for local use. Restrict a CA with [`CertificateAuthorityBuilder::permitted`] (X.509 name
//! constraints) so that, if its key leaks, it cannot vouch for other sites.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::net::IpAddr;
use core::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha1::Digest;
use time::OffsetDateTime;
use zeroize::Zeroizing;

use crate::error::{Error, Reason};
use crate::identity::{CertificateInfo, Identity, KeyType};
use crate::password::KeyPassword;
use crate::server::{ClientAuth, ServerTls, ServerTlsBuilder};
use crate::settings::IssueSettings;

fn credential(what: &'static str) -> Error {
    Error::Credential(Reason::new(what))
}

fn rcgen_error(_error: rcgen::Error) -> Error {
    credential("a certificate could not be made from these values")
}

/// A new key of `key_type` as rcgen holds it.
fn generate_key(key_type: KeyType, rsa_bits: u32) -> Result<KeyPair, Error> {
    let algorithm = match key_type {
        KeyType::EcP256 => &rcgen::PKCS_ECDSA_P256_SHA256,
        KeyType::EcP384 => &rcgen::PKCS_ECDSA_P384_SHA384,
        KeyType::Ed25519 => &rcgen::PKCS_ED25519,
        KeyType::Rsa => {
            use rsa::pkcs8::EncodePrivateKey;
            let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, rsa_bits as usize)
                .map_err(|_| credential("an RSA key of this size could not be made"))?;
            let der = key
                .to_pkcs8_der()
                .map_err(|_| credential("the RSA key could not be encoded"))?;
            return KeyPair::from_pkcs8_der_and_sign_algo(
                &PrivatePkcs8KeyDer::from(der.as_bytes()),
                &rcgen::PKCS_RSA_SHA256,
            )
            .map_err(rcgen_error);
        }
        _ => {
            return Err(Error::Unsupported(Reason::new(
                "this key type cannot be issued",
            )));
        }
    };
    KeyPair::generate_for(algorithm).map_err(rcgen_error)
}

fn validity(
    settings: &IssueSettings,
    lifetime: Duration,
) -> Result<(OffsetDateTime, OffsetDateTime), Error> {
    let too_long = || credential("the validity period is too long");
    let now = OffsetDateTime::now_utc();
    let backdate = time::Duration::try_from(settings.backdate).map_err(|_| too_long())?;
    let lifetime = time::Duration::try_from(lifetime).map_err(|_| too_long())?;
    Ok((
        now.checked_sub(backdate).ok_or_else(too_long)?,
        now.checked_add(lifetime).ok_or_else(too_long)?,
    ))
}

/// The name constraint a permitted entry stands for: a DNS name (`corp.example`, `.corp.example`
/// and `*.corp.example` all cover the name and its subdomains) or an IP network.
fn subtree(entry: &str) -> Result<GeneralSubtree, Error> {
    let text = entry.trim().to_lowercase();
    let text = text.trim_end_matches('.');
    if let Some((address, prefix)) = text.split_once('/')
        && let (Ok(address), Ok(prefix)) = (address.parse::<IpAddr>(), prefix.parse::<u8>())
    {
        let limit = if address.is_ipv4() { 32 } else { 128 };
        if prefix <= limit {
            return Ok(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(
                address, prefix,
            )));
        }
    } else if let Ok(address) = text.parse::<IpAddr>() {
        let prefix = if address.is_ipv4() { 32 } else { 128 };
        return Ok(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(
            address, prefix,
        )));
    }
    let domain = text
        .strip_prefix("*.")
        .unwrap_or(text)
        .trim_start_matches('.');
    if domain.is_empty() || domain.contains('*') || domain.contains('/') || !domain.is_ascii() {
        return Err(credential(
            "a permitted name cannot be expressed as a name constraint",
        ));
    }
    Ok(GeneralSubtree::DnsName(domain.to_string()))
}

/// Builds a [`CertificateAuthority`].
#[derive(Debug, Clone)]
#[must_use]
pub struct CertificateAuthorityBuilder {
    common_name: String,
    permitted: Vec<String>,
    settings: IssueSettings,
}

impl CertificateAuthorityBuilder {
    /// What the CA may vouch for, as critical name constraints: DNS names (`corp.example` covers
    /// its subdomains; `*.corp.example` and `.corp.example` mean the same) and IP networks
    /// (`10.0.0.0/8`). Empty means unconstrained.
    pub fn permitted<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.permitted = names.into_iter().map(Into::into).collect();
        self
    }

    /// Validity, key type and the other plain settings.
    pub fn settings(mut self, settings: IssueSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Makes the CA: a self-signed certificate and a new key.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for a permitted name that cannot be a name constraint;
    /// [`Error::Unsupported`] for a key type that cannot be issued.
    pub fn create(self) -> Result<CertificateAuthority, Error> {
        let settings = self.settings;
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, self.common_name.as_str());
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(settings.path_len));
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        (params.not_before, params.not_after) = validity(&settings, settings.ca_validity)?;
        if !self.permitted.is_empty() {
            let permitted: Vec<GeneralSubtree> = self
                .permitted
                .iter()
                .map(|entry| subtree(entry))
                .collect::<Result<_, _>>()?;
            // A constraint limits only the kinds of name it lists, so a kind with no permitted
            // entry is excluded whole: all IP addresses, or every DNS name.
            let mut excluded = Vec::new();
            if !permitted
                .iter()
                .any(|entry| matches!(entry, GeneralSubtree::IpAddress(_)))
            {
                for address in [IpAddr::from([0u8; 4]), IpAddr::from([0u8; 16])] {
                    excluded.push(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(
                        address, 0,
                    )));
                }
            }
            if !permitted
                .iter()
                .any(|entry| matches!(entry, GeneralSubtree::DnsName(_)))
            {
                excluded.push(GeneralSubtree::DnsName(String::new()));
            }
            params.name_constraints = Some(NameConstraints {
                permitted_subtrees: permitted,
                excluded_subtrees: excluded,
            });
        }
        let key = generate_key(settings.key_type, settings.rsa_bits)?;
        let certificate = params.self_signed(&key).map_err(rcgen_error)?;
        let (der, pem) = (certificate.der().clone(), certificate.pem());
        let issuer = Issuer::from_ca_cert_der(&der, key).map_err(rcgen_error)?;
        CertificateAuthority::from_issuer(issuer, der, pem, settings)
    }
}

/// A certificate authority that holds its key in memory and issues leaf certificates.
pub struct CertificateAuthority {
    issuer: Issuer<'static, KeyPair>,
    der: CertificateDer<'static>,
    pem: String,
    info: CertificateInfo,
    settings: IssueSettings,
}

impl fmt::Debug for CertificateAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CertificateAuthority")
            .field("subject", &self.info.subject())
            .finish_non_exhaustive()
    }
}

impl CertificateAuthority {
    /// Starts a new CA named `common_name`.
    pub fn builder(common_name: impl Into<String>) -> CertificateAuthorityBuilder {
        CertificateAuthorityBuilder {
            common_name: common_name.into(),
            permitted: Vec::new(),
            settings: IssueSettings::default(),
        }
    }

    fn from_issuer(
        issuer: Issuer<'static, KeyPair>,
        der: CertificateDer<'static>,
        pem: String,
        settings: IssueSettings,
    ) -> Result<Self, Error> {
        let info = CertificateInfo::from_chain(core::slice::from_ref(&der))?;
        Ok(Self {
            issuer,
            der,
            pem,
            info,
            settings,
        })
    }

    /// A CA from its PEM certificate and key (an encrypted PKCS#8 key is opened with `password`).
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for text that is not a CA certificate with its key, or a missing or
    /// wrong password.
    pub fn from_pem(certificate: &[u8], key: &[u8], password: &KeyPassword) -> Result<Self, Error> {
        let key = crate::load::pem_private_key(key, password)?;
        let key = KeyPair::try_from(&key).map_err(|_| credential("the CA key is not usable"))?;
        let text = core::str::from_utf8(certificate)
            .map_err(|_| credential("the certificate text is not PEM"))?;
        let issuer = Issuer::from_ca_cert_pem(text, key)
            .map_err(|_| credential("the CA certificate does not match its key or is not a CA"))?;
        let der = CertificateDer::from_pem_slice(certificate)
            .map_err(|_| credential("the certificate text is not PEM"))?;
        Self::from_issuer(issuer, der, text.to_string(), IssueSettings::default())
    }

    /// The same CA issuing under other settings (leaf validity, key type, backdating).
    #[must_use]
    pub fn with_settings(mut self, settings: IssueSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The CA certificate, DER.
    pub fn certificate_der(&self) -> &CertificateDer<'static> {
        &self.der
    }

    /// The CA certificate, PEM.
    pub fn certificate_pem(&self) -> String {
        self.pem.clone()
    }

    /// What the CA certificate says about itself.
    pub fn info(&self) -> &CertificateInfo {
        &self.info
    }

    /// The CA key as PKCS#8 PEM, encrypted (PBES2, AES-256-CBC) when `password` gives one.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a password without the `encrypted-keys` feature.
    pub fn private_key_pem(&self, password: &KeyPassword) -> Result<Zeroizing<String>, Error> {
        match password.resolve()? {
            None => Ok(Zeroizing::new(self.issuer.key().serialize_pem())),
            Some(password) => encrypt_pem(
                &Zeroizing::new(self.issuer.key().serialize_der()),
                password.expose(),
            ),
        }
    }

    /// A leaf certificate for `names` (DNS names and IP addresses) under the default settings.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for no names or a name a certificate cannot hold.
    pub fn issue<I, S>(&self, names: I) -> Result<IssuedCertificate, Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.leaf(names).issue()
    }

    /// Starts a leaf certificate for `names`, to set its validity, key type or common name.
    pub fn leaf<I, S>(&self, names: I) -> LeafBuilder<'_>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        LeafBuilder {
            authority: self,
            names: names.into_iter().map(Into::into).collect(),
            common_name: None,
            validity: self.settings.leaf_validity,
            key_type: self.settings.key_type,
        }
    }

    /// A server configuration source presenting a fresh certificate for `names`; set the client
    /// authentication, ALPN and versions on the builder.
    ///
    /// # Errors
    ///
    /// As [`issue`](Self::issue).
    pub fn server_tls<I, S>(&self, names: I) -> Result<ServerTlsBuilder, Error>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Ok(ServerTls::builder(self.issue(names)?.identity()?))
    }
}

#[cfg(feature = "encrypted-keys")]
fn encrypt_pem(der: &[u8], password: &str) -> Result<Zeroizing<String>, Error> {
    use pkcs8::der::Decode;
    let info = pkcs8::PrivateKeyInfoRef::from_der(der)
        .map_err(|_| credential("the key does not decode"))?;
    let document = info
        .encrypt(password)
        .map_err(|_| credential("the key could not be encrypted"))?;
    document
        .to_pem("ENCRYPTED PRIVATE KEY", pkcs8::der::pem::LineEnding::LF)
        .map_err(|_| credential("the key could not be written"))
}

#[cfg(not(feature = "encrypted-keys"))]
fn encrypt_pem(_der: &[u8], _password: &str) -> Result<Zeroizing<String>, Error> {
    Err(Error::Unsupported(Reason::new(
        "an encrypted key needs the `encrypted-keys` feature",
    )))
}

/// Builds a leaf certificate.
#[derive(Debug)]
#[must_use]
pub struct LeafBuilder<'a> {
    authority: &'a CertificateAuthority,
    names: Vec<String>,
    common_name: Option<String>,
    validity: Duration,
    key_type: KeyType,
}

impl LeafBuilder<'_> {
    /// The subject common name (default: the first name).
    pub fn common_name(mut self, common_name: impl Into<String>) -> Self {
        self.common_name = Some(common_name.into());
        self
    }

    /// How long the certificate is valid (default [`IssueSettings::leaf_validity`]).
    pub fn validity(mut self, validity: Duration) -> Self {
        self.validity = validity;
        self
    }

    /// The kind of key (default [`IssueSettings::key_type`]).
    pub fn key_type(mut self, key_type: KeyType) -> Self {
        self.key_type = key_type;
        self
    }

    /// Makes the certificate and its key. The leaf carries the names as subject alternative
    /// names, server and client authentication, and key identifiers, so strict verification
    /// accepts it.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for no names or a name a certificate cannot hold.
    pub fn issue(self) -> Result<IssuedCertificate, Error> {
        let first = self
            .names
            .first()
            .ok_or_else(|| credential("a certificate needs at least one name"))?;
        let settings = &self.authority.settings;
        let mut params = CertificateParams::default();
        params.subject_alt_names = self
            .names
            .iter()
            .map(|name| match name.parse::<IpAddr>() {
                Ok(address) => Ok(SanType::IpAddress(address)),
                Err(_) => name
                    .as_str()
                    .try_into()
                    .map(SanType::DnsName)
                    .map_err(|_| credential("a name cannot be a DNS name")),
            })
            .collect::<Result<_, _>>()?;
        params.distinguished_name.push(
            DnType::CommonName,
            self.common_name.as_deref().unwrap_or(first),
        );
        params.is_ca = IsCa::NoCa;
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        (params.not_before, params.not_after) = validity(settings, self.validity)?;
        let key = generate_key(self.key_type, settings.rsa_bits)?;
        // rcgen adds a subject key identifier to CA certificates only: a leaf gets the SHA-1 of its
        // public key (RFC 5280, method 1), as an OCTET STRING inside the extension value.
        let digest = sha1::Sha1::digest(key.public_key_raw());
        let mut identifier = vec![0x04, 0x14];
        identifier.extend_from_slice(&digest);
        params
            .custom_extensions
            .push(rcgen::CustomExtension::from_oid_content(
                &[2, 5, 29, 14],
                identifier,
            ));
        let certificate = params
            .signed_by(&key, &self.authority.issuer)
            .map_err(rcgen_error)?;
        Ok(IssuedCertificate {
            certificate: certificate.der().clone(),
            certificate_pem: certificate.pem(),
            key_der: Zeroizing::new(key.serialize_der()),
            key_pem: Zeroizing::new(key.serialize_pem()),
            chain_pem: self.authority.certificate_pem(),
            chain_der: self.authority.der.clone(),
        })
    }
}

/// A leaf certificate and its key, with the CA certificate that issued it.
#[derive(Clone)]
pub struct IssuedCertificate {
    certificate: CertificateDer<'static>,
    certificate_pem: String,
    key_der: Zeroizing<Vec<u8>>,
    key_pem: Zeroizing<String>,
    chain_pem: String,
    chain_der: CertificateDer<'static>,
}

impl fmt::Debug for IssuedCertificate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedCertificate")
            .finish_non_exhaustive()
    }
}

impl IssuedCertificate {
    /// The leaf certificate, PEM.
    pub fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }

    /// The private key, PKCS#8 PEM, unencrypted.
    pub fn key_pem(&self) -> &str {
        &self.key_pem
    }

    /// The issuing CA's certificate, PEM.
    pub fn chain_pem(&self) -> &str {
        &self.chain_pem
    }

    /// The leaf certificate, DER.
    pub fn certificate_der(&self) -> &CertificateDer<'static> {
        &self.certificate
    }

    /// What the leaf certificate says about itself.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the certificate does not decode (it always does).
    pub fn info(&self) -> Result<CertificateInfo, Error> {
        CertificateInfo::from_chain(&[self.certificate.clone(), self.chain_der.clone()])
    }

    /// The certificate, its chain and its key as an [`Identity`].
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] when the key cannot sign.
    pub fn identity(&self) -> Result<Identity, Error> {
        Identity::from_der(
            [self.certificate.as_ref(), self.chain_der.as_ref()],
            &self.key_der,
        )
    }

    /// A server configuration presenting this certificate, with the given client authentication.
    ///
    /// # Errors
    ///
    /// As [`ServerTlsBuilder::build`].
    pub fn server_config(
        &self,
        client_auth: ClientAuth,
    ) -> Result<alloc::sync::Arc<rustls::ServerConfig>, Error> {
        Ok(ServerTls::builder(self.identity()?)
            .client_auth(client_auth)
            .build()?
            .server_config())
    }

    /// The key as a rustls private key.
    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.to_vec()))
    }
}

impl From<&str> for CertificateAuthorityBuilder {
    fn from(common_name: &str) -> Self {
        CertificateAuthority::builder(common_name)
    }
}
