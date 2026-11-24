//! How a PIN reaches a keystore: a value, a callback, the keystore's own prompt, or nothing.
//!
//! With [`Pin::None`], an operation that needs a PIN fails at once and nothing waits on a person.
//! A keystore's own prompt (a Windows dialog, a PIN pad) is used for [`Pin::Keystore`] or a callback
//! answering [`PinAnswer::Keystore`]; [`CngSource`](crate::keystore) and
//! [`TlsBuilder`](crate::TlsBuilder) default to `Pin::Keystore`, as a browser does, while
//! [`Pkcs11Source`](crate::keystore) and the keychain default to `Pin::None`. A PIN the keystore
//! rejected is not sent again.

use alloc::string::String;
use alloc::sync::Arc;
use core::fmt;

use crate::identity::CertificateInfo;
use crate::secret::Secret;

/// Which kind of keystore is asking for a PIN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeystoreKind {
    /// Windows CNG (a certificate store and its key storage providers).
    Cng,
    /// A PKCS#11 module.
    Pkcs11,
    /// The macOS Keychain.
    Keychain,
}

impl KeystoreKind {
    /// The name used in diagnostics: `cng`, `pkcs11` or `keychain`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cng => "cng",
            Self::Pkcs11 => "pkcs11",
            Self::Keychain => "keychain",
        }
    }
}

impl fmt::Display for KeystoreKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a PIN is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PinPurpose {
    /// Logging in to see private keys; declining only hides them.
    List,
    /// Logging in before a signature; declining fails it.
    Sign,
    /// The key demands the PIN for every signature (`CKA_ALWAYS_AUTHENTICATE`, a PIV card's
    /// signature key).
    PerSignature,
}

impl PinPurpose {
    /// `list`, `sign` or `per-signature`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Sign => "sign",
            Self::PerSignature => "per-signature",
        }
    }
}

/// What a PIN callback is told. It never contains a PIN.
#[derive(Debug, Clone)]
pub struct PinRequest {
    source: KeystoreKind,
    token: String,
    certificate: Option<CertificateInfo>,
    purpose: PinPurpose,
    keystore_prompt: bool,
}

impl PinRequest {
    /// A request from `source` for the token or key container `token`.
    pub fn new(
        source: KeystoreKind,
        token: impl Into<String>,
        certificate: Option<CertificateInfo>,
        purpose: PinPurpose,
        keystore_prompt: bool,
    ) -> Self {
        Self {
            source,
            token: token.into(),
            certificate,
            purpose,
            keystore_prompt,
        }
    }

    /// The kind of keystore asking.
    pub fn source(&self) -> KeystoreKind {
        self.source
    }

    /// The token's label (PKCS#11), or the store the key is in (CNG).
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The certificate whose key needs the PIN; `None` when the request is about a whole token.
    pub fn certificate(&self) -> Option<&CertificateInfo> {
        self.certificate.as_ref()
    }

    /// Why the PIN is wanted.
    pub fn purpose(&self) -> PinPurpose {
        self.purpose
    }

    /// Whether answering [`PinAnswer::Keystore`] can work here (the token has a PIN pad, or the
    /// system can show its dialog).
    pub fn keystore_prompt(&self) -> bool {
        self.keystore_prompt
    }
}

/// What a PIN callback answers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PinAnswer {
    /// This PIN.
    Secret(Secret),
    /// Let the keystore ask the person itself.
    Keystore,
    /// No PIN: the operation fails (or the key stays hidden).
    Decline,
}

impl From<Secret> for PinAnswer {
    fn from(secret: Secret) -> Self {
        Self::Secret(secret)
    }
}

impl From<Option<Secret>> for PinAnswer {
    fn from(secret: Option<Secret>) -> Self {
        secret.map_or(Self::Decline, Self::Secret)
    }
}

/// A function that answers a [`PinRequest`]. It is called each time a PIN is wanted and decides
/// whether to ask a person, cache or decline.
pub type PinCallback = Arc<dyn Fn(&PinRequest) -> PinAnswer + Send + Sync>;

/// Where a keystore gets its PIN.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum Pin {
    /// No PIN: an operation that needs one fails at once.
    #[default]
    None,
    /// This PIN, for every request.
    Secret(Secret),
    /// The keystore's own prompt: Windows' PIN dialog, a reader's PIN pad, the Keychain's sheet.
    /// It waits for a person.
    Keystore,
    /// Ask this function each time.
    Callback(PinCallback),
}

impl Pin {
    /// A PIN callback from a closure.
    pub fn callback(answer: impl Fn(&PinRequest) -> PinAnswer + Send + Sync + 'static) -> Self {
        Self::Callback(Arc::new(answer))
    }

    /// What this source answers to `request`.
    pub fn resolve(&self, request: &PinRequest) -> PinAnswer {
        match self {
            Self::None => PinAnswer::Decline,
            Self::Secret(secret) => PinAnswer::Secret(secret.clone()),
            Self::Keystore => PinAnswer::Keystore,
            Self::Callback(callback) => callback(request),
        }
    }

    /// Whether there is nothing to ask: no PIN will ever be given.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl fmt::Debug for Pin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("Pin::None"),
            Self::Secret(_) => formatter.write_str("Pin::Secret(***)"),
            Self::Keystore => formatter.write_str("Pin::Keystore"),
            Self::Callback(_) => formatter.write_str("Pin::Callback(..)"),
        }
    }
}

impl From<Secret> for Pin {
    fn from(secret: Secret) -> Self {
        Self::Secret(secret)
    }
}

impl From<&str> for Pin {
    fn from(pin: &str) -> Self {
        Self::Secret(Secret::new(pin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PinRequest {
        PinRequest::new(KeystoreKind::Pkcs11, "token", None, PinPurpose::Sign, true)
    }

    #[test]
    fn a_pin_never_shows_in_debug_or_display() {
        let pin = Pin::Secret(Secret::new("123456"));
        assert!(!alloc::format!("{pin:?}").contains("123456"));
        let secret = Secret::new("123456");
        assert_eq!(alloc::format!("{secret} {secret:?}"), "*** ***");
        assert!(!alloc::format!("{:?}", PinAnswer::Secret(secret)).contains("123456"));
    }

    #[test]
    fn each_pin_source_answers_as_documented() {
        assert!(matches!(Pin::None.resolve(&request()), PinAnswer::Decline));
        assert!(matches!(
            Pin::Keystore.resolve(&request()),
            PinAnswer::Keystore
        ));
        match Pin::from("1234").resolve(&request()) {
            PinAnswer::Secret(secret) => assert_eq!(secret.expose(), "1234"),
            other => panic!("{other:?}"),
        }
        let asked = Pin::callback(|request| {
            assert_eq!(request.token(), "token");
            assert_eq!(request.purpose(), PinPurpose::Sign);
            Some(Secret::new("9")).into()
        });
        assert!(matches!(asked.resolve(&request()), PinAnswer::Secret(_)));
        let declined = Pin::callback(|_| None::<Secret>.into());
        assert!(matches!(declined.resolve(&request()), PinAnswer::Decline));
    }
}
