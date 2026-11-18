//! The certificate authority a session's proxy signs with when its profile asks the proxy to end TLS for some destinations so as to
//! present the person's client certificate to them.
//!
//! One authority is kept per home, in a directory that only this user can enter, and is made again once a year. Its certificate is
//! handed to the programs of a session that asks for it, in a file the session's private directory holds; it is never added to the
//! operating system's store or any other the person's own programs read. Its key stays in the home and in this process: it is in
//! no variable, no report and no file a session's programs are given. Each session signs its own certificate for each destination,
//! with a key of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, clocks and the system's roots are std's"
)]

use core::net::IpAddr;
use core::time::Duration;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

/// What the home keeps: the authority's certificate and, after it, its key.
const FILE: &str = "ca.pem";
/// The directory in the home that holds it.
const DIRECTORY: [&str; 2] = ["network", "session-ca"];
/// How long the authority is valid.
const VALID: Duration = Duration::from_secs(2 * 365 * 24 * 3600);
/// How old the kept authority may be before another is made.
const RENEW_AFTER: Duration = Duration::from_secs(365 * 24 * 3600);
/// How long a certificate for one destination is valid: far longer than a session, short of the authority's end.
const LEAF_VALID: Duration = Duration::from_secs(90 * 24 * 3600);
/// The key's block in the file.
const KEY_BLOCK: &str = "-----BEGIN PRIVATE KEY-----";

/// The authority.
pub(crate) struct SessionCa {
    issuer: Issuer<'static, KeyPair>,
    certificate_pem: String,
}

impl core::fmt::Debug for SessionCa {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionCa").finish_non_exhaustive()
    }
}

/// A certificate for one destination, signed by the authority, and its key.
pub(crate) struct Leaf {
    pub(crate) chain: Vec<CertificateDer<'static>>,
    pub(crate) key: PrivateKeyDer<'static>,
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0)
}

fn seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

/// What the authority is: a name, a window of validity that begins a day ago for a clock a little behind, and the right to sign
/// certificates for destinations and nothing below them.
fn authority() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "Canoa session proxy (this computer)");
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = at(now() - 24 * 3600);
    params.not_after = at(now() + seconds(VALID));
    params
}

impl SessionCa {
    /// The authority of `home`, made if there is none or the one there is a year old; one that lives only in this process when the
    /// host has no home.
    ///
    /// # Errors
    ///
    /// What cannot be made or kept, in words for the report.
    pub(crate) fn open(home: Option<&Path>) -> Result<Self, String> {
        let Some(home) = home else {
            return Self::generate().map(|(ca, _)| ca);
        };
        let directory = DIRECTORY
            .iter()
            .fold(home.to_path_buf(), |dir, part| dir.join(part));
        crate::profile::hosted::private::keep(&directory).map_err(|error| {
            format!(
                "{} cannot be kept for this user alone: {error}",
                directory.display()
            )
        })?;
        let file = directory.join(FILE);
        if let Some(kept) = Self::read(&file) {
            return Ok(kept);
        }
        let (ca, text) = Self::generate()?;
        write_private(&file, text.as_bytes())
            .map_err(|error| format!("{} cannot be written: {error}", file.display()))?;
        Ok(ca)
    }

    /// The authority `file` holds, when it is whole and not yet due to be made again.
    fn read(file: &Path) -> Option<Self> {
        let young = fs::metadata(file)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|made| made.elapsed().ok())
            .is_some_and(|age| age < RENEW_AFTER);
        if !young {
            return None;
        }
        let text = fs::read_to_string(file).ok()?;
        let (certificate, key) = text.split_once(KEY_BLOCK)?;
        let key = KeyPair::from_pem(&format!("{KEY_BLOCK}{key}")).ok()?;
        Some(Self {
            issuer: Issuer::new(authority(), key),
            certificate_pem: certificate.to_owned(),
        })
    }

    /// A new authority, and the text a home keeps it as.
    fn generate() -> Result<(Self, String), String> {
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let certificate = authority()
            .self_signed(&key)
            .map_err(|error| error.to_string())?
            .pem();
        let text = format!("{certificate}{}", key.serialize_pem());
        Ok((
            Self {
                issuer: Issuer::new(authority(), key),
                certificate_pem: certificate,
            },
            text,
        ))
    }

    /// The authority's certificate, which is all a program is given of it.
    pub(crate) fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }

    /// The authority's certificate and then every root the operating system trusts: what a program that takes the place of its own
    /// roots with a file is given, so it still reaches every destination the proxy does not end.
    pub(crate) fn with_system_roots(&self) -> String {
        let mut text = self.certificate_pem.clone();
        for root in rustls_native_certs::load_native_certs().certs {
            text.push_str(&pem("CERTIFICATE", root.as_ref()));
        }
        text
    }

    /// A certificate for `host` (a name or an address) with a key made now.
    ///
    /// # Errors
    ///
    /// The host is no name a certificate can carry, or signing failed.
    pub(crate) fn leaf(&self, host: &str) -> Result<Leaf, String> {
        let mut params = CertificateParams::default();
        let named = |name: &str| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
        };
        if host.parse::<IpAddr>().is_err() && !named(host) {
            return Err(format!("{host} is not a name a certificate can carry"));
        }
        params.subject_alt_names = vec![match host.parse::<IpAddr>() {
            Ok(address) => SanType::IpAddress(address),
            Err(_) => SanType::DnsName(
                rcgen::string::Ia5String::try_from(host.to_owned())
                    .map_err(|_| format!("{host} is not a name a certificate can carry"))?,
            ),
        }];
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.use_authority_key_identifier_extension = true;
        params.not_before = at(now() - 24 * 3600);
        params.not_after = at(now() + seconds(LEAF_VALID));
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let certificate = params
            .signed_by(&key, &self.issuer)
            .map_err(|error| error.to_string())?;
        Ok(Leaf {
            chain: vec![certificate.der().clone()],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        })
    }
}

/// `der` as a PEM block of `label`.
fn pem(label: &str, der: &[u8]) -> String {
    let body = data_encoding::BASE64.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        text.push_str(&String::from_utf8_lossy(line));
        text.push('\n');
    }
    text.push_str(&format!("-----END {label}-----\n"));
    text
}

/// Writes `bytes` to `file` so that no one else can read them at any moment: into a file of its own that only this user can open,
/// then moved into place.
fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let staged = file.with_extension("new");
    let _ = fs::remove_file(&staged);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(&staged)?;
    out.write_all(bytes)?;
    out.sync_all()?;
    drop(out);
    fs::rename(&staged, file)
}

#[cfg(test)]
mod tests;
