//! `coqui-access-totp` as a library a host loads: the Guatiao entry point and the config that
//! builds the provider.
//!
//! The config names where accounts come from in `source`, a file in `toml` or `json`; the
//! enrolments waiting for a first code are kept beside it (`<path>.pending`) unless `pending`
//! names another source:
//!
//! ```json
//! { "source": { "file": { "path": "totp.toml", "format": "toml" } },
//!   "issuer": "Example", "algorithm": "sha1", "digits": 6, "period": 30, "skew": 1,
//!   "recovery_codes": 10, "enrol_seconds": 600, "max_records": 10000,
//!   "lockout_after": 5, "lockout_seconds": 30, "lockout_longest_seconds": 3600 }
//! ```
//!
//! A file source may name `max_bytes`; absent, the bound is sized from `max_records`, the
//! recovery code count and the `record_limits` of a name and a label, as large as they allow
//! (at least 1 MiB). With `field_bytes` or `display_name_bytes` unbounded there is no largest
//! record, and the combination is refused unless every file source names its `max_bytes`.
//!
//! Every key is optional. With no `source` the accounts live in memory. The file holds each
//! account's secret readable, as a time-based code needs it: it is written with the owner's
//! permissions only, and belongs where only the service can read it.

mod format;

use std::sync::Arc;

use coqui_access::guatiao::library::ProviderError;
use coqui_access::guatiao::{FromWalk, Provider, Walk};
use coqui_access::{
    AccountInfo, AccountPut, Accounts, Attempt, Authenticator, Description, PutOutcome, Verdict,
    Waiting,
};
use coqui_access_kit::file::Source;
use coqui_access_kit::{Keyed, MemoryTable, RecordLimits, Table};
use coqui_access_totp::{
    Algorithm, DEFAULT_DIGITS, DEFAULT_ENROL_SECONDS, DEFAULT_MAX_RECORDS, DEFAULT_PERIOD,
    DEFAULT_RECOVERY_CODES, DEFAULT_SKEW, Lockout, PendingTotp, TotpAccounts, TotpRecord,
};

use coqui_builder::{Describe, Partial};
use guatiao_builder::Described;

pub use format::Format;

/// Guatiao configuration for the one-time code provider.
#[derive(Debug, Clone, Describe)]
pub struct TotpConfig {
    /// Where the enrolled accounts are kept; memory when absent.
    pub source: Option<Source<Format>>,
    /// Where enrolments waiting for a first code are kept; a file beside `source`
    /// (`<path>.pending`) when that is a file, else memory.
    pub pending: Option<Source<Format>>,
    /// The name an authenticator app shows beside the account.
    pub issuer: Option<String>,
    /// The hash new secrets use: `sha1` (every app reads it), `sha256` or `sha512`.
    #[map(default)]
    pub algorithm: Algorithm,
    /// Digits of a code, 6 to 8.
    #[map(default = DEFAULT_DIGITS)]
    pub digits: u32,
    /// Seconds a code lasts.
    #[map(default = DEFAULT_PERIOD)]
    pub period: u32,
    /// Steps either side of the present one a code is accepted in, at most 10.
    #[map(default = DEFAULT_SKEW)]
    pub skew: u32,
    /// Recovery codes made at enrolment, at most 64.
    #[map(default = DEFAULT_RECOVERY_CODES)]
    pub recovery_codes: u32,
    /// Seconds an enrolment waits for its first code.
    #[map(default = DEFAULT_ENROL_SECONDS)]
    pub enrol_seconds: u32,
    /// The most accounts, and the most enrolments waiting, the tables may hold.
    #[map(default = DEFAULT_MAX_RECORDS as u32)]
    pub max_records: u32,
    /// Wrong codes in a row before an account takes no code for a while; 0 never delays,
    /// which leaves counting wrong codes to the consumer alone.
    #[map(default = Lockout::DEFAULT.after)]
    pub lockout_after: u32,
    /// The first delay, in seconds; it doubles with each further wrong code.
    #[map(default = Lockout::DEFAULT.first_seconds)]
    pub lockout_seconds: u32,
    /// The longest delay, in seconds.
    #[map(default = Lockout::DEFAULT.longest_seconds)]
    pub lockout_longest_seconds: u32,
    /// How large a record's fields may be: `groups`, `field_bytes`, `email_bytes`,
    /// `display_name_bytes`, each a number or `none` for no bound.
    #[map(default)]
    pub record_limits: RecordLimits,
}

