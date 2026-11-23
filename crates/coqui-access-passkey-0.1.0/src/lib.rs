//! Passkeys (WebAuthn) as a sign-in, kept in any [`Table`].
//!
//! A person registers a passkey through [`Accounts::put`]: the first call answers the options
//! for `navigator.credentials.create`, the second takes the browser's answer. A sign-in is two
//! calls on one provider: [`Challenger::challenge`](coqui_access::Challenger::challenge) answers
//! the options for `navigator.credentials.get`, and
//! [`Authenticator::authenticate`](coqui_access::Authenticator::authenticate) takes the signed
//! answer with [`Attempt::challenge`](coqui_access::Attempt::challenge) set to the challenge the
//! first call made. Verification is `webauthn-rs-core`'s; nothing is checked by hand here.
//!
//! The options and the answers cross as WebAuthn's own JSON text (`PublicKeyCredential.toJSON()`),
//! passed between the browser and the provider unread by the consumer. The provider does not
//! throttle answers: the consumer's per-address throttle counts them, as for a certificate.
//! Build it in code with [`Passkeys::builder`].
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]

mod ceremony;
mod records;
mod settings;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use coqui_access::guatiao::library::ProviderError;
use coqui_access::{
    AccountInfo, AccountPut, Accounts, Directory, Limit, Person, PutOutcome, Waiting, error,
};
use coqui_access_kit::{
    MemoryTable, RecordLimits, Reloading, SourceError, Table, check_keys, secret, unix_now,
};
use coqui_builder::{Datum, NonDefault, Shown};
use coqui_peer::WaitingLimits;
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use url::Url;
use webauthn_attestation_ca::AttestationCaList;
use webauthn_rs_core::WebauthnCore;
use webauthn_rs_core::proto::{
    CreationChallengeResponse, RegisterPublicKeyCredential, RegistrationState,
};

pub use records::{Ceremony, PasskeyAccount, PendingRegistration, StoredCredential};
pub use settings::{Algorithm, Attestation, OnRegression, ResidentKey, UserVerification};

/// The most records each table may hold unless the builder says otherwise.
pub const DEFAULT_MAX_RECORDS: usize = 10_000;
/// Seconds a registration waits for the browser's answer unless the builder says otherwise.
pub const DEFAULT_ENROL_SECONDS: u32 = 600;
/// Seconds a sign-in challenge lives, and the timeout the browser is told, unless the builder
/// says otherwise.
pub const DEFAULT_CEREMONY_SECONDS: u32 = 300;
/// Passkeys one account may hold unless the builder says otherwise.
pub const DEFAULT_MAX_CREDENTIALS: Limit = Limit::at(16);
/// The largest answer from a browser, in bytes, unless the builder says otherwise.
pub const DEFAULT_MAX_RESPONSE_BYTES: Limit = Limit::at(65_536);
/// Places one source holds among the challenges waiting for an answer unless the builder says
/// otherwise.
pub const DEFAULT_CEREMONIES_PER_SOURCE: usize = 8;
/// Challenges waiting for an answer in all unless the builder says otherwise.
pub const DEFAULT_CEREMONIES_TOTAL: usize = 1_024;

/// The bounds on the sign-in challenges waiting for an answer unless the builder says
/// otherwise: 8 per source (an IPv4 address, or an IPv6 /64), 1 024 in all.
pub fn default_ceremony_limits() -> WaitingLimits {
    let mut limits = WaitingLimits::default();
    limits.per_source = DEFAULT_CEREMONIES_PER_SOURCE;
    limits.total = DEFAULT_CEREMONIES_TOTAL;
    limits
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Options for [`Passkeys`]; every one has a default except the relying party and its origins.
pub struct PasskeysBuilder {
    rp_id: Option<String>,
    rp_name: Option<String>,
    origins: Vec<String>,
    allow_subdomains: bool,
    allow_any_port: bool,
    user_verification: UserVerification,
    resident_key: ResidentKey,
    attestation: Attestation,
    attestation_cas: Vec<Vec<u8>>,
    algorithms: Vec<Algorithm>,
    reject_synced: bool,
    enrol_seconds: u32,
    ceremony_seconds: u32,
    max_records: usize,
    max_credentials: Limit,
    max_response_bytes: Limit,
    ceremony_limits: WaitingLimits,
    counter_regression: OnRegression,
    limits: RecordLimits,
    table: Option<Arc<dyn Table<PasskeyAccount>>>,
    pending: Option<Arc<dyn Table<PendingRegistration>>>,
    ceremonies: Option<Arc<dyn Table<Ceremony>>>,
    clock: Clock,
}

impl fmt::Debug for PasskeysBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeysBuilder")
            .field("rp_id", &self.rp_id)
            .field("origins", &self.origins.len())
            .finish_non_exhaustive()
    }
}

