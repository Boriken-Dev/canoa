//! Plain settings: every bound and switch with its default, as data.

use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::path::PathBuf;

/// A TLS protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TlsVersion {
    /// TLS 1.2.
    Tls12,
    /// TLS 1.3.
    Tls13,
}

impl TlsVersion {
    /// `1.2` or `1.3`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls12 => "1.2",
            Self::Tls13 => "1.3",
        }
    }
}

impl fmt::Display for TlsVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Where the session secrets of connections are written for a traffic analyser (the NSS key log
/// format that Wireshark reads).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyLogging {
    /// Nowhere (the default).
    #[default]
    Off,
    /// The file named by the `SSLKEYLOGFILE` environment variable, when it is set.
    Environment,
    /// This file, appended to. Anyone who can read it can read the traffic.
    File(PathBuf),
}

/// The settings of a client configuration source ([`Tls`](crate::Tls)).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsSettings {
    /// The oldest protocol version offered (default TLS 1.2).
    pub minimum_version: TlsVersion,
    /// The newest protocol version offered (default TLS 1.3).
    pub maximum_version: TlsVersion,
    /// Where session secrets are logged (default nowhere).
    pub key_log: KeyLogging,
    /// How many per-host configurations are kept (default [`TlsSettings::DEFAULT_CONFIG_CACHE`]);
    /// the oldest is dropped beyond it.
    pub config_cache_capacity: usize,
}

impl TlsSettings {
    /// The default for [`config_cache_capacity`](Self::config_cache_capacity).
    pub const DEFAULT_CONFIG_CACHE: usize = 256;
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            minimum_version: TlsVersion::Tls12,
            maximum_version: TlsVersion::Tls13,
            key_log: KeyLogging::Off,
            config_cache_capacity: Self::DEFAULT_CONFIG_CACHE,
        }
    }
}

/// The settings of a [`Selector`](crate::Selector).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectorSettings {
    /// How long a listing of the sources is reused (default 60 seconds).
    pub cache_ttl: Duration,
    /// Whether the choice made for a host is reused for it (default `true`).
    pub remember: bool,
    /// Whether the browsers' `AutoSelectCertificateForUrls` policies also apply (default `false`).
    pub browser_policy: bool,
    /// How many hosts' choices are remembered (default
    /// [`SelectorSettings::DEFAULT_REMEMBERED`]).
    pub remembered_capacity: usize,
}

impl SelectorSettings {
    /// The default for [`remembered_capacity`](Self::remembered_capacity).
    pub const DEFAULT_REMEMBERED: usize = 256;
}

impl Default for SelectorSettings {
    fn default() -> Self {
        Self {
            cache_ttl: Duration::from_secs(60),
            remember: true,
            browser_policy: false,
            remembered_capacity: Self::DEFAULT_REMEMBERED,
        }
    }
}

/// The settings of a [`CertificateAuthority`](crate::CertificateAuthority).
#[cfg(feature = "issue")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssueSettings {
    /// How long a CA certificate is valid (default 3650 days).
    pub ca_validity: Duration,
    /// How long a leaf certificate is valid (default 30 days).
    pub leaf_validity: Duration,
    /// How far before now a certificate's validity starts, for clients with slow clocks
    /// (default one hour).
    pub backdate: Duration,
    /// The kind of key made (default EC P-256).
    pub key_type: crate::KeyType,
    /// The size of an RSA key (default 2048 bits).
    pub rsa_bits: u32,
    /// How many CAs may sit below this CA (default 0).
    pub path_len: u8,
}

#[cfg(feature = "issue")]
impl Default for IssueSettings {
    fn default() -> Self {
        Self {
            ca_validity: Duration::from_secs(3650 * 24 * 3600),
            leaf_validity: Duration::from_secs(30 * 24 * 3600),
            backdate: Duration::from_secs(3600),
            key_type: crate::KeyType::EcP256,
            rsa_bits: 2048,
            path_len: 0,
        }
    }
}

/// The settings of a PKCS#11 source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pkcs11Settings {
    /// The modules tried in order when none is named: the first that exists is used (default:
    /// p11-kit's proxy where it is installed, then OpenSC's module, by platform).
    pub module_candidates: Vec<PathBuf>,
}

impl Pkcs11Settings {
    /// The default [`module_candidates`](Self::module_candidates) for this platform.
    pub fn default_candidates() -> Vec<PathBuf> {
        let paths: &[&str] = if cfg!(windows) {
            &[
                r"C:\Program Files\OpenSC Project\OpenSC\pkcs11\opensc-pkcs11.dll",
                r"C:\Windows\System32\opensc-pkcs11.dll",
            ]
        } else if cfg!(target_os = "macos") {
            &[
                "/Library/OpenSC/lib/opensc-pkcs11.so",
                "/opt/homebrew/lib/opensc-pkcs11.so",
                "/usr/local/lib/opensc-pkcs11.so",
            ]
        } else {
            &[
                "/usr/lib64/pkcs11/p11-kit-proxy.so",
                "/usr/lib/x86_64-linux-gnu/pkcs11/p11-kit-proxy.so",
                "/usr/lib/aarch64-linux-gnu/pkcs11/p11-kit-proxy.so",
                "/usr/lib/pkcs11/p11-kit-proxy.so",
                "/usr/lib64/p11-kit-proxy.so",
                "/usr/lib/p11-kit-proxy.so",
                "/usr/lib64/pkcs11/opensc-pkcs11.so",
                "/usr/lib/x86_64-linux-gnu/opensc-pkcs11.so",
                "/usr/lib/aarch64-linux-gnu/opensc-pkcs11.so",
                "/usr/lib/opensc-pkcs11.so",
            ]
        };
        paths.iter().map(PathBuf::from).collect()
    }
}

impl Default for Pkcs11Settings {
    fn default() -> Self {
        Self {
            module_candidates: Self::default_candidates(),
        }
    }
}
