//! What the tables keep.
//!
//! The records derive guatiao's `ToValue`, `FromValue` and `Schema`, so any table keeps them in any
//! format. What the WebAuthn library needs to remember (a credential's public key, a ceremony's
//! challenge) is kept as that library's own JSON text in a field called `key` or `state`: opaque,
//! never edited by hand, and no type of the library is public here.

use std::borrow::Cow;
use std::fmt;

use coqui_access::guatiao::{FromValue, Schema, ToValue};
use coqui_access_kit::Keyed;

/// One account and its passkeys, keyed by [`name`](PasskeyAccount::name).
///
/// `Debug` shows counts, never a name, an id or a key.
#[derive(Clone, PartialEq, Eq, FromValue, Schema, ToValue)]
#[map(crate = ::coqui_access::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct PasskeyAccount {
    /// The account key: unique in the table, and what a sign-in names.
    pub name: String,
    /// The stable identity, 32 lowercase hex digits (16 random bytes). Those bytes are the
    /// WebAuthn user handle, so the id is what a discoverable sign-in names when it names nobody.
    pub id: String,
    /// What the person is called in the browser's account chooser; the name when absent.
    pub display_name: Option<String>,
    /// Groups the account is in.
    #[map(default, skip_serializing_if = Vec::is_empty)]
    pub groups: Vec<String>,
    /// A disabled account signs nobody in.
    #[map(default, skip_serializing_if = core::ops::Not::not)]
    pub disabled: bool,
    /// When it was made, in seconds since the Unix epoch.
    pub made_unix: Option<i64>,
    /// Its passkeys.
    #[map(default, skip_serializing_if = Vec::is_empty)]
    pub credentials: Vec<StoredCredential>,
}

impl Keyed for PasskeyAccount {
    fn key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.name)
    }
}

impl fmt::Debug for PasskeyAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PasskeyAccount")
            .field("name", &"[REDACTED]")
            .field("id", &"[REDACTED]")
            .field("groups", &self.groups.len())
            .field("disabled", &self.disabled)
            .field("credentials", &self.credentials.len())
            .finish()
    }
}

/// One passkey of an account.
///
/// `Debug` shows the flags only.
#[derive(Clone, PartialEq, Eq, FromValue, Schema, ToValue)]
#[map(crate = ::coqui_access::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct StoredCredential {
    /// The credential id as the authenticator made it, unpadded base64url.
    pub id: String,
    /// What the person calls it.
    pub label: Option<String>,
    /// When it was registered, in seconds since the Unix epoch.
    pub made_unix: i64,
    /// When it last signed in, in seconds since the Unix epoch.
    pub used_unix: Option<i64>,
    /// A disabled credential signs nobody in.
    #[map(default, skip_serializing_if = core::ops::Not::not)]
    pub disabled: bool,
    /// The credential as the WebAuthn library keeps it (its public key, its counter), as JSON
    /// text. Not a secret, and not to be edited.
    pub key: String,
}

impl fmt::Debug for StoredCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredCredential")
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

/// A registration begun and not confirmed, keyed by [`name`](PendingRegistration::name).
///
/// `Debug` shows the begin time only.
#[derive(Clone, PartialEq, Eq, FromValue, Schema, ToValue)]
#[map(crate = ::coqui_access::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct PendingRegistration {
    /// The account it is for.
    pub name: String,
    /// The id the account has, or will have once the registration is confirmed.
    pub id: String,
    /// The display name given when it began.
    pub display_name: Option<String>,
    /// The challenge and what was asked, as the WebAuthn library keeps them, as JSON text.
    pub state: String,
    /// When it began, in seconds since the Unix epoch.
    pub created_unix: i64,
}

impl Keyed for PendingRegistration {
    fn key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.name)
    }
}

impl fmt::Debug for PendingRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingRegistration")
            .field("created_unix", &self.created_unix)
            .finish_non_exhaustive()
    }
}

/// A sign-in challenge issued and not answered, keyed by [`challenge`](Ceremony::challenge).
///
/// `Debug` shows the issue time only.
#[derive(Clone, PartialEq, Eq, FromValue, Schema, ToValue)]
#[map(crate = ::coqui_access::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Ceremony {
    /// The challenge, unpadded base64url: what the consumer keeps and returns.
    pub challenge: String,
    /// The account an earlier step named, when one did.
    pub name: Option<String>,
    /// The address the challenge was asked from, which the per-source bound counts.
    pub source: Option<String>,
    /// When it was issued, in seconds since the Unix epoch.
    pub created_unix: i64,
    /// What was asked, as the WebAuthn library keeps it, as JSON text.
    pub state: String,
}

impl Keyed for Ceremony {
    fn key(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.challenge)
    }
}

impl fmt::Debug for Ceremony {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Ceremony")
            .field("created_unix", &self.created_unix)
            .finish_non_exhaustive()
    }
}