impl PasskeysBuilder {
    /// The relying party id: the registrable domain the browser binds a passkey to
    /// (`portal.example`). Required; a passkey cannot be used from outside it.
    #[must_use]
    pub fn rp_id(mut self, rp_id: impl Into<String>) -> Self {
        self.rp_id = Some(rp_id.into());
        self
    }

    /// What the browser shows of the relying party. Default the `rp_id`.
    #[must_use]
    pub fn rp_name(mut self, name: impl Into<String>) -> Self {
        self.rp_name = Some(name.into());
        self
    }

    /// An origin a passkey may be used from (`https://portal.example`): a scheme, a host and
    /// optionally a port, nothing else, and the host is the `rp_id` or under it. Required at
    /// least once; repeat for each.
    #[must_use]
    pub fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origins.push(origin.into());
        self
    }

    /// Whether a subdomain of an origin's host is an origin too. Default false.
    #[must_use]
    pub fn allow_subdomains(mut self, allow: bool) -> Self {
        self.allow_subdomains = allow;
        self
    }

    /// Whether any port of an origin's host is an origin too. Default false.
    #[must_use]
    pub fn allow_any_port(mut self, allow: bool) -> Self {
        self.allow_any_port = allow;
        self
    }

    /// Whether the person must be verified. Default [`UserVerification::Required`]; anything
    /// else is listed by [`Passkeys::non_defaults`].
    #[must_use]
    pub fn user_verification(mut self, verification: UserVerification) -> Self {
        self.user_verification = verification;
        self
    }

    /// Whether a registration asks for a discoverable credential. Default
    /// [`ResidentKey::Required`].
    #[must_use]
    pub fn resident_key(mut self, resident_key: ResidentKey) -> Self {
        self.resident_key = resident_key;
        self
    }

    /// What a registration asks the authenticator to say about itself. Default
    /// [`Attestation::None`].
    #[must_use]
    pub fn attestation(mut self, attestation: Attestation) -> Self {
        self.attestation = attestation;
        self
    }

    /// A certificate authority (PEM) a registration's attestation must chain to; with any, a
    /// registration that does not fails. Repeat for each. Needs an attestation other than none.
    #[must_use]
    pub fn attestation_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.attestation_cas.push(pem.into());
        self
    }

    /// The algorithms a registration offers, in order of preference. Default
    /// [`Algorithm::DEFAULT`]. [`Algorithm::Eddsa`] is refused when the provider is built.
    #[must_use]
    pub fn algorithms(mut self, algorithms: impl IntoIterator<Item = Algorithm>) -> Self {
        self.algorithms = algorithms.into_iter().collect();
        self
    }

    /// Whether a passkey the platform syncs between devices is refused: at registration, and at
    /// sign-in for one registered before this was set. Default false: synced passkeys are
    /// accepted.
    #[must_use]
    pub fn reject_synced(mut self, reject: bool) -> Self {
        self.reject_synced = reject;
        self
    }

    /// Seconds a registration waits for the browser's answer. Default
    /// [`DEFAULT_ENROL_SECONDS`].
    #[must_use]
    pub fn enrol_seconds(mut self, seconds: u32) -> Self {
        self.enrol_seconds = seconds;
        self
    }

    /// Seconds a sign-in challenge lives, and the timeout the browser is told. Default
    /// [`DEFAULT_CEREMONY_SECONDS`].
    #[must_use]
    pub fn ceremony_seconds(mut self, seconds: u32) -> Self {
        self.ceremony_seconds = seconds;
        self
    }

    /// The most accounts, and the most registrations waiting, the tables may hold. Default
    /// [`DEFAULT_MAX_RECORDS`].
    #[must_use]
    pub fn max_records(mut self, max: usize) -> Self {
        self.max_records = max;
        self
    }

    /// The most passkeys one account may hold. Default [`DEFAULT_MAX_CREDENTIALS`].
    #[must_use]
    pub fn max_credentials(mut self, max: Limit) -> Self {
        self.max_credentials = max;
        self
    }

    /// The largest answer from a browser, in bytes. Default [`DEFAULT_MAX_RESPONSE_BYTES`].
    #[must_use]
    pub fn max_response_bytes(mut self, max: Limit) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// The bounds on the challenges waiting for an answer, applied by coqui-peer's
    /// [`WaitingQueue`](coqui_peer::WaitingQueue) per source and in all. Default
    /// [`default_ceremony_limits`].
    #[must_use]
    pub fn ceremony_limits(mut self, limits: WaitingLimits) -> Self {
        self.ceremony_limits = limits;
        self
    }

    /// What a counter that does not rise does. Default [`OnRegression::Disable`].
    #[must_use]
    pub fn counter_regression(mut self, on: OnRegression) -> Self {
        self.counter_regression = on;
        self
    }

    /// How large an account's name, group and display name may be. Default
    /// [`RecordLimits::DEFAULT`].
    #[must_use]
    pub fn record_limits(mut self, limits: RecordLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Keeps the accounts in `table` instead of in memory.
    #[must_use]
    pub fn table(mut self, table: Arc<dyn Table<PasskeyAccount>>) -> Self {
        self.table = Some(table);
        self
    }

    /// Keeps the registrations waiting for an answer in `table` instead of in memory.
    #[must_use]
    pub fn pending(mut self, table: Arc<dyn Table<PendingRegistration>>) -> Self {
        self.pending = Some(table);
        self
    }

    /// Keeps the sign-in challenges in `table` instead of in memory. A host that builds one
    /// instance per kind needs a table they all read.
    #[must_use]
    pub fn ceremonies(mut self, table: Arc<dyn Table<Ceremony>>) -> Self {
        self.ceremonies = Some(table);
        self
    }

    /// The clock, in seconds since the Unix epoch. Default the system's; a test supplies its
    /// own.
    #[must_use]
    pub fn clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Loads the tables and makes the provider.
    ///
    /// # Errors
    ///
    /// When the relying party or an origin is missing or malformed (an origin with a path, a
    /// query or a fragment, or whose host is neither the `rp_id` nor under it, could never sign
    /// anyone in), no algorithm is offered or `eddsa` is, an attestation authority is given with
    /// no attestation asked for or cannot be read, no time is given to register or sign in, or a
    /// table cannot be loaded or holds an invalid or duplicated record.
    pub fn build(self) -> Result<Passkeys, SourceError> {
        let rp_id = self
            .rp_id
            .ok_or_else(|| SourceError::invalid("a passkey provider needs an rp_id"))?;
        if !is_domain(&rp_id) {
            return Err(SourceError::invalid("the rp_id is not a domain name"));
        }
        if self.origins.is_empty() {
            return Err(SourceError::invalid("a passkey provider needs an origin"));
        }
        let origins = self
            .origins
            .iter()
            .map(|text| parse_origin(text, &rp_id))
            .collect::<Result<Vec<_>, _>>()?;
        if self.algorithms.is_empty() {
            return Err(SourceError::invalid(
                "a passkey provider needs an algorithm",
            ));
        }
        if self.algorithms.contains(&Algorithm::Eddsa) {
            return Err(SourceError::invalid(
                "eddsa is not offered: webauthn-rs-core does not verify EdDSA signatures",
            ));
        }
        if self.enrol_seconds == 0 || self.ceremony_seconds == 0 {
            return Err(SourceError::invalid(
                "a passkey needs time to register and to sign in",
            ));
        }
        let cas = if self.attestation_cas.is_empty() {
            None
        } else {
            if self.attestation == Attestation::None {
                return Err(SourceError::invalid(
                    "attestation authorities need an attestation other than none",
                ));
            }
            let mut list = AttestationCaList::default();
            for pem in &self.attestation_cas {
                let one = AttestationCaList::try_from(pem.as_slice()).map_err(|_| {
                    SourceError::invalid("an attestation authority is not a PEM certificate")
                })?;
                list.union(&one);
            }
            Some(list)
        };
        let rp_name = self.rp_name.unwrap_or_else(|| rp_id.clone());
        let core = WebauthnCore::new_unsafe_experts_only(
            &rp_name,
            &rp_id,
            origins,
            Duration::from_secs(u64::from(self.ceremony_seconds)),
            Some(self.allow_subdomains),
            Some(self.allow_any_port),
        );
        let table = self
            .table
            .unwrap_or_else(|| Arc::new(MemoryTable::<PasskeyAccount>::new()));
        let pending = self
            .pending
            .unwrap_or_else(|| Arc::new(MemoryTable::<PendingRegistration>::new()));
        let ceremonies = self
            .ceremonies
            .unwrap_or_else(|| Arc::new(MemoryTable::<Ceremony>::new()));
        let (max, limits) = (self.max_records, self.limits);
        Ok(Passkeys {
            core,
            accounts: Reloading::new(table, move |records| validate(records, max, &limits))?,
            pending: Reloading::new(pending, move |records| {
                validate_pending(records, max, &limits)
            })?,
            ceremonies,
            limits,
            cas,
            rp_id,
            origins: self.origins,
            allow_subdomains: self.allow_subdomains,
            allow_any_port: self.allow_any_port,
            user_verification: self.user_verification,
            resident_key: self.resident_key,
            attestation: self.attestation,
            algorithms: self.algorithms,
            reject_synced: self.reject_synced,
            enrol_seconds: self.enrol_seconds,
            ceremony_seconds: self.ceremony_seconds,
            max_credentials: self.max_credentials,
            max_response_bytes: self.max_response_bytes,
            ceremony_limits: self.ceremony_limits,
            counter_regression: self.counter_regression,
            clock: self.clock,
        })
    }
}

