//! Client certificates held by a PKCS #11 module: smart cards, tokens and SoftHSM.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
use cryptoki::error::{Error as CryptokiError, RvError};
use cryptoki::mechanism::rsa::{PkcsMgfType, PkcsPssParams};
use cryptoki::mechanism::{Mechanism, MechanismType};
use cryptoki::object::{Attribute, AttributeType, CertificateType, ObjectClass, ObjectHandle};
use cryptoki::session::{Session, SessionState, UserType};
use cryptoki::slot::Slot;
use cryptoki::types::{AuthPin, Ulong};
use rustls::SignatureScheme;
use rustls::pki_types::CertificateDer;
use rustls::sign::{Signer, SigningKey};
use sha2::{Digest, Sha256, Sha384, Sha512};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Encode};

use crate::error::{Error, Reason};
use crate::identity::{CertificateInfo, Identity};
use crate::pin::{KeystoreKind, Pin, PinAnswer, PinPurpose, PinRequest};
use crate::select::IdentitySource;
use crate::settings::Pkcs11Settings;

fn failure(error: &CryptokiError) -> Error {
    match error {
        CryptokiError::Pkcs11(RvError::PinIncorrect, _) => Error::WrongPin,
        other => Error::Keystore {
            code: None,
            reason: Reason::new(alloc::format!("pkcs11: {other}")),
        },
    }
}

/// Whether the session, its token or its login is gone, as when a card is removed and reinserted.
fn lost(error: &CryptokiError) -> bool {
    matches!(
        error,
        CryptokiError::Pkcs11(
            RvError::SessionHandleInvalid
                | RvError::SessionClosed
                | RvError::DeviceRemoved
                | RvError::DeviceError
                | RvError::TokenNotPresent
                | RvError::UserNotLoggedIn,
            _
        )
    )
}

/// What every key of a source shares: the loaded module, the PIN source and whether the token
/// rejected the PIN (after which none is sent again).
struct Shared {
    context: Pkcs11,
    pin: Pin,
    /// Serial numbers of the tokens that rejected a PIN. Held across the whole login, so two
    /// threads cannot each send a wrong PIN.
    rejected: Mutex<BTreeSet<String>>,
}

/// One token, found again by its serial number and label when its card is reinserted.
struct Token {
    shared: Arc<Shared>,
    slot: Mutex<Slot>,
    serial: String,
    label: String,
}