impl Default for TotpConfig {
    /// What an empty configuration reads as: every default the schema declares.
    fn default() -> Self {
        Partial::new()
            .build()
            .expect("an empty configuration reads, every key having a default")
    }
}

/// What one account's record takes in a file besides its name, label and recovery codes: the
/// keys, the secret, the numbers and the lockout, in either format and indented.
const FIXED_BYTES: u64 = 400;
/// What one recovery code's digest takes in a file: 64 hex digits, quotes, a comma and the
/// indent.
const BYTES_PER_CODE: u64 = 80;

/// The bound on a file holding `max_records` accounts, each as large as the configured limits
/// allow (a name or label may double when quoted): room for all of them, and never less than
/// the kit's default.
///
/// # Errors
///
/// When the name or label size is unbounded and no source names a size of its own, as there
/// is then no largest record to size for.
fn file_limit(config: &TotpConfig) -> Result<u32, ProviderError> {
    let named = |source: &Option<Source<Format>>| {
        source
            .as_ref()
            .and_then(|source| source.file.as_ref())
            .is_some_and(|file| file.max_bytes.is_some())
    };
    let least = coqui_access_kit::file::DEFAULT_MAX_BYTES as u64;
    if config.source.is_none() && config.pending.is_none() {
        return Ok(least as u32);
    }
    let (Some(name), Some(label)) = (
        config.record_limits.field_bytes.get(),
        config.record_limits.display_name_bytes.get(),
    ) else {
        return if named(&config.source) && (config.pending.is_none() || named(&config.pending)) {
            Ok(least as u32)
        } else {
            Err(coqui_access::error::bad_value(
                "an unbounded record_limits field_bytes or display_name_bytes needs a max_bytes on the source files",
            ))
        };
    };
    let record = FIXED_BYTES
        + 2 * u64::from(name)
        + 2 * u64::from(label)
        + BYTES_PER_CODE * u64::from(config.recovery_codes);
    let wanted = u64::from(config.max_records).saturating_mul(record);
    Ok(u32::try_from(wanted.max(least)).unwrap_or(u32::MAX))
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
            .map_err(|reason| reason.writing("one-time code table")),
        None => Ok(Arc::new(MemoryTable::new())),
    }
}

/// The one-time code provider a host loads.
#[derive(Debug, Provider)]
#[provider(
    Authenticator,
    Accounts,
    id = "coqui_access_totp",
    name = "One-time codes",
    config = Described<TotpConfig>,
    crate = ::coqui_access::guatiao
)]
pub struct TotpProvider(TotpAccounts);

impl TryFrom<TotpConfig> for TotpProvider {
    type Error = ProviderError;

    fn try_from(config: TotpConfig) -> Result<Self, Self::Error> {
        let size = file_limit(&config)?;
        let sized = |source: &Source<Format>| source.clone().max_bytes_unless_set(size);
        let pending_source = config.pending.as_ref().map(sized).or_else(|| {
            config
                .source
                .as_ref()
                .map(|source| sized(&source.sibling("pending")))
        });
        let source = config.source.as_ref().map(sized);
        let mut builder = TotpAccounts::builder()
            .table(open::<TotpRecord>(source.as_ref(), "accounts")?)
            .pending(open::<PendingTotp>(pending_source.as_ref(), "pending")?)
            .algorithm(config.algorithm)
            .digits(config.digits)
            .period(config.period)
            .skew(config.skew)
            .recovery_codes(config.recovery_codes)
            .enrol_seconds(config.enrol_seconds)
            .max_records(config.max_records as usize)
            .lockout(Lockout::new(
                config.lockout_after,
                config.lockout_seconds,
                config.lockout_longest_seconds,
            ))
            .record_limits(config.record_limits);
        if let Some(issuer) = config.issuer {
            builder = builder.issuer(issuer);
        }
        builder
            .build()
            .map(Self)
            .map_err(|reason| reason.writing("one-time code table"))
    }
}

impl TryFrom<Described<TotpConfig>> for TotpProvider {
    type Error = ProviderError;

    fn try_from(config: Described<TotpConfig>) -> Result<Self, Self::Error> {
        Self::try_from(config.0)
    }
}

impl Authenticator for TotpProvider {
    fn authenticate(&self, attempt: Attempt) -> Result<Verdict, ProviderError> {
        self.0.authenticate(attempt)
    }

    fn describe(&self) -> Result<Description, ProviderError> {
        Authenticator::describe(&self.0)
    }
}

impl Accounts for TotpProvider {
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

coqui_access::guatiao::providers!(TotpProvider);

#[cfg(test)]
mod tests {
    use super::*;