/// Whether `text` is a domain name a browser could hold a relying party id for.
fn is_domain(text: &str) -> bool {
    let Ok(url) = Url::parse(&format!("https://{text}/")) else {
        return false;
    };
    matches!(url.host(), Some(url::Host::Domain(host)) if host == text)
        && url.port().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

/// Whether `host` is one the browser treats as a secure context without TLS.
fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "[::1]"
}

/// An origin as the verifier takes it: a web address with a scheme, a host and a port only,
/// whose host the relying party id covers.
fn parse_origin(text: &str, rp_id: &str) -> Result<Url, SourceError> {
    let url = Url::parse(text).map_err(|_| SourceError::invalid("an origin is not a URL"))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(SourceError::invalid(
            "an origin is not an http or https one",
        ));
    }
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(SourceError::invalid(
            "an origin is a scheme, a host and a port: no path, query, fragment or credentials",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| SourceError::invalid("an origin has no host"))?;
    if host != rp_id && !host.ends_with(&format!(".{rp_id}")) {
        return Err(SourceError::invalid(
            "an origin's host is neither the rp_id nor under it, so no passkey could sign in from it",
        ));
    }
    Ok(url)
}

fn validate(
    records: &[PasskeyAccount],
    max: usize,
    limits: &RecordLimits,
) -> Result<(), SourceError> {
    check_keys(records, max)?;
    for record in records {
        let valid = limits.allows_field(&record.name)
            && record.id.len() == 32
            && record
                .id
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            && limits.allows_groups(&record.groups)
            && record
                .display_name
                .as_deref()
                .is_none_or(|name| limits.allows_display_name(name))
            && record.credentials.iter().all(|credential| {
                !credential.id.is_empty()
                    && credential
                        .label
                        .as_deref()
                        .is_none_or(|label| limits.allows_display_name(label))
            });
        if !valid {
            return Err(SourceError::invalid("an account entry is invalid"));
        }
    }
    Ok(())
}

