//! `coqui-access-passkey` as a library a host loads: the Guatiao entry point and the config that
//! builds the provider.
//!
//! The library offers four kinds, `Authenticator`, `Directory`, `Accounts` and `Challenger`, and a
//! host builds one instance per kind. What a sign-in needs to carry from one instance to another
//! (the account, the challenge waiting for its answer) therefore lives in files, never in memory:
//! the registrations waiting are kept beside `source` as `<path>.pending` and the challenges as
//! `<path>.ceremonies`, unless `pending` and `ceremonies` name other sources.
//!
//! ```json
//! { "source": { "file": { "path": "passkeys.toml", "format": "toml" } },
//!   "rp_id": "portal.example", "rp_name": "Portal",
//!   "origins": ["https://portal.example"],
//!   "user_verification": "required", "resident_key": "required", "attestation": "none",
//!   "algorithms": ["es256", "es384", "rs256"], "reject_synced": false,
//!   "enrol_seconds": 600, "ceremony_seconds": 300,
//!   "max_credentials": 16, "ceremonies_per_source": 8, "ceremonies_total": 1024,
//!   "counter_regression": "disable" }
//! ```
//!
//! `rp_id` and `origins` are required. A file source may name `max_bytes`; absent, each file's
//! bound is sized from `max_records`, `max_credentials` and `ceremonies_total` and the
//! `record_limits` of a name and a label, as large as they allow (at least 1 MiB). With
//! `field_bytes`, `display_name_bytes` or `max_credentials` unbounded there is no largest record,
//! and the combination is refused unless every file source names its `max_bytes`.
//! `attestation_ca_files` are PEM files read once when the provider is built. The files hold
//! public keys and challenges, not secrets, and are written with the owner's permissions only.

mod format;

use std::sync::Arc;

use coqui_access::guatiao::library::ProviderError;
use coqui_access::guatiao::{FromWalk, Provider, Walk};
use coqui_access::{
    AccountInfo, AccountPut, Accounts, Attempt, Authenticator, Challenge, ChallengeRequest,
    Challenger, Description, Directory, Limit, Lookup, Person, PutOutcome, Verdict, Waiting, error,
};
use coqui_access_kit::file::Source;
use coqui_access_kit::{Keyed, MemoryTable, RecordLimits, Table};
use coqui_access_passkey::{
    Algorithm, Attestation, Ceremony, DEFAULT_CEREMONIES_PER_SOURCE, DEFAULT_CEREMONIES_TOTAL,
    DEFAULT_CEREMONY_SECONDS, DEFAULT_ENROL_SECONDS, DEFAULT_MAX_CREDENTIALS, DEFAULT_MAX_RECORDS,
    DEFAULT_MAX_RESPONSE_BYTES, OnRegression, PasskeyAccount, Passkeys, PendingRegistration,
    ResidentKey, UserVerification, default_ceremony_limits,
};

use coqui_builder::{Describe, Partial};
use guatiao_builder::Described;

pub use format::Format;