impl Token {
    fn slot(&self) -> Slot {
        *self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads the slot list again, which is what makes a module notice card events, and takes the
    /// slot that now holds this token.
    fn relocate(&self) -> Result<(), Error> {
        let context = &self.shared.context;
        for slot in context.get_slots_with_token().map_err(|e| failure(&e))? {
            let Ok(info) = context.get_token_info(slot) else {
                continue;
            };
            if info.serial_number().trim() == self.serial && info.label().trim() == self.label {
                *self
                    .slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = slot;
                return Ok(());
            }
        }
        Err(Error::Keystore {
            code: None,
            reason: Reason::new("the token is not present"),
        })
    }

    fn needs_login(&self) -> Result<bool, Error> {
        Ok(self
            .shared
            .context
            .get_token_info(self.slot())
            .map_err(|e| failure(&e))?
            .login_required())
    }

    /// Logs in for `purpose`; `true` once logged in. The PIN source is asked only when the
    /// application is not logged in yet, or for every per-signature login. A PIN the token
    /// rejected is never sent again.
    ///
    /// Without a PIN a listing login returns `false`; any other fails at once. The PIN pad is
    /// used only when the source says [`PinAnswer::Keystore`].
    fn log_in(
        &self,
        session: &Session,
        purpose: PinPurpose,
        certificate: Option<&CertificateInfo>,
    ) -> Result<bool, Error> {
        let per_signature = purpose == PinPurpose::PerSignature;
        if !per_signature
            && session
                .get_session_info()
                .map(|info| {
                    matches!(
                        info.session_state(),
                        SessionState::RoUser | SessionState::RwUser
                    )
                })
                .unwrap_or(false)
        {
            return Ok(true);
        }
        let mut rejected = self
            .shared
            .rejected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if rejected.contains(&self.serial) {
            return Err(Error::WrongPin);
        }
        let pin_pad = self
            .shared
            .context
            .get_token_info(self.slot())
            .map_err(|e| failure(&e))?
            .protected_authentication_path();
        let request = PinRequest::new(
            KeystoreKind::Pkcs11,
            self.label.clone(),
            certificate.cloned(),
            purpose,
            pin_pad,
        );
        let denied = |why: &'static str| {
            if purpose == PinPurpose::List {
                Ok(false)
            } else {
                Err(Error::Keystore {
                    code: None,
                    reason: Reason::new(why),
                })
            }
        };
        let pin = match self.shared.pin.resolve(&request) {
            PinAnswer::Decline => return denied("the token needs a PIN and none was given"),
            PinAnswer::Keystore if !pin_pad => {
                return denied("the token has no PIN pad, so the keystore cannot ask");
            }
            PinAnswer::Keystore => None,
            PinAnswer::Secret(secret) => Some(AuthPin::new(secret.expose().into())),
        };
        let user = if per_signature {
            UserType::ContextSpecific
        } else {
            UserType::User
        };
        match session.login(user, pin.as_ref()) {
            Ok(()) | Err(CryptokiError::Pkcs11(RvError::UserAlreadyLoggedIn, _)) => Ok(true),
            Err(CryptokiError::Pkcs11(RvError::PinIncorrect, _)) => {
                rejected.insert(self.serial.clone());
                Err(Error::WrongPin)
            }
            Err(error) => Err(failure(&error)),
        }
    }
}

/// The certificates of a PKCS #11 module's tokens, with the private keys that go with them.
///
/// Listing opens a read-only session on each token and logs in as the source's [`Pin`] says (a
/// listing login that is declined only hides the keys that need it). The key itself never leaves
/// the token: each signature is made by the module, logging in again for a key that demands the
/// PIN every time (`CKA_ALWAYS_AUTHENTICATE`) and finding the token again if its card was removed
/// and reinserted. A PIN the token rejects is never sent again.
pub struct Pkcs11Source {
    module: PathBuf,
    pin: Pin,
    token: Option<String>,
    token_label: Option<String>,
    token_serial: Option<String>,
    slot: Option<u64>,
    shared: Mutex<Option<Arc<Shared>>>,
}

impl fmt::Debug for Pkcs11Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Source")
            .field("module", &self.module)
            .field("pin", &self.pin)
            .field("token_label", &self.token_label)
            .field("token_serial", &self.token_serial)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

/// Builds a [`Pkcs11Source`].
#[derive(Debug)]
#[must_use]
pub struct Pkcs11SourceBuilder {
    source: Pkcs11Source,
}

impl Pkcs11SourceBuilder {
    /// Where the user PIN comes from, to log in to each token: a [`Secret`](crate::Secret) is a
    /// PIN; [`Pin::Keystore`] uses a reader's PIN pad.
    pub fn pin(mut self, pin: impl Into<Pin>) -> Self {
        self.source.pin = pin.into();
        self
    }

    /// Only the token with this label.
    pub fn token_label(mut self, label: impl Into<String>) -> Self {
        self.source.token_label = Some(label.into());
        self
    }

    /// Only the token with this serial number.
    pub fn token_serial(mut self, serial: impl Into<String>) -> Self {
        self.source.token_serial = Some(serial.into());
        self
    }

    /// Only the token with this label or serial number.
    pub fn token(mut self, label_or_serial: impl Into<String>) -> Self {
        self.source.token = Some(label_or_serial.into());
        self
    }