fn validate_pending(
    entries: &[PendingRegistration],
    max: usize,
    limits: &RecordLimits,
) -> Result<(), SourceError> {
    check_keys(entries, max)?;
    for entry in entries {
        let valid = limits.allows_field(&entry.name)
            && entry.id.len() == 32
            && entry
                .display_name
                .as_deref()
                .is_none_or(|name| limits.allows_display_name(name));
        if !valid {
            return Err(SourceError::invalid("a registration entry is invalid"));
        }
    }
    Ok(())
}

/// Passkeys as an [`Accounts`] and [`Directory`] implementation; with the `ceremony` module's
/// [`Challenger`](coqui_access::Challenger) and [`Authenticator`](coqui_access::Authenticator)
/// they are a sign-in.
///
/// ```
/// use coqui_access::{AccountPut, Accounts};
/// use coqui_access_passkey::Passkeys;
///
/// let passkeys = Passkeys::builder()
///     .rp_id("portal.example")
///     .origin("https://portal.example")
///     .build()
///     .unwrap();
/// let begun = passkeys
///     .put(AccountPut { name: "alice".into(), ..AccountPut::default() })
///     .unwrap();
/// // The options for `navigator.credentials.create`, as JSON text.
/// assert!(begun.secret.unwrap().starts_with('{'));
/// ```
pub struct Passkeys {
    core: WebauthnCore,
    accounts: Reloading<PasskeyAccount>,
    pending: Reloading<PendingRegistration>,
    ceremonies: Arc<dyn Table<Ceremony>>,
    limits: RecordLimits,
    cas: Option<AttestationCaList>,
    rp_id: String,
    origins: Vec<String>,
    allow_subdomains: bool,
    allow_any_port: bool,
    user_verification: UserVerification,
    resident_key: ResidentKey,
    attestation: Attestation,
    algorithms: Vec<Algorithm>,
    reject_synced: bool,
    enrol_seconds: u32,
    ceremony_seconds: u32,
    max_credentials: Limit,
    max_response_bytes: Limit,
    ceremony_limits: WaitingLimits,
    counter_regression: OnRegression,
    clock: Clock,
}