/// Guatiao configuration for the passkey provider.
#[derive(Debug, Clone, Describe)]
pub struct PasskeyConfig {
    /// Where the accounts and their passkeys are kept; memory when absent, which a host that
    /// builds one instance per kind cannot use: the instances would share nothing.
    pub source: Option<Source<Format>>,
    /// Where registrations waiting for the browser's answer are kept; a file beside `source`
    /// (`<path>.pending`) when that is a file, else memory.
    pub pending: Option<Source<Format>>,
    /// Where sign-in challenges are kept; a file beside `source` (`<path>.ceremonies`) when that
    /// is a file, else memory.
    pub ceremonies: Option<Source<Format>>,
    /// The relying party id: the registrable domain a passkey is bound to (`portal.example`).
    pub rp_id: String,
    /// What the browser shows of the relying party; the `rp_id` when absent.
    pub rp_name: Option<String>,
    /// The origins a passkey may be used from (`https://portal.example`): a scheme, a host and
    /// optionally a port, the host being the `rp_id` or under it. At least one.
    #[map(default)]
    pub origins: Vec<String>,
    /// Whether a subdomain of an origin's host is an origin too.
    #[map(default)]
    pub allow_subdomains: bool,
    /// Whether any port of an origin's host is an origin too.
    #[map(default)]
    pub allow_any_port: bool,
    /// Whether the person must be verified: `required` (a passkey alone is then two factors),
    /// `preferred` or `discouraged`.
    #[map(default)]
    pub user_verification: UserVerification,
    /// Whether a registration asks for a discoverable credential: `required` (sign in with no
    /// name typed) or `discouraged`.
    #[map(default)]
    pub resident_key: ResidentKey,
    /// What a registration asks the authenticator to say about itself: `none`, `indirect` or
    /// `direct`.
    #[map(default)]
    pub attestation: Attestation,
    /// PEM files of the certificate authorities a registration's attestation must chain to; with
    /// any, a registration that does not fails. Needs an `attestation` other than `none`.
    #[map(default)]
    pub attestation_ca_files: Vec<String>,
    /// The algorithms a registration offers, in order of preference: `es256`, `es384`, `rs256`.
    /// `eddsa` is a word and is refused: the verifier does not check EdDSA signatures.
    #[map(default = Algorithm::DEFAULT.to_vec())]
    pub algorithms: Vec<Algorithm>,
    /// Whether a passkey the platform syncs between devices is refused, at registration and at
    /// sign-in.
    #[map(default)]
    pub reject_synced: bool,
    /// Seconds a registration waits for the browser's answer.
    #[map(default = DEFAULT_ENROL_SECONDS)]
    pub enrol_seconds: u32,
    /// Seconds a sign-in challenge lives, and the timeout the browser is told.
    #[map(default = DEFAULT_CEREMONY_SECONDS)]
    pub ceremony_seconds: u32,
    /// The most accounts, and the most registrations waiting, the tables may hold.
    #[map(default = DEFAULT_MAX_RECORDS as u32)]
    pub max_records: u32,
    /// The most passkeys one account may hold; a number or `none`.
    #[map(default = DEFAULT_MAX_CREDENTIALS)]
    pub max_credentials: Limit,
    /// The largest answer from a browser, in bytes; a number or `none`.
    #[map(default = DEFAULT_MAX_RESPONSE_BYTES)]
    pub max_response_bytes: Limit,
    /// Challenges one source (an IPv4 address, or an IPv6 /64) may have waiting for an answer.
    #[map(default = DEFAULT_CEREMONIES_PER_SOURCE as u32)]
    pub ceremonies_per_source: u32,
    /// Challenges waiting for an answer in all.
    #[map(default = DEFAULT_CEREMONIES_TOTAL as u32)]
    pub ceremonies_total: u32,
    /// What a counter that does not rise does: `disable` the passkey or only `refuse` the answer.
    #[map(default)]
    pub counter_regression: OnRegression,
    /// How large a record's fields may be: `groups`, `field_bytes`, `email_bytes`,
    /// `display_name_bytes`, each a number or `none` for no bound.
    #[map(default)]
    pub record_limits: RecordLimits,
}

impl Default for PasskeyConfig {
    /// What a configuration of the required keys only reads as. `rp_id` and `origins` are empty
    /// here, so it does not build; a test or a caller sets them.
    fn default() -> Self {
        let mut partial = Partial::new();
        partial
            .put("rp_id", String::new())
            .expect("rp_id is a key of the configuration");
        partial.build().expect("every other key has a default")
    }
}

/// What one account's record takes in a file besides its name, label and passkeys: the keys,
/// the flags and the groups, in either format and indented.
const FIXED_BYTES: u64 = 600;
/// What one passkey takes in a file besides its label: its id, the numbers and the credential as
/// JSON text (a public key and what the verifier keeps), with the quoting a format adds. An RSA
/// key with its attestation is the largest.
const BYTES_PER_CREDENTIAL: u64 = 4096;
/// What one waiting challenge takes in a file besides the credentials it lists.
const BYTES_PER_CEREMONY: u64 = 2048;

/// The bounds on the three files: room for every record as large as the limits allow, and never
/// less than the kit's default.
#[derive(Debug, Clone, Copy)]
struct FileLimits {
    accounts: u32,
    pending: u32,
    ceremonies: u32,
}

/// Sizes the files from the configuration.
///
/// # Errors
///
/// When the name or label size, or the passkeys one account may hold, is unbounded and no file
/// source names a size of its own, as there is then no largest record to size for.
fn file_limits(config: &PasskeyConfig) -> Result<FileLimits, ProviderError> {
    let named = |source: &Option<Source<Format>>| {
        source
            .as_ref()
            .and_then(|source| source.file.as_ref())
            .is_some_and(|file| file.max_bytes.is_some())
    };
    let least = coqui_access_kit::file::DEFAULT_MAX_BYTES as u64;
    let clamp = |wanted: u64| u32::try_from(wanted.max(least)).unwrap_or(u32::MAX);
    let kept = config.source.is_some() || config.pending.is_some() || config.ceremonies.is_some();
    if !kept {
        let size = clamp(0);
        return Ok(FileLimits {
            accounts: size,
            pending: size,
            ceremonies: size,
        });
    }
    let (Some(name), Some(label), Some(credentials)) = (
        config.record_limits.field_bytes.get(),
        config.record_limits.display_name_bytes.get(),
        config.max_credentials.get(),
    ) else {
        let every = |source: &Option<Source<Format>>| source.is_none() || named(source);
        return if every(&config.source) && every(&config.pending) && every(&config.ceremonies) {
            let size = clamp(0);
            Ok(FileLimits {
                accounts: size,
                pending: size,
                ceremonies: size,
            })
        } else {
            Err(error::bad_value(
                "an unbounded record_limits field_bytes or display_name_bytes, or an unbounded max_credentials, needs a max_bytes on every file source",
            ))
        };
    };
    let (name, label, credentials) = (u64::from(name), u64::from(label), u64::from(credentials));
    let account =
        FIXED_BYTES + 2 * name + 2 * label + credentials * (BYTES_PER_CREDENTIAL + 2 * label);
    let pending = FIXED_BYTES + 2 * name + 2 * label + BYTES_PER_CEREMONY + credentials * 256;
    let ceremony = BYTES_PER_CEREMONY + 2 * name + credentials * BYTES_PER_CREDENTIAL;
    Ok(FileLimits {
        accounts: clamp(u64::from(config.max_records).saturating_mul(account)),
        pending: clamp(u64::from(config.max_records).saturating_mul(pending)),
        ceremonies: clamp(u64::from(config.ceremonies_total).saturating_mul(ceremony)),
    })
}

