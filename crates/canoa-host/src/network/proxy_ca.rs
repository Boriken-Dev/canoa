//! The certificate authority a session's proxy signs with when its profile asks the proxy to end TLS for some destinations so as to
//! present the person's client certificate to them.
//!
//! An authority is made for each session, in memory, when its proxy starts, and is gone with the session: its key is never written
//! anywhere. It is limited by its own certificate (a critical name constraint) to the destinations the profile names and no other,
//! so a program that is given its certificate to trust, and anyone who gets hold of that certificate, can be shown a certificate for
//! those destinations and for nothing else. The certificate is handed to the programs of the session in a file the session's private
//! directory holds; it is never added to the operating system's store or any other the person's own programs read. Each destination
//! gets a certificate signed by the session's authority, with a key of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "clocks and the system's roots are std's"
)]

use core::net::IpAddr;
use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

/// How long the authority, and so each certificate it signs, is valid: far longer than a session.
const VALID: Duration = Duration::from_secs(365 * 24 * 3600);
/// A name no one can reach (RFC 6761), allowed in a constraint that has to name some DNS name when the profile names none, so
/// that the authority cannot vouch for any.
const NO_NAME: &str = "canoa.invalid";

/// The authority.
pub(crate) struct SessionCa {
    issuer: Issuer<'static, KeyPair>,
    certificate_pem: String,
    not_after: OffsetDateTime,
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

/// The name constraint that limits an authority to `hosts` (a name, `*.name` for the name and what is under it, or an address, as a
/// profile writes them): each name is permitted with everything under it (a DNS constraint cannot stop short of that), each address
/// alone. A kind of name the profile names none of is excluded outright, since a constraint says nothing of a kind it does not
/// mention.
fn constraints(hosts: &[String]) -> NameConstraints {
    let mut permitted = Vec::new();
    let (mut names, mut v4, mut v6) = (false, false, false);
    for host in hosts {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let host = host.strip_prefix("*.").unwrap_or(&host);
        match host.parse::<IpAddr>() {
            Ok(address) => {
                let prefix = if address.is_ipv4() { 32 } else { 128 };
                permitted.push(GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(
                    address, prefix,
                )));
                if address.is_ipv4() {
                    v4 = true;
                } else {
                    v6 = true;
                }
            }
            Err(_) => {
                permitted.push(GeneralSubtree::DnsName(host.to_owned()));
                names = true;
            }
        }
    }
    if !names {
        permitted.push(GeneralSubtree::DnsName(NO_NAME.to_owned()));
    }
    let mut excluded = Vec::new();
    if !v4 {
        excluded.push(GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix(
            [0; 4], 0,
        )));
    }
    if !v6 {
        excluded.push(GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix(
            [0; 16], 0,
        )));
    }
    NameConstraints {
        permitted_subtrees: permitted,
        excluded_subtrees: excluded,
    }
}

/// What the authority is: a name, a window of validity that begins a day ago for a clock a little behind, the right to sign
/// certificates for destinations and nothing below them, and no right to vouch for any destination but `hosts`.
fn authority(hosts: &[String]) -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "Canoa session proxy (this session)");
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(constraints(hosts));
    params.not_before = at(now() - 24 * 3600);
    params.not_after = at(now() + seconds(VALID));
    params
}

impl SessionCa {
    /// A new authority, limited to `hosts`, that exists only in this process.
    ///
    /// # Errors
    ///
    /// No host is named, or the key or certificate could not be made.
    pub(crate) fn new(hosts: &[String]) -> Result<Self, String> {
        if hosts.is_empty() {
            return Err("an authority needs the hosts it is to vouch for".to_owned());
        }
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let params = authority(hosts);
        let not_after = params.not_after;
        let certificate_pem = params
            .clone()
            .self_signed(&key)
            .map_err(|error| error.to_string())?
            .pem();
        Ok(Self {
            issuer: Issuer::new(params, key),
            certificate_pem,
            not_after,
        })
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

    /// A certificate for `host` (a name or an address) with a key made now. A host the authority is not limited to is signed all
    /// the same, and refused by whoever checks the constraint.
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
        params.not_after = self.not_after;
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

#[cfg(test)]
mod tests;