impl fmt::Debug for Passkeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Passkeys")
            .field("rp_id", &self.rp_id)
            .field("origins", &self.origins.len())
            .field("user_verification", &self.user_verification)
            .finish_non_exhaustive()
    }
}

impl Passkeys {
    /// Options for the provider, kept in memory unless a table says otherwise.
    pub fn builder() -> PasskeysBuilder {
        PasskeysBuilder {
            rp_id: None,
            rp_name: None,
            origins: Vec::new(),
            allow_subdomains: false,
            allow_any_port: false,
            user_verification: UserVerification::default(),
            resident_key: ResidentKey::default(),
            attestation: Attestation::default(),
            attestation_cas: Vec::new(),
            algorithms: Algorithm::DEFAULT.to_vec(),
            reject_synced: false,
            enrol_seconds: DEFAULT_ENROL_SECONDS,
            ceremony_seconds: DEFAULT_CEREMONY_SECONDS,
            max_records: DEFAULT_MAX_RECORDS,
            max_credentials: DEFAULT_MAX_CREDENTIALS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            ceremony_limits: default_ceremony_limits(),
            counter_regression: OnRegression::default(),
            limits: RecordLimits::DEFAULT,
            table: None,
            pending: None,
            ceremonies: None,
            clock: Arc::new(unix_now),
        }
    }