    /// Only this slot.
    pub fn slot(mut self, slot: u64) -> Self {
        self.source.slot = Some(slot);
        self
    }

    /// The source. The module is loaded at the first listing.
    pub fn build(self) -> Pkcs11Source {
        self.source
    }
}

impl Pkcs11Source {
    /// Starts a source over the PKCS #11 module at `module`.
    pub fn builder(module: impl Into<PathBuf>) -> Pkcs11SourceBuilder {
        Pkcs11SourceBuilder {
            source: Self {
                module: module.into(),
                pin: Pin::None,
                token: None,
                token_label: None,
                token_serial: None,
                slot: None,
                shared: Mutex::new(None),
            },
        }
    }

    /// The first of `settings`' module candidates that exists, as a builder.
    pub fn discover(settings: &Pkcs11Settings) -> Option<Pkcs11SourceBuilder> {
        settings
            .module_candidates
            .iter()
            .find(|path| path.exists())
            .map(|path| Self::builder(path.clone()))
    }

    /// The system's default module (p11-kit's proxy, else OpenSC's), when one is installed.
    pub fn system_default() -> Option<Self> {
        Self::discover(&Pkcs11Settings::default()).map(Pkcs11SourceBuilder::build)
    }

    fn shared(&self) -> Result<Arc<Shared>, Error> {
        let mut slot = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(shared) = slot.as_ref() {
            return Ok(Arc::clone(shared));
        }
        let context = Pkcs11::new(&self.module).map_err(|e| failure(&e))?;
        match context.initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK)) {
            Ok(()) | Err(CryptokiError::Pkcs11(RvError::CryptokiAlreadyInitialized, _)) => {}
            Err(error) => return Err(failure(&error)),
        }
        let shared = Arc::new(Shared {
            context,
            pin: self.pin.clone(),
            rejected: Mutex::new(BTreeSet::new()),
        });
        *slot = Some(Arc::clone(&shared));
        Ok(shared)
    }

    /// The tokens present that pass the source's filters.
    fn tokens_present(&self, shared: &Arc<Shared>) -> Result<Vec<Token>, Error> {
        let mut found = Vec::new();
        for slot in shared
            .context
            .get_slots_with_token()
            .map_err(|e| failure(&e))?
        {
            if self.slot.is_some_and(|wanted| wanted != slot.id()) {
                continue;
            }
            let info = shared
                .context
                .get_token_info(slot)
                .map_err(|e| failure(&e))?;
            if !info.token_initialized() {
                continue;
            }
            let (label, serial) = (info.label().trim(), info.serial_number().trim());
            let wanted = |filter: &Option<String>, value: &str| {
                filter.as_deref().is_none_or(|filter| filter == value)
            };
            if !wanted(&self.token_label, label)
                || !wanted(&self.token_serial, serial)
                || self
                    .token
                    .as_deref()
                    .is_some_and(|filter| filter != label && filter != serial)
            {
                continue;
            }
            found.push(Token {
                shared: Arc::clone(shared),
                slot: Mutex::new(slot),
                serial: serial.to_string(),
                label: label.to_string(),
            });
        }
        Ok(found)
    }

    /// The labels of the tokens present, after the token and slot filters.
    ///
    /// # Errors
    ///
    /// [`Error::Keystore`] when the module cannot be loaded or read.
    pub fn tokens(&self) -> Result<Vec<String>, Error> {
        let shared = self.shared()?;
        Ok(self
            .tokens_present(&shared)?
            .into_iter()
            .map(|token| token.label)
            .collect())
    }
}

impl IdentitySource for Pkcs11Source {
    fn identities(&self) -> Result<Vec<Identity>, Error> {
        let shared = self.shared()?;
        let mut found = Vec::new();
        for token in self.tokens_present(&shared)? {
            found.extend(token_identities(Arc::new(token))?);
        }
        Ok(found)
    }