    fn config(path: &std::path::Path) -> TotpConfig {
        TotpConfig {
            source: Some(Source::file(path.to_str().unwrap(), Format::Toml)),
            ..TotpConfig::default()
        }
    }

    fn begin(provider: &TotpProvider, name: &str) -> PutOutcome {
        provider
            .put(AccountPut {
                name: name.to_owned(),
                ..AccountPut::default()
            })
            .unwrap()
    }

    #[test]
    fn a_file_source_keeps_accounts_and_enrolments_beside_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let provider = TotpProvider::try_from(config(&path)).unwrap();
        let begun = begin(&provider, "id-alice");
        let pending = directory.path().join("totp.toml.pending");
        let text = std::fs::read_to_string(&pending).unwrap();
        assert!(text.contains("[[pending]]"), "{text}");
        // Neither the link, the secret nor a recovery code is written as itself in the
        // recovery digests.
        let first = begun.codes.unwrap().remove(0);
        assert!(!text.contains(&first) && !text.contains(&first.replace('-', "")));
        // A second provider over the same files sees the enrolment waiting.
        let other = TotpProvider::try_from(config(&path)).unwrap();
        assert_eq!(other.waiting().unwrap().len(), 1);
        assert!(other.accounts().unwrap().is_empty());
    }

    #[test]
    fn a_hand_written_file_with_a_bad_secret_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        std::fs::write(
            &path,
            "[[accounts]]\nname='id-alice'\nsecret='!!!'\nalgorithm='sha1'\ndigits=6\nperiod=30\n",
        )
        .unwrap();
        assert!(TotpProvider::try_from(config(&path)).is_err());
    }

    #[test]
    fn no_source_is_memory_and_the_defaults_read() {
        let provider = TotpProvider::try_from(TotpConfig::default()).unwrap();
        let begun = begin(&provider, "id-bob");
        assert_eq!(begun.codes.unwrap().len(), 10);
        let defaults = TotpConfig::default();
        assert_eq!(
            (defaults.digits, defaults.period, defaults.skew),
            (6, 30, 1)
        );
    }

    /// The code for the present step, written from RFC 4226 and independent of the crate.
    fn present_code(link: &str) -> String {
        use hmac::{Hmac, KeyInit, Mac};
        let secret = link
            .split(['?', '&'])
            .find_map(|part| part.strip_prefix("secret="))
            .unwrap();
        let key = data_encoding::BASE32_NOPAD
            .decode(secret.as_bytes())
            .unwrap();
        let now = coqui_access_kit::unix_now();
        let mut mac = <Hmac<sha1::Sha1> as KeyInit>::new_from_slice(&key).unwrap();
        mac.update(&((now / 30) as u64).to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let offset = usize::from(digest[19] & 0x0f);
        let number = u32::from_be_bytes([
            digest[offset] & 0x7f,
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
        ]);
        format!("{:06}", number % 1_000_000)
    }

    fn enrolled(provider: &TotpProvider, name: &str) -> Vec<String> {
        let begun = begin(provider, name);
        provider
            .put(AccountPut {
                name: name.to_owned(),
                secret: Some(present_code(&begun.secret.unwrap())),
                ..AccountPut::default()
            })
            .unwrap();
        begun.codes.unwrap()
    }

    fn recovery(name: &str, code: &str) -> Attempt {
        Attempt {
            name: Some(name.to_owned()),
            secret: Some(code.to_owned()),
            method: Some("recovery_code".to_owned()),
            ..Attempt::default()
        }
    }

    #[test]
    fn a_code_used_in_one_process_is_refused_in_another_over_the_same_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let first = TotpProvider::try_from(config(&path)).unwrap();
        let second = TotpProvider::try_from(config(&path)).unwrap();
        let codes = enrolled(&first, "id-alice");
        for code in codes.iter().take(5) {
            let outcomes = std::thread::scope(|scope| {
                let a = scope.spawn(|| first.authenticate(recovery("id-alice", code)).unwrap());
                let b = scope.spawn(|| second.authenticate(recovery("id-alice", code)).unwrap());
                [a.join().unwrap(), b.join().unwrap()]
            });
            let taken = outcomes
                .iter()
                .filter(|verdict| matches!(verdict, Verdict::Valid(_)))
                .count();
            assert_eq!(taken, 1, "one use of a one-time code");
        }
    }

    #[test]
    fn wrong_codes_are_counted_across_processes_over_the_same_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let delayed = |path: &std::path::Path| TotpConfig {
            lockout_after: 2,
            lockout_seconds: 600,
            lockout_longest_seconds: 600,
            ..config(path)
        };
        let first = TotpProvider::try_from(delayed(&path)).unwrap();
        let second = TotpProvider::try_from(delayed(&path)).unwrap();
        let codes = enrolled(&first, "id-alice");
        first
            .authenticate(recovery("id-alice", "AAAAA-AAAAA"))
            .unwrap();
        second
            .authenticate(recovery("id-alice", "BBBBB-BBBBB"))
            .unwrap();
        // Two wrong ones, one in each: the account is delayed for both.
        for provider in [&first, &second] {
            assert_eq!(
                provider
                    .authenticate(recovery("id-alice", &codes[0]))
                    .unwrap(),
                Verdict::Invalid
            );
        }
    }

    #[test]
    fn a_file_of_more_accounts_than_a_megabyte_holds_loads_under_the_default_bound() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let digest = "ab".repeat(32);
        let mut text = String::new();
        for number in 0..1_500 {
            text.push_str(&format!(
                "[[accounts]]
name = 'id-{number:05}'
secret = 'GEZDGNBVGY3TQOJQ'
algorithm = 'sha1'
digits = 6
period = 30
codes = [{}]
",
                (0..10)
                    .map(|_| format!("'{digest}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        assert!(text.len() > 1024 * 1024);
        std::fs::write(&path, text).unwrap();
        let provider = TotpProvider::try_from(config(&path)).unwrap();
        assert_eq!(provider.accounts().unwrap().len(), 1_500);
        // The source's own bound wins over the sized default.
        let small = TotpConfig {
            source: Some(Source::file(path.to_str().unwrap(), Format::Toml).max_bytes(4096)),
            ..TotpConfig::default()
        };
        assert!(TotpProvider::try_from(small).is_err());
        let sized = |recovery_codes: u32, max_records: u32| {
            file_limit(&TotpConfig {
                source: Some(Source::file("totp.toml", Format::Toml)),
                recovery_codes,
                max_records,
                ..TotpConfig::default()
            })
            .unwrap()
        };
        // The defaults: a 256-byte name and a 1 024-byte label doubled, and ten digests.
        assert_eq!(sized(10, 10_000), 10_000 * (400 + 512 + 2048 + 800));
        assert_eq!(sized(10, 10), 1024 * 1024);
        // More recovery codes make a larger record.
        assert!(sized(64, 10_000) > sized(10, 10_000));
        assert_eq!(sized(64, 10_000), 10_000 * (400 + 512 + 2048 + 64 * 80));
    }

    #[test]
    fn a_setting_out_of_range_does_not_build() {
        let bad = TotpConfig {
            digits: 12,
            ..TotpConfig::default()
        };
        assert!(TotpProvider::try_from(bad).is_err());
    }

    #[test]
    fn an_unbounded_field_limit_is_refused_unless_the_files_name_their_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let unbounded = |source: Source<Format>| TotpConfig {
            source: Some(source),
            record_limits: RecordLimits::UNBOUNDED,
            ..TotpConfig::default()
        };
        let plain = Source::file(path.to_str().unwrap(), Format::Toml);
        assert!(TotpProvider::try_from(unbounded(plain.clone())).is_err());
        assert!(TotpProvider::try_from(unbounded(plain.max_bytes(1 << 20))).is_ok());
        let memory = TotpConfig {
            record_limits: RecordLimits::UNBOUNDED,
            ..TotpConfig::default()
        };
        assert!(
            TotpProvider::try_from(memory).is_ok(),
            "memory has no file to size"
        );
    }

    #[test]
    fn a_file_of_accounts_with_the_most_recovery_codes_loads_under_the_sized_bound() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("totp.toml");
        let digests = (0..64)
            .map(|number| format!("'{number:02}{}'", "ab".repeat(31)))
            .collect::<Vec<_>>()
            .join(", ");
        let mut text = String::new();
        for number in 0..300 {
            text.push_str(&format!(
                "[[accounts]]
name = 'id-{number:05}'
secret = 'GEZDGNBVGY3TQOJQ'
algorithm = 'sha1'
digits = 6
period = 30
codes = [{digests}]
"
            ));
        }
        assert!(text.len() > 1024 * 1024, "past the kit's default bound");
        std::fs::write(&path, text).unwrap();
        let sixty_four = TotpConfig {
            recovery_codes: 64,
            max_records: 300,
            ..config(&path)
        };
        let provider = TotpProvider::try_from(sixty_four).unwrap();
        assert_eq!(provider.accounts().unwrap().len(), 300);
    }
}