    /// Every setting that differs from its default and weakens or widens what a passkey proves:
    /// the user verification asked for, a discoverable credential not asked for, subdomains and
    /// any port accepted, a counter that is only refused, and each origin that is not https
    /// (a loopback host is not listed: browsers treat it as secure).
    pub fn non_defaults(&self) -> Vec<NonDefault> {
        let word = |ty: &'static str, text: &str| Shown::One(Datum::new(ty, text));
        let mut found = Vec::new();
        if self.user_verification != UserVerification::default() {
            found.push(NonDefault::new(
                "user_verification",
                word("UserVerification", self.user_verification.as_str()),
                word("UserVerification", UserVerification::default().as_str()),
            ));
        }
        if self.resident_key != ResidentKey::default() {
            found.push(NonDefault::new(
                "resident_key",
                word("ResidentKey", self.resident_key.as_str()),
                word("ResidentKey", ResidentKey::default().as_str()),
            ));
        }
        if self.allow_subdomains {
            found.push(NonDefault::new(
                "allow_subdomains",
                word("bool", "true"),
                word("bool", "false"),
            ));
        }
        if self.allow_any_port {
            found.push(NonDefault::new(
                "allow_any_port",
                word("bool", "true"),
                word("bool", "false"),
            ));
        }
        if self.counter_regression != OnRegression::default() {
            found.push(NonDefault::new(
                "counter_regression",
                word("OnRegression", self.counter_regression.as_str()),
                word("OnRegression", OnRegression::default().as_str()),
            ));
        }
        for origin in &self.origins {
            let secure = Url::parse(origin).is_ok_and(|url| {
                url.scheme() == "https" || url.host_str().is_some_and(is_loopback)
            });
            if !secure {
                found.push(NonDefault::new(
                    "origin",
                    word("String", origin),
                    word("String", "https"),
                ));
            }
        }
        found
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// The options for `navigator.credentials.create`, and what the verifier keeps to check the
    /// answer, for the account `id` that already holds `exclude`.
    fn registration_options(
        &self,
        id: &str,
        name: &str,
        display_name: &str,
        exclude: Vec<Vec<u8>>,
    ) -> Result<(String, String), ProviderError> {
        let handle = HEXLOWER
            .decode(id.as_bytes())
            .map_err(|_| error::internal("an account id is not hex"))?;
        let algorithms = self
            .algorithms
            .iter()
            .filter_map(|algorithm| algorithm.cose())
            .collect();
        let builder = self
            .core
            .new_challenge_register_builder(&handle, name, display_name)
            .map_err(|_| error::bad_value("that account cannot register a passkey"))?
            .attestation(self.attestation.preference())
            .user_verification_policy(self.user_verification.policy())
            .credential_algorithms(algorithms)
            .require_resident_key(self.resident_key == ResidentKey::Required)
            .reject_synchronised_authenticators(self.reject_synced)
            .exclude_credentials(Some(exclude));
        let (options, state): (CreationChallengeResponse, RegistrationState) = self
            .core
            .generate_challenge_register(builder)
            .map_err(|_| error::internal("the registration options could not be made"))?;
        let options = serde_json::to_string(&options)
            .map_err(|_| error::internal("the registration options could not be written"))?;
        let state = serde_json::to_string(&state)
            .map_err(|_| error::internal("the registration state could not be written"))?;
        Ok((options, state))
    }

    /// Begins a registration for `account`, replacing one already waiting.
    fn begin(&self, account: &AccountPut) -> Result<PutOutcome, ProviderError> {
        if account
            .display_name
            .as_deref()
            .is_some_and(|label| !self.limits.allows_display_name(label))
        {
            return Err(error::bad_value("that display name is not valid"));
        }
        let held = self
            .accounts
            .get(&account.name)
            .map_err(|reason| reason.reading("passkey table"))?;
        let (id, exclude) = match &held {
            Some(held) => (
                held.id.clone(),
                held.credentials
                    .iter()
                    .filter_map(|credential| BASE64URL_NOPAD.decode(credential.id.as_bytes()).ok())
                    .collect(),
            ),
            None => (
                secret::identifier().map_err(|_| error::internal("no random source"))?,
                Vec::new(),
            ),
        };
        let shown = account
            .display_name
            .clone()
            .or_else(|| held.as_ref().and_then(|held| held.display_name.clone()));
        let (options, state) = self.registration_options(
            &id,
            &account.name,
            shown.as_deref().unwrap_or(&account.name),
            exclude,
        )?;
        let entry = PendingRegistration {
            name: account.name.clone(),
            id,
            display_name: shown,
            state,
            created_unix: self.now(),
        };
        self.pending
            .put_with(|_| Ok((entry, ())))
            .map_err(|reason| reason.writing("passkey table"))?;
        Ok(PutOutcome {
            secret: Some(options),
            ..PutOutcome::default()
        })
    }

    /// Takes the browser's answer to the registration waiting for `account`.
    fn confirm(&self, account: &AccountPut, text: &str) -> Result<PutOutcome, ProviderError> {
        if !self.max_response_bytes.allows(text.len()) {
            return Err(error::bad_value("that answer is too large"));
        }
        let entry = self
            .pending
            .get(&account.name)
            .map_err(|reason| reason.reading("passkey table"))?
            .ok_or_else(|| error::not_found("no registration is waiting for that account"))?;
        // Whichever call removes the entry owns it: a second answer for it finds none.
        let taken = self
            .pending
            .remove(&account.name)
            .map_err(|reason| reason.writing("passkey table"))?;
        if !taken {
            return Err(error::not_found(
                "no registration is waiting for that account",
            ));
        }
        let now = self.now();
        if now.saturating_sub(entry.created_unix) > i64::from(self.enrol_seconds) {
            return Err(error::bad_value("the registration has expired"));
        }
        let state: RegistrationState = serde_json::from_str(&entry.state)
            .map_err(|_| error::internal("a waiting registration cannot be read"))?;
        let answer: RegisterPublicKeyCredential = serde_json::from_str(text)
            .map_err(|_| error::bad_value("that is not a passkey registration"))?;
        let credential = self
            .core
            .register_credential(&answer, &state, self.cas.as_ref())
            .map_err(|refused| {
                error::bad_value(&format!("the registration was refused: {refused}"))
            })?;
        let credential_id = BASE64URL_NOPAD.encode(credential.cred_id.as_slice());
        let key = serde_json::to_string(&credential)
            .map_err(|_| error::internal("a passkey could not be written"))?;
        if account
            .display_name
            .as_deref()
            .is_some_and(|label| !self.limits.allows_display_name(label))
        {
            return Err(error::bad_value("that display name is not valid"));
        }
        if let Some(groups) = &account.groups
            && !self.limits.allows_groups(groups)
        {
            return Err(error::bad_value("those groups are not valid"));
        }
        let stored = StoredCredential {
            id: credential_id.clone(),
            label: account.display_name.clone(),
            made_unix: now,
            used_unix: None,
            disabled: false,
            key,
        };
        self.accounts
            .put_with(|records| {
                if records
                    .iter()
                    .any(|held| held.credentials.iter().any(|own| own.id == credential_id))
                {
                    return Err(SourceError::invalid("that passkey is already registered"));
                }
                let mut next = match records.iter().find(|held| held.name == entry.name) {
                    Some(held) if held.id != entry.id => {
                        return Err(SourceError::invalid(
                            "the account changed since the registration began",
                        ));
                    }
                    Some(held) => held.clone(),
                    None => PasskeyAccount {
                        name: entry.name.clone(),
                        id: entry.id.clone(),
                        display_name: entry.display_name.clone(),
                        groups: Vec::new(),
                        disabled: false,
                        made_unix: Some(now),
                        credentials: Vec::new(),
                    },
                };
                if !self.max_credentials.allows(next.credentials.len() + 1) {
                    return Err(SourceError::invalid("the account holds too many passkeys"));
                }
                next.credentials.push(stored.clone());
                if let Some(groups) = &account.groups {
                    next.groups = groups.clone();
                }
                Ok((next, ()))
            })
            .map_err(|reason| reason.writing("passkey table"))?;
        Ok(PutOutcome::default())
    }

    /// Changes one account's `disabled` or `groups`.
    fn change(&self, account: &AccountPut) -> Result<PutOutcome, ProviderError> {
        if let Some(groups) = &account.groups
            && !self.limits.allows_groups(groups)
        {
            return Err(error::bad_value("those groups are not valid"));
        }
        self.edit(&account.name, |held| {
            if let Some(disabled) = account.disabled {
                held.disabled = disabled;
            }
            if let Some(groups) = &account.groups {
                held.groups = groups.clone();
            }
            Ok(())
        })
    }

    /// Changes the account `name` in one write under the table's lock; `NOT_FOUND` when there
    /// is none.
    fn edit(
        &self,
        name: &str,
        change: impl FnOnce(&mut PasskeyAccount) -> Result<(), SourceError>,
    ) -> Result<PutOutcome, ProviderError> {
        let found = self
            .accounts
            .update_one(name, |current| {
                let Some(current) = current else {
                    return Ok((None, false));
                };
                let mut next = current.clone();
                change(&mut next)?;
                Ok((Some(next), true))
            })
            .map_err(|reason| reason.writing("passkey table"))?;
        if found {
            Ok(PutOutcome::default())
        } else {
            Err(error::not_found("there is no such account"))
        }
    }

    /// An administrator's reset: every passkey and any registration waiting go; the account
    /// stays, so its id and groups do.
    fn reset(&self, account: &AccountPut) -> Result<PutOutcome, ProviderError> {
        let waiting = self
            .pending
            .remove(&account.name)
            .map_err(|reason| reason.writing("passkey table"))?;
        match self.edit(&account.name, |held| {
            held.credentials.clear();
            Ok(())
        }) {
            Err(refused)
                if waiting
                    && refused.status == coqui_access::guatiao::Status::GUATIAO_ERR_NOT_FOUND =>
            {
                Ok(PutOutcome::default())
            }
            other => other,
        }
    }

    /// Takes one passkey away.
    fn remove_credential(&self, name: &str, id: &str) -> Result<PutOutcome, ProviderError> {
        self.edit(name, |held| {
            let before = held.credentials.len();
            held.credentials.retain(|credential| credential.id != id);
            if held.credentials.len() == before {
                return Err(SourceError::invalid("the account holds no such passkey"));
            }
            Ok(())
        })
    }

    /// Sets one passkey's label from `display_name`.
    fn label_credential(
        &self,
        account: &AccountPut,
        id: &str,
    ) -> Result<PutOutcome, ProviderError> {
        let Some(label) = account.display_name.clone() else {
            return Err(error::bad_value("a label needs a display name"));
        };
        if !self.limits.allows_display_name(&label) {
            return Err(error::bad_value("that label is not valid"));
        }
        self.edit(&account.name, |held| {
            let Some(credential) = held
                .credentials
                .iter_mut()
                .find(|credential| credential.id == id)
            else {
                return Err(SourceError::invalid("the account holds no such passkey"));
            };
            credential.label = Some(label);
            Ok(())
        })
    }
}

impl Accounts for Passkeys {
    fn accounts(&self) -> Result<Vec<coqui_access::AccountInfo>, ProviderError> {
        self.accounts
            .read(|records| {
                records
                    .iter()
                    .map(|record| AccountInfo {
                        id: record.id.clone(),
                        name: record.name.clone(),
                        groups: record.groups.clone(),
                        disabled: Some(record.disabled),
                        made_unix: record.made_unix,
                        credentials: Some(record.credentials.iter().map(ceremony::info).collect()),
                        ..AccountInfo::default()
                    })
                    .collect()
            })
            .map_err(|reason| reason.reading("passkey table"))
    }

