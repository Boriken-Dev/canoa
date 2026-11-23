//! The words a passkey provider's settings take, each a `coqui_builder::Leaf` text.

use coqui_builder::Leaf;
use webauthn_rs_core::proto::{
    AttestationConveyancePreference, COSEAlgorithm, UserVerificationPolicy,
};

/// Whether the person must be verified (a PIN, a fingerprint, a face) when a passkey is used.
///
/// With `required`, the default, a passkey alone proves who the person is and that the person is
/// there, which is two factors in one: a stolen device does not sign in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Leaf)]
#[map(rename_all = "lowercase")]
pub enum UserVerification {
    /// Refuse a registration and a sign-in that did not verify the person. The default.
    #[default]
    Required,
    /// Ask for it, and accept an answer without. A passkey is then only something the person
    /// has: a lost device signs in.
    Preferred,
    /// Do not ask. A passkey is then only something the person has.
    Discouraged,
}

impl UserVerification {
    /// The word a setting uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Preferred => "preferred",
            Self::Discouraged => "discouraged",
        }
    }

    pub(crate) fn policy(self) -> UserVerificationPolicy {
        match self {
            Self::Required => UserVerificationPolicy::Required,
            Self::Preferred => UserVerificationPolicy::Preferred,
            Self::Discouraged => UserVerificationPolicy::Discouraged_DO_NOT_USE,
        }
    }
}

/// Whether a registration asks for a discoverable credential: one the device can offer without
/// the person typing a name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Leaf)]
#[map(rename_all = "lowercase")]
pub enum ResidentKey {
    /// Ask for one (`residentKey: required`), so every passkey signs in with no name typed.
    /// The default.
    #[default]
    Required,
    /// Do not ask (`residentKey: discouraged`): the person names the account first.
    Discouraged,
}

impl ResidentKey {
    /// The word a setting uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Discouraged => "discouraged",
        }
    }
}

/// What a registration asks the authenticator to say about itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Leaf)]
#[map(rename_all = "lowercase")]
pub enum Attestation {
    /// Ask for none. The default; it keeps the model of the device private.
    #[default]
    None,
    /// Ask for it in a form the client may anonymise.
    Indirect,
    /// Ask for the authenticator's own statement.
    Direct,
}

impl Attestation {
    /// The word a setting uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Indirect => "indirect",
            Self::Direct => "direct",
        }
    }

    pub(crate) fn preference(self) -> AttestationConveyancePreference {
        match self {
            Self::None => AttestationConveyancePreference::None,
            Self::Indirect => AttestationConveyancePreference::Indirect,
            Self::Direct => AttestationConveyancePreference::Direct,
        }
    }
}

/// A signature algorithm a registration may offer.
///
/// `eddsa` is a word so that a configuration that names it is told why it cannot be used: the
/// verifier (`webauthn-rs-core`) does not check EdDSA signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Leaf)]
#[map(rename_all = "lowercase")]
pub enum Algorithm {
    /// ECDSA over P-256 with SHA-256.
    Es256,
    /// ECDSA over P-384 with SHA-384.
    Es384,
    /// RSASSA-PKCS1-v1_5 with SHA-256.
    Rs256,
    /// EdDSA. Refused when the provider is built.
    Eddsa,
}

impl Algorithm {
    /// The algorithms offered unless the builder says otherwise: the ones the verifier calls
    /// secure.
    pub const DEFAULT: [Algorithm; 3] = [Self::Es256, Self::Es384, Self::Rs256];

    /// The word a setting uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Es256 => "es256",
            Self::Es384 => "es384",
            Self::Rs256 => "rs256",
            Self::Eddsa => "eddsa",
        }
    }

    pub(crate) fn cose(self) -> Option<COSEAlgorithm> {
        match self {
            Self::Es256 => Some(COSEAlgorithm::ES256),
            Self::Es384 => Some(COSEAlgorithm::ES384),
            Self::Rs256 => Some(COSEAlgorithm::RS256),
            Self::Eddsa => None,
        }
    }
}

/// What the provider does with a credential whose signature counter did not rise.
///
/// A counter that does not rise is the sign that a copy of the key is in use. Passkeys that sync
/// between devices report a counter of 0 and are not checked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Leaf)]
#[map(rename_all = "lowercase")]
pub enum OnRegression {
    /// Refuse the answer and disable that credential until the person or an administrator
    /// removes it. The default.
    #[default]
    Disable,
    /// Refuse the answer and leave the credential working for the next one.
    Refuse,
}

impl OnRegression {
    /// The word a setting uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Refuse => "refuse",
        }
    }
}