    fn close(&self) {
        // Dropping the shared context finalizes the module once every key is gone.
        *self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

fn token_identities(token: Arc<Token>) -> Result<Vec<Identity>, Error> {
    let context = &token.shared.context;
    let session = context
        .open_ro_session(token.slot())
        .map_err(|e| failure(&e))?;
    let needs_login = token.needs_login()?;
    let logged_in = needs_login && token.log_in(&session, PinPurpose::List, None)?;
    // Every certificate of the token: the leaves, and the CAs a chain is built from.
    let mut certificates: Vec<(Vec<u8>, Vec<u8>, String)> = Vec::new();
    let handles = session
        .find_objects(&[
            Attribute::Class(ObjectClass::CERTIFICATE),
            Attribute::CertificateType(CertificateType::X_509),
        ])
        .map_err(|e| failure(&e))?;
    for handle in handles {
        let Ok(attributes) = session.get_attributes(
            handle,
            &[
                AttributeType::Value,
                AttributeType::Id,
                AttributeType::Label,
            ],
        ) else {
            continue;
        };
        let (mut value, mut id, mut label) = (None, Vec::new(), String::new());
        for attribute in attributes {
            match attribute {
                Attribute::Value(bytes) => value = Some(bytes),
                Attribute::Id(bytes) => id = bytes,
                Attribute::Label(bytes) => label = String::from_utf8_lossy(&bytes).into_owned(),
                _ => {}
            }
        }
        if let Some(value) = value {
            certificates.push((value, id, label));
        }
    }
    let mut key_ids: Vec<Vec<u8>> = Vec::new();
    for handle in session
        .find_objects(&[Attribute::Class(ObjectClass::PRIVATE_KEY)])
        .map_err(|e| failure(&e))?
    {
        if let Ok(attributes) = session.get_attributes(handle, &[AttributeType::Id])
            && let Some(Attribute::Id(id)) = attributes.into_iter().next()
        {
            key_ids.push(id);
        }
    }
    // Without a login the private keys may be hidden: a certificate with an id is then assumed to
    // have its key, which the first signature checks.
    let unseen_keys_possible = needs_login && !logged_in;
    let pool: Vec<&[u8]> = certificates
        .iter()
        .map(|(der, _, _)| der.as_slice())
        .collect();
    let mut identities = Vec::new();
    for (der, id, label) in &certificates {
        if id.is_empty() || (!key_ids.contains(id) && !unseen_keys_possible) {
            continue;
        }
        let chain = build_chain(der, &pool);
        let Ok(info) = CertificateInfo::from_chain(&chain) else {
            continue;
        };
        let name = if label.is_empty() {
            alloc::format!("pkcs11:{}", token.label)
        } else {
            label.clone()
        };
        let key = Arc::new(Pkcs11Key {
            inner: Arc::new(KeyInner {
                token: Arc::clone(&token),
                key_id: id.clone(),
                info,
                session: Mutex::new(None),
            }),
        });
        if let Ok(identity) = Identity::new(chain, key, name) {
            identities.push(identity);
        }
    }
    Ok(identities)
}

/// `leaf` followed by the certificates of `all` that issued it, one above the other.
fn build_chain(leaf: &[u8], all: &[&[u8]]) -> Vec<CertificateDer<'static>> {
    let mut chain = alloc::vec![CertificateDer::from(leaf.to_vec())];
    let mut current = leaf.to_vec();
    for _ in 0..8 {
        let Ok(certificate) = Certificate::from_der(&current) else {
            break;
        };
        let (Ok(issuer), Ok(subject)) = (
            certificate.tbs_certificate().issuer().to_der(),
            certificate.tbs_certificate().subject().to_der(),
        ) else {
            break;
        };
        if issuer == subject {
            break;
        }
        let parent = all.iter().find(|candidate| {
            Certificate::from_der(candidate)
                .ok()
                .and_then(|parent| parent.tbs_certificate().subject().to_der().ok())
                .is_some_and(|name| name == issuer)
        });
        let Some(parent) = parent else { break };
        chain.push(CertificateDer::from(parent.to_vec()));
        current = parent.to_vec();
    }
    chain
}

/// An open session on a key's token, with the key found in it.
struct KeySession {
    session: Session,
    handle: ObjectHandle,
    always_authenticate: bool,
}

/// A private key on a token, signing through the module's session. Every signer made from it
/// shares its session.
struct Pkcs11Key {
    inner: Arc<KeyInner>,
}

struct KeyInner {
    token: Arc<Token>,
    key_id: Vec<u8>,
    info: CertificateInfo,
    session: Mutex<Option<KeySession>>,
}

impl fmt::Debug for Pkcs11Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Key")
            .field("key_type", &self.inner.info.key_type())
            .finish_non_exhaustive()
    }
}