    /// Registers, and manages one account's passkeys.
    ///
    /// - No `secret`, `disabled`, `groups` or `reset`: begins a registration for `name`,
    ///   replacing one waiting, and answers the options for `navigator.credentials.create` as JSON
    ///   text in `secret`. `display_name` is what the browser shows (the name when absent).
    /// - `secret` the browser's answer (JSON text, starting `{`): confirms the registration
    ///   waiting, which is removed whatever follows. `display_name` becomes the passkey's label
    ///   and `groups` the account's, when given.
    /// - `secret` `remove:<id>` takes one passkey away; `label:<id>` sets its label from
    ///   `display_name`.
    /// - `disabled` or `groups` alone change the account; `reset` takes every passkey and any
    ///   registration waiting away. `unlock` is ignored: a passkey has no lockout.
    ///
    /// `expires_unix` is refused.
    fn put(&self, account: AccountPut) -> Result<PutOutcome, ProviderError> {
        if !self.limits.allows_field(&account.name) {
            return Err(error::bad_value("an account has a name"));
        }
        if account.expires_unix.is_some() {
            return Err(error::bad_value("passkey accounts do not expire"));
        }
        if account.reset {
            if account.secret.is_some() {
                return Err(error::bad_value("a reset does not come with a secret"));
            }
            return self.reset(&account);
        }
        match account.secret.as_deref() {
            Some(text) if text.starts_with('{') => self.confirm(&account, text),
            Some(text) => {
                if let Some(id) = text.strip_prefix("remove:") {
                    self.remove_credential(&account.name, id)
                } else if let Some(id) = text.strip_prefix("label:") {
                    self.label_credential(&account, id)
                } else {
                    Err(error::bad_value(
                        "a secret is the browser's answer, remove:<id> or label:<id>",
                    ))
                }
            }
            None if account.disabled.is_some() || account.groups.is_some() => self.change(&account),
            // An unlock has nothing to clear; it must not begin a registration.
            None if account.unlock => self.edit(&account.name, |_| Ok(())),
            None => self.begin(&account),
        }
    }