/// The table `source` names, or one in memory.
fn open<T>(
    source: Option<&Source<Format>>,
    section: &str,
) -> Result<Arc<dyn Table<T>>, ProviderError>
where
    T: Keyed + Clone + Send + Walk + FromWalk + 'static,
{
    match source {
        Some(source) => source
            .open(section)
            .map_err(|reason| reason.writing("passkey table")),
        None => Ok(Arc::new(MemoryTable::new())),
    }
}

/// The passkey provider a host loads.
#[derive(Debug, Provider)]
#[provider(
    Authenticator,
    Directory,
    Accounts,
    Challenger,
    id = "coqui_access_passkey",
    name = "Passkeys",
    config = Described<PasskeyConfig>,
    crate = ::coqui_access::guatiao
)]
pub struct PasskeyProvider(Passkeys);

impl TryFrom<PasskeyConfig> for PasskeyProvider {
    type Error = ProviderError;

    fn try_from(config: PasskeyConfig) -> Result<Self, Self::Error> {
        let sizes = file_limits(&config)?;
        let sized = |source: &Source<Format>, size: u32| source.clone().max_bytes_unless_set(size);
        let sibling = |own: &Option<Source<Format>>, suffix: &str, size: u32| {
            own.as_ref().map(|source| sized(source, size)).or_else(|| {
                config
                    .source
                    .as_ref()
                    .map(|source| sized(&source.sibling(suffix), size))
            })
        };
        let pending = sibling(&config.pending, "pending", sizes.pending);
        let ceremonies = sibling(&config.ceremonies, "ceremonies", sizes.ceremonies);
        let source = config
            .source
            .as_ref()
            .map(|source| sized(source, sizes.accounts));
        let mut limits = default_ceremony_limits();
        limits.per_source = config.ceremonies_per_source as usize;
        limits.total = config.ceremonies_total as usize;
        let mut builder = Passkeys::builder()
            .table(open::<PasskeyAccount>(source.as_ref(), "accounts")?)
            .pending(open::<PendingRegistration>(pending.as_ref(), "pending")?)
            .ceremonies(open::<Ceremony>(ceremonies.as_ref(), "ceremonies")?)
            .rp_id(config.rp_id)
            .allow_subdomains(config.allow_subdomains)
            .allow_any_port(config.allow_any_port)
            .user_verification(config.user_verification)
            .resident_key(config.resident_key)
            .attestation(config.attestation)
            .algorithms(config.algorithms)
            .reject_synced(config.reject_synced)
            .enrol_seconds(config.enrol_seconds)
            .ceremony_seconds(config.ceremony_seconds)
            .max_records(config.max_records as usize)
            .max_credentials(config.max_credentials)
            .max_response_bytes(config.max_response_bytes)
            .ceremony_limits(limits)
            .counter_regression(config.counter_regression)
            .record_limits(config.record_limits);
        if let Some(name) = config.rp_name {
            builder = builder.rp_name(name);
        }
        for origin in config.origins {
            builder = builder.origin(origin);
        }
        for path in &config.attestation_ca_files {
            let pem = std::fs::read(path)
                .map_err(|_| error::bad_value("an attestation authority file cannot be read"))?;
            builder = builder.attestation_ca_pem(pem);
        }
        builder
            .build()
            .map(Self)
            .map_err(|reason| reason.writing("passkey table"))
    }
}

impl TryFrom<Described<PasskeyConfig>> for PasskeyProvider {
    type Error = ProviderError;

    fn try_from(config: Described<PasskeyConfig>) -> Result<Self, Self::Error> {
        Self::try_from(config.0)
    }
}

impl PasskeyProvider {
    /// The settings that differ from their default and weaken or widen what a passkey proves.
    pub fn non_defaults(&self) -> Vec<coqui_builder::NonDefault> {
        self.0.non_defaults()
    }
}

