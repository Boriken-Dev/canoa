//! The certificate authority a host's proxies sign with when a profile asks a proxy to end TLS for some destinations so as to present a
//! client certificate to them.
//!
//! The process makes an authority when a proxy first needs one and keeps it in memory for as long as it runs: its key is never
//! written anywhere. Which authority depends on what the profile names: when every pattern is a plain host, a `*.name` or an address,
//! the authority is limited by its own certificate (a critical name constraint) to exactly those, and one is made for each distinct
//! set; when any pattern is a regular expression, which no constraint can say, the process's one unconstrained authority is used.
//! Either way a certificate for a destination is made when a program first connects to it and kept, by destination, until shortly
//! before it expires. The authority's certificate is handed to the programs of a session in a file of its private directory; it is
//! never added to the operating system's store or any other the person's own programs read.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "clocks and the system's roots are std's"
)]

use core::net::IpAddr;
use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::OffsetDateTime;

/// How long the authority is valid: far longer than a process runs, since its key goes with the process.
const VALID: Duration = Duration::from_secs(5 * 365 * 24 * 3600);
/// How long a certificate for one destination is valid.
const LEAF_VALID: Duration = Duration::from_secs(30 * 24 * 3600);
/// How long before a certificate expires it is made again instead of being reused.
const LEAF_REFRESH: Duration = Duration::from_secs(24 * 3600);
/// A name no one can reach (RFC 6761), allowed in a constraint that has to name some DNS name when the profile names none, so
/// that the authority cannot vouch for any.
const NO_NAME: &str = "canoa.invalid";

/// The authority.
pub(crate) struct ProxyCa {
    issuer: Issuer<'static, KeyPair>,
    certificate_pem: String,
    not_after: OffsetDateTime,
    /// What answers a program's TLS for each destination, made when first asked.
    answers: Mutex<HashMap<String, Answer>>,
    /// How many certificates were made.
    made: Mutex<usize>,
}

/// A certificate and key for a destination, ready to answer a program's TLS, and when the certificate expires.
struct Answer {
    config: Arc<ServerConfig>,
    expires: i64,
}

impl core::fmt::Debug for ProxyCa {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProxyCa").finish_non_exhaustive()
    }
}

/// A certificate for one destination, signed by the authority, and its key.
pub(crate) struct Leaf {
    pub(crate) chain: Vec<CertificateDer<'static>>,
    pub(crate) key: PrivateKeyDer<'static>,
    /// When the certificate expires, in seconds since the Unix epoch.
    expires: i64,
}

fn moment(seconds: i64) -> OffsetDateTime {
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
/// certificates for destinations and nothing below them, and, when `hosts` names them, no right to vouch for any destination but
/// those.
fn authority(hosts: Option<&[String]>) -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(
        DnType::CommonName,
        if hosts.is_some() {
            "Canoa proxy (limited to the hosts a profile names, this run)"
        } else {
            "Canoa proxy (this run)"
        },
    );
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = hosts.map(constraints);
    params.not_before = moment(now() - 24 * 3600);
    params.not_after = moment(now() + seconds(VALID));
    params
}

/// The authorities this process has made: one for every limit that was asked for, and the unconstrained one.
static AUTHORITIES: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// An authority and what it is limited to: the hosts, or nothing.
type Held = (Option<Vec<String>>, Arc<ProxyCa>);

impl ProxyCa {
    /// This process's authority for `hosts` (names, `*.name`s and addresses, which the authority is limited to), or for every host
    /// when there are none to limit it to. It is made the first time and the same one is returned after, so every proxy that asks
    /// for the same limit signs with the same authority and every program it is given to trust trusts one certificate.
    ///
    /// # Errors
    ///
    /// The key or the certificate could not be made.
    pub(crate) fn shared(hosts: Option<&[String]>) -> Result<Arc<Self>, String> {
        let wanted = hosts.map(|hosts| {
            let mut names: Vec<String> = hosts
                .iter()
                .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
                .collect();
            names.sort();
            names.dedup();
            names
        });
        let mut held = AUTHORITIES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, ca)) = held.iter().find(|(limit, _)| *limit == wanted) {
            return Ok(Arc::clone(ca));
        }
        let ca = Arc::new(Self::new(wanted.as_deref())?);
        held.push((wanted, Arc::clone(&ca)));
        Ok(ca)
    }

    /// A new authority, limited to `hosts` when given, that exists only in this process.
    ///
    /// # Errors
    ///
    /// No host is named in a limit, or the key or certificate could not be made.
    pub(crate) fn new(hosts: Option<&[String]>) -> Result<Self, String> {
        if hosts.is_some_and(<[String]>::is_empty) {
            return Err(
                "an authority limited to hosts needs the hosts it is to vouch for".to_owned(),
            );
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
            answers: Mutex::new(HashMap::new()),
            made: Mutex::new(0),
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

    /// What answers a program's TLS for `host` (a name or an address): a certificate with a key of its own, made the first time and
    /// kept until shortly before it expires, ALPN `http/1.1` (what the destination is asked is plain HTTP/1.1 inside a TLS the network
    /// makes with no ALPN, so that is all a program is offered).
    ///
    /// # Errors
    ///
    /// The host is no name a certificate can carry, or signing failed.
    pub(crate) fn answer(&self, host: &str) -> Result<Arc<ServerConfig>, String> {
        self.answer_at(host, now())
    }

    fn answer_at(&self, host: &str, at: i64) -> Result<Arc<ServerConfig>, String> {
        let name = host.trim_end_matches('.').to_ascii_lowercase();
        let mut held = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(answer) = held.get(&name)
            && answer.expires - seconds(LEAF_REFRESH) > at
        {
            return Ok(Arc::clone(&answer.config));
        }
        let leaf = self.leaf_at(&name, at)?;
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|error| error.to_string())?
                .with_no_client_auth()
                .with_single_cert(leaf.chain, leaf.key)
                .map_err(|error| error.to_string())?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        held.insert(
            name,
            Answer {
                config: Arc::clone(&config),
                expires: leaf.expires,
            },
        );
        Ok(config)
    }

    /// How many certificates this authority has made.
    #[cfg(test)]
    pub(crate) fn made(&self) -> usize {
        *self.made.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A certificate for `host` with a key made now, whatever has been kept. A host the authority is limited away from is signed all
    /// the same, and refused by whoever checks the constraint.
    ///
    /// # Errors
    ///
    /// The host is no name a certificate can carry, or signing failed.
    #[cfg(test)]
    pub(crate) fn leaf(&self, host: &str) -> Result<Leaf, String> {
        self.leaf_at(host, now())
    }

    fn leaf_at(&self, host: &str, at: i64) -> Result<Leaf, String> {
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
        let ends = (at + seconds(LEAF_VALID)).min(self.not_after.unix_timestamp());
        params.not_before = moment(at - 24 * 3600);
        params.not_after = moment(ends);
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let certificate = params
            .signed_by(&key, &self.issuer)
            .map_err(|error| error.to_string())?;
        *self.made.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(Leaf {
            chain: vec![certificate.der().clone()],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            expires: ends,
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