    /// Takes the account, its passkeys and any registration waiting away;
    /// `GUATIAO_ERR_NOT_FOUND` when neither existed.
    fn remove(&self, name: String) -> Result<(), ProviderError> {
        let account = self
            .accounts
            .remove(&name)
            .map_err(|reason| reason.writing("passkey table"))?;
        let waiting = self
            .pending
            .remove(&name)
            .map_err(|reason| reason.writing("passkey table"))?;
        if account || waiting {
            Ok(())
        } else {
            Err(error::not_found("there is no such account"))
        }
    }

    /// The registrations that have not been confirmed, `shown` being the account name.
    fn waiting(&self) -> Result<Vec<Waiting>, ProviderError> {
        self.pending
            .read(|entries| {
                entries
                    .iter()
                    .map(|entry| Waiting {
                        shown: entry.name.clone(),
                        peer: None,
                        seen_unix: Some(entry.created_unix),
                    })
                    .collect()
            })
            .map_err(|reason| reason.reading("passkey table"))
    }
}

impl Directory for Passkeys {
    /// Looks an enabled account up by `id` or `name`.
    fn lookup(&self, lookup: coqui_access::Lookup) -> Result<Option<Person>, ProviderError> {
        self.accounts
            .read(|records| {
                records
                    .iter()
                    .find(|record| {
                        !record.disabled
                            && match lookup.by.as_str() {
                                "id" => record.id == lookup.value,
                                "name" => record.name == lookup.value,
                                _ => false,
                            }
                    })
                    .map(|record| Person {
                        id: record.id.clone(),
                        name: record.name.clone(),
                        display_name: record.display_name.clone(),
                        email: None,
                        groups: record.groups.clone(),
                        attributes: Default::default(),
                    })
            })
            .map_err(|reason| reason.reading("passkey table"))
    }
}