impl Authenticator for PasskeyProvider {
    fn authenticate(&self, attempt: Attempt) -> Result<Verdict, ProviderError> {
        self.0.authenticate(attempt)
    }

    fn describe(&self) -> Result<Description, ProviderError> {
        Authenticator::describe(&self.0)
    }
}

impl Challenger for PasskeyProvider {
    fn challenge(&self, request: ChallengeRequest) -> Result<Challenge, ProviderError> {
        self.0.challenge(request)
    }
}

impl Directory for PasskeyProvider {
    fn lookup(&self, lookup: Lookup) -> Result<Option<Person>, ProviderError> {
        self.0.lookup(lookup)
    }

    fn describe(&self) -> Result<Description, ProviderError> {
        Directory::describe(&self.0)
    }
}

impl Accounts for PasskeyProvider {
    fn accounts(&self) -> Result<Vec<AccountInfo>, ProviderError> {
        self.0.accounts()
    }

    fn put(&self, account: AccountPut) -> Result<PutOutcome, ProviderError> {
        self.0.put(account)
    }

    fn remove(&self, name: String) -> Result<(), ProviderError> {
        self.0.remove(name)
    }

    fn waiting(&self) -> Result<Vec<Waiting>, ProviderError> {
        self.0.waiting()
    }
}

coqui_access::guatiao::providers!(PasskeyProvider);

#[cfg(test)]
mod tests {
    use super::*;

    fn config(path: &std::path::Path) -> PasskeyConfig {
        PasskeyConfig {
            source: Some(Source::file(path.to_str().unwrap(), Format::Toml)),
            rp_id: "portal.example".to_owned(),
            origins: vec!["https://portal.example".to_owned()],
            ..PasskeyConfig::default()
        }
    }

    #[test]
    fn a_file_source_keeps_registrations_and_challenges_beside_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("passkeys.toml");
        let provider = PasskeyProvider::try_from(config(&path)).unwrap();
        provider
            .put(AccountPut {
                name: "alice".to_owned(),
                ..AccountPut::default()
            })
            .unwrap();
        let challenge = provider.challenge(ChallengeRequest::default()).unwrap();
        let pending =
            std::fs::read_to_string(directory.path().join("passkeys.toml.pending")).unwrap();
        assert!(pending.contains("[[pending]]"), "{pending}");
        let ceremonies =
            std::fs::read_to_string(directory.path().join("passkeys.toml.ceremonies")).unwrap();
        assert!(ceremonies.contains("[[ceremonies]]"), "{ceremonies}");
        assert!(ceremonies.contains(&challenge.challenge));
        // A second provider over the same files sees the registration waiting.
        let other = PasskeyProvider::try_from(config(&path)).unwrap();
        assert_eq!(other.waiting().unwrap().len(), 1);
    }

    #[test]
    fn what_a_passkey_could_not_do_without_is_refused_by_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("passkeys.toml");
        for bad in [
            PasskeyConfig {
                origins: Vec::new(),
                ..config(&path)
            },
            PasskeyConfig {
                origins: vec!["https://portal.example/login".to_owned()],
                ..config(&path)
            },
            PasskeyConfig {
                algorithms: vec![Algorithm::Eddsa],
                ..config(&path)
            },
            PasskeyConfig {
                attestation_ca_files: vec!["no-such-file.pem".to_owned()],
                attestation: Attestation::Direct,
                ..config(&path)
            },
            PasskeyConfig::default(),
        ] {
            assert!(PasskeyProvider::try_from(bad).is_err());
        }
    }

    #[test]
    fn the_files_are_sized_from_the_limits_and_an_unbounded_one_needs_a_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("passkeys.toml");
        let base = config(&path);
        let sized = |records: u32, credentials: Limit| {
            file_limits(&PasskeyConfig {
                max_records: records,
                max_credentials: credentials,
                ..base.clone()
            })
            .unwrap()
            .accounts
        };
        let few = sized(10, Limit::at(16));
        assert_eq!(few, 1024 * 1024, "never less than the kit's default");
        let many = sized(1_000, Limit::at(16));
        assert!(many > few);
        assert!(sized(1_000, Limit::at(64)) > many);
        let unbounded = PasskeyConfig {
            max_credentials: Limit::UNBOUNDED,
            ..base.clone()
        };
        assert!(file_limits(&unbounded).is_err());
        let mut named = unbounded;
        named.source = Some(Source::file(path.to_str().unwrap(), Format::Toml).max_bytes(4096));
        assert!(file_limits(&named).is_ok());
        // Memory has no file to size.
        let memory = PasskeyConfig {
            max_credentials: Limit::UNBOUNDED,
            source: None,
            ..base
        };
        assert!(PasskeyProvider::try_from(memory).is_ok());
    }
}