impl KeyInner {
    fn open(&self) -> Result<KeySession, (Error, bool)> {
        let wrap = |error: CryptokiError| (failure(&error), lost(&error));
        let session = self
            .token
            .shared
            .context
            .open_ro_session(self.token.slot())
            .map_err(wrap)?;
        if self.token.needs_login().map_err(|error| (error, false))? {
            self.token
                .log_in(&session, PinPurpose::Sign, Some(&self.info))
                .map_err(|error| (error, false))?;
        }
        let handles = session
            .find_objects(&[
                Attribute::Class(ObjectClass::PRIVATE_KEY),
                Attribute::Id(self.key_id.clone()),
            ])
            .map_err(wrap)?;
        let handle = handles.first().copied().ok_or_else(|| {
            (
                Error::Keystore {
                    code: None,
                    reason: Reason::new("the token holds no private key for the certificate"),
                },
                false,
            )
        })?;
        let always_authenticate = session
            .get_attributes(handle, &[AttributeType::AlwaysAuthenticate])
            .ok()
            .and_then(|attributes| attributes.into_iter().next())
            .is_some_and(|attribute| matches!(attribute, Attribute::AlwaysAuthenticate(true)));
        Ok(KeySession {
            session,
            handle,
            always_authenticate,
        })
    }

    fn sign_once(
        &self,
        guard: &mut Option<KeySession>,
        scheme: SignatureScheme,
        message: &[u8],
    ) -> Result<Vec<u8>, (Error, bool)> {
        if guard.is_none() {
            *guard = Some(self.open()?);
        }
        let Some(open) = guard.as_ref() else {
            unreachable!("the session was just opened");
        };
        let wrap = |error: CryptokiError| (failure(&error), lost(&error));
        let multipart = open.always_authenticate;
        let (mechanism, data, ecdsa) = prepare(scheme, message, multipart).ok_or_else(|| {
            (
                Error::Signing(Reason::new("a signature scheme this token cannot make")),
                false,
            )
        })?;
        let signature = if multipart {
            // A key that demands the PIN for every signature is logged in to between the start
            // of the operation and its end, so the operation is multi-part.
            open.session
                .sign_init(&mechanism, open.handle)
                .map_err(wrap)?;
            self.token
                .log_in(&open.session, PinPurpose::PerSignature, Some(&self.info))
                .map_err(|error| (error, false))?;
            open.session.sign_update(&data).map_err(wrap)?;
            open.session.sign_final().map_err(wrap)?
        } else {
            open.session
                .sign(&mechanism, open.handle, &data)
                .map_err(wrap)?
        };
        if ecdsa {
            p1363_to_der(&signature).ok_or_else(|| {
                (
                    Error::Signing(Reason::new("the token's signature is malformed")),
                    false,
                )
            })
        } else {
            Ok(signature)
        }
    }

    fn sign(&self, scheme: SignatureScheme, message: &[u8]) -> Result<Vec<u8>, Error> {
        let mut guard = self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for attempt in 0..2 {
            match self.sign_once(&mut guard, scheme, message) {
                Ok(signature) => return Ok(signature),
                Err((error, session_lost)) => {
                    // A failed signature leaves the session in an unknown state (an operation may
                    // still be active): drop it.
                    *guard = None;
                    if session_lost {
                        // A reinserted card invalidates sessions and handles: find the token
                        // again and reopen once.
                        if attempt == 0 && self.token.relocate().is_ok() {
                            continue;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Err(Error::Signing(Reason::new(
            "the token could not be reached",
        )))
    }
}

impl SigningKey for Pkcs11Key {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        let scheme = self
            .inner
            .info
            .key_type()
            .schemes()
            .iter()
            .copied()
            .find(|scheme| offered.contains(scheme) && supported(*scheme))?;
        Some(Box::new(Pkcs11Signer {
            key: Arc::clone(&self.inner),
            scheme,
        }))
    }

    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        self.inner.info.key_type().algorithm()
    }
}

fn supported(scheme: SignatureScheme) -> bool {
    !matches!(scheme, SignatureScheme::ED25519)
}

struct Pkcs11Signer {
    key: Arc<KeyInner>,
    scheme: SignatureScheme,
}

impl fmt::Debug for Pkcs11Signer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pkcs11Signer")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

/// The mechanism, the data it takes and whether the answer is a raw ECDSA signature, for `scheme`.
/// A multi-part operation takes the message itself, since it cannot take a digest for `CKM_ECDSA`.
fn prepare(
    scheme: SignatureScheme,
    message: &[u8],
    multipart: bool,
) -> Option<(Mechanism<'static>, Vec<u8>, bool)> {
    let pss = |hash: MechanismType, mgf: PkcsMgfType, bytes: u8| PkcsPssParams {
        hash_alg: hash,
        mgf,
        s_len: Ulong::new(bytes.into()),
    };
    let message_vec = message.to_vec();
    Some(match scheme {
        SignatureScheme::RSA_PKCS1_SHA256 => (Mechanism::Sha256RsaPkcs, message_vec, false),
        SignatureScheme::RSA_PKCS1_SHA384 => (Mechanism::Sha384RsaPkcs, message_vec, false),
        SignatureScheme::RSA_PKCS1_SHA512 => (Mechanism::Sha512RsaPkcs, message_vec, false),
        SignatureScheme::RSA_PSS_SHA256 => (
            Mechanism::Sha256RsaPkcsPss(pss(MechanismType::SHA256, PkcsMgfType::MGF1_SHA256, 32)),
            message_vec,
            false,
        ),
        SignatureScheme::RSA_PSS_SHA384 => (
            Mechanism::Sha384RsaPkcsPss(pss(MechanismType::SHA384, PkcsMgfType::MGF1_SHA384, 48)),
            message_vec,
            false,
        ),
        SignatureScheme::RSA_PSS_SHA512 => (
            Mechanism::Sha512RsaPkcsPss(pss(MechanismType::SHA512, PkcsMgfType::MGF1_SHA512, 64)),
            message_vec,
            false,
        ),
        SignatureScheme::ECDSA_NISTP256_SHA256 if multipart => {
            (Mechanism::EcdsaSha256, message_vec, true)
        }
        SignatureScheme::ECDSA_NISTP384_SHA384 if multipart => {
            (Mechanism::EcdsaSha384, message_vec, true)
        }
        SignatureScheme::ECDSA_NISTP521_SHA512 if multipart => {
            (Mechanism::EcdsaSha512, message_vec, true)
        }
        SignatureScheme::ECDSA_NISTP256_SHA256 => {
            (Mechanism::Ecdsa, Sha256::digest(message).to_vec(), true)
        }
        SignatureScheme::ECDSA_NISTP384_SHA384 => {
            (Mechanism::Ecdsa, Sha384::digest(message).to_vec(), true)
        }
        SignatureScheme::ECDSA_NISTP521_SHA512 => {
            (Mechanism::Ecdsa, Sha512::digest(message).to_vec(), true)
        }
        _ => return None,
    })
}

impl Signer for Pkcs11Signer {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        self.key.sign(self.scheme, message).map_err(|error| {
            rustls::Error::General(alloc::format!("the token did not sign: {error}"))
        })
    }

    fn scheme(&self) -> SignatureScheme {
        self.scheme
    }
}

/// An ECDSA signature as the token makes it, `r || s`, in the DER `SEQUENCE` TLS carries.
fn p1363_to_der(signature: &[u8]) -> Option<Vec<u8>> {
    if signature.is_empty() || !signature.len().is_multiple_of(2) {
        return None;
    }
    let (r, s) = signature.split_at(signature.len() / 2);
    fn integer(bytes: &[u8]) -> Vec<u8> {
        let trimmed = &bytes[bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len() - 1)..];
        let mut body = Vec::with_capacity(trimmed.len() + 1);
        if trimmed[0] & 0x80 != 0 {
            body.push(0);
        }
        body.extend_from_slice(trimmed);
        let mut out = alloc::vec![0x02];
        out.extend(length(body.len()));
        out.extend(body);
        out
    }
    fn length(length: usize) -> Vec<u8> {
        if length < 0x80 {
            alloc::vec![length as u8]
        } else {
            alloc::vec![0x81, length as u8]
        }
    }
    let mut body = integer(r);
    body.extend(integer(s));
    let mut out = alloc::vec![0x30];
    out.extend(length(body.len()));
    out.extend(body);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Secret;

    #[test]
    fn a_raw_ecdsa_signature_becomes_a_der_sequence_of_two_integers() {
        // r = 0x01..., s = 0x80... (needs a leading zero), both 4 bytes.
        let der = p1363_to_der(&[0x01, 0x02, 0x03, 0x04, 0x80, 0x00, 0x00, 0x01]).unwrap();
        assert_eq!(
            der,
            [
                0x30, 0x0d, 0x02, 0x04, 0x01, 0x02, 0x03, 0x04, 0x02, 0x05, 0x00, 0x80, 0x00, 0x00,
                0x01
            ]
        );
    }

    #[test]
    fn leading_zeros_of_an_integer_are_trimmed_and_an_odd_length_is_refused() {
        let der = p1363_to_der(&[0x00, 0x00, 0x05, 0x00, 0x00, 0x07]).unwrap();
        assert_eq!(der, [0x30, 0x06, 0x02, 0x01, 0x05, 0x02, 0x01, 0x07]);
        assert!(p1363_to_der(&[1, 2, 3]).is_none());
        assert!(p1363_to_der(&[]).is_none());
    }

    #[test]
    fn a_module_that_does_not_exist_is_an_error_not_a_panic() {
        let source = Pkcs11Source::builder("/no/such/module.so").build();
        assert!(source.identities().is_err());
        assert!(source.tokens().is_err());
    }

    #[test]
    fn the_secret_token_pin_is_not_in_debug() {
        let source = Pkcs11Source::builder("/m.so")
            .pin(Secret::new("1234"))
            .build();
        assert!(!alloc::format!("{source:?}").contains("1234"));
    }

    #[test]
    fn an_always_authenticate_key_signs_ecdsa_over_the_message_not_a_digest() {
        let message = b"handshake transcript";
        let (_, data, ecdsa) =
            prepare(SignatureScheme::ECDSA_NISTP256_SHA256, message, true).unwrap();
        assert_eq!(data, message);
        assert!(ecdsa);
        let (_, digest, _) =
            prepare(SignatureScheme::ECDSA_NISTP256_SHA256, message, false).unwrap();
        assert_eq!(digest.len(), 32);
        assert!(prepare(SignatureScheme::ED25519, message, false).is_none());
    }
}
