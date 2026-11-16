//! Who may sign in, and as whom: the contract the portal asks of its accounts.

use core::net::IpAddr;

/// The longest password a sign-in takes, in bytes.
pub const MAX_PASSWORD: usize = 1024;

/// What a sign-in came to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignIn {
    /// The account, as the system spells it.
    Admitted {
        /// The account's name.
        user: String,
        /// The account's user id: what the portal expects of whoever made the socket it dials.
        uid: u32,
        /// The operating system's groups of the account, which the sign-in policy's `subjects` rules read.
        groups: Vec<String>,
        /// The id of the provider that accepted the credential, which the policy's `providers` read.
        provider: String,
    },
    /// Not an account, a wrong password, or an account the system refuses now.
    Refused,
    /// A certificate sign-in whose request carried no certificate: nothing was guessed, so nothing counts as a failure.
    NoCertificate,
    /// The certificate is outside its validity dates, whatever else is true of it.
    Expired,
    /// Every provider asked has never heard of the certificate: it is pinned to no one.
    Unpinned,
    /// The accounts could not be asked; the reason is for the log, never for the browser.
    Failed(String),
}

/// A certificate a request carried: what its subject says of it and its SHA-256 fingerprint (`sha256:` and lower-case hex).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presented {
    /// The subject, as text. Whatever the certificate says: show it escaped.
    pub subject: String,
    /// The fingerprint of the certificate, which is what is pinned.
    pub fingerprint: String,
    /// Whether the certificate is outside its validity dates now: such a certificate signs no one in and is never pinned.
    pub expired: bool,
}

/// Whose a presented certificate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PinState {
    /// Pinned to the account asking.
    Yours,
    /// Pinned to another account.
    Another,
    /// Pinned to no one.
    Unpinned,
}

/// What a signed-in person is shown about certificates.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CertificateStatus {
    /// The certificate the request carried, with whose it is; none when the request carried none the portal can read.
    pub presented: Option<(Presented, PinState)>,
    /// Whether a certificate is pinned to the account (one pinned to a user id this name had before does not count).
    pub pinned: bool,
    /// Whether the account's pin is disabled by an administrator: it can then be neither replaced nor removed from here.
    pub disabled: bool,
}

/// Why a certificate was not pinned or unpinned.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PinError {
    /// The request carried no certificate the portal can read.
    NoCertificate,
    /// It is pinned to another account.
    Taken,
    /// The certificate is outside its validity dates.
    Expired,
    /// The account's pin is disabled by an administrator.
    Disabled,
    /// This portal pins no certificates.
    Unsupported,
    /// The store could not be read or written; the reason is for the log.
    Failed(String),
}

/// A way to sign in that the portal can offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// A name and a password, typed into the page.
    Password,
    /// A client certificate that the proxy verified and forwards in a header; only on the proxy's socket.
    Certificate,
    /// A one-time code from an authenticator app. It is checked for an account an earlier step named.
    Totp,
    /// A one-time recovery code made when the code was set up. It is checked for an account an earlier step named.
    RecoveryCode,
}

impl Method {
    /// The word `GET /portal/methods` and the sign-in form use for it.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Certificate => "certificate",
            Self::Totp => "totp",
            Self::RecoveryCode => "recovery_code",
        }
    }

    /// The method a policy's word names, if the portal has it.
    #[must_use]
    pub fn from_id(word: &str) -> Option<Self> {
        [
            Self::Password,
            Self::Certificate,
            Self::Totp,
            Self::RecoveryCode,
        ]
        .into_iter()
        .find(|method| method.id() == word)
    }

    /// Whether the method checks a code against an account an earlier step already named, so it is never offered first.
    #[must_use]
    pub const fn needs_account(self) -> bool {
        matches!(self, Self::Totp | Self::RecoveryCode)
    }
}

/// What a one-time code came to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeCheck {
    /// The code is the person's; the id is the provider that accepted it.
    Valid {
        /// The provider that accepted the code, which the sign-in policy's `providers` read.
        provider: String,
    },
    /// Not accepted: wrong, used, expired, for an account with no code set up or one the provider has disabled.
    Refused,
    /// The store could not be asked; the reason is for the log, never for the browser.
    Failed(String),
}

/// Whether a person has a one-time code set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FactorStanding {
    /// A code is set up and in use.
    Enrolled,
    /// A code is set up and an administrator disabled it.
    Disabled,
    /// An administrator reset it: none is set up, and only an administrator sets one up again.
    Reset,
    /// None is set up.
    NotEnrolled,
    /// This portal keeps no one-time codes.
    Unsupported,
    /// The store could not be read.
    Unknown,
}

/// A one-time code just begun for a person: nothing is a factor until its first code confirms it.
#[derive(Clone, PartialEq, Eq)]
#[must_use]
pub struct Enrolment {
    /// The `otpauth://` link an authenticator app reads, a secret in it.
    pub uri: String,
    /// The recovery codes, shown once.
    pub codes: Vec<String>,
}

impl core::fmt::Debug for Enrolment {
    /// Nothing of the link or the codes: they are the factor.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Enrolment")
            .field("codes", &self.codes.len())
            .finish_non_exhaustive()
    }
}

/// Why a one-time code was not begun, confirmed or reset.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FactorError {
    /// This portal keeps no one-time codes.
    Unsupported,
    /// No setup was waiting for a code, or it timed out.
    NothingWaiting,
    /// The code did not confirm the setup.
    Wrong,
    /// The store could not be read or written; the reason is for the log.
    Failed(String),
}

/// Whoever says if a credential is an account.
pub trait Accounts: Send + Sync {
    /// The ways in this portal accepts. The default is a password alone.
    fn methods(&self) -> Vec<Method> {
        vec![Method::Password]
    }

    /// Checks `password` for `user`, asking the providers in `providers` in that order, or every provider that takes a password
    /// when it is empty. Blocks: the system may sleep after a wrong one, or ask a directory across a network.
    fn sign_in(&self, user: &str, password: &str, providers: &[String]) -> SignIn;

    /// What this portal knows of the certificate the proxy forwarded in `headers` and of `user`'s pinned certificate; `None`
    /// when it pins no certificates. The portal asks for `headers` only from a request on the proxy's socket.
    fn certificate_status(
        &self,
        user: &str,
        uid: u32,
        headers: &[(String, String)],
    ) -> Option<CertificateStatus> {
        let _ = (user, uid, headers);
        None
    }

    /// Pins the certificate in `headers` to `user`, so that it alone signs them in from now on. Replaces the one pinned already.
    ///
    /// # Errors
    ///
    /// [`PinError`] for no certificate, one pinned to another account, or a store that cannot be written.
    fn pin(&self, user: &str, uid: u32, headers: &[(String, String)]) -> Result<(), PinError> {
        let _ = (user, uid, headers);
        Err(PinError::Unsupported)
    }

    /// Removes the certificate pinned to `user`; whether there was one.
    ///
    /// # Errors
    ///
    /// [`PinError`] when the store cannot be written.
    fn unpin(&self, user: &str) -> Result<bool, PinError> {
        let _ = user;
        Err(PinError::Unsupported)
    }

    /// Whether `headers` carry a forwarded certificate at all, readable or not. A certificate sign-in whose request carries none
    /// is not a try the throttle counts and no provider is asked. The default says yes.
    fn carries_certificate(&self, headers: &[(String, String)]) -> bool {
        let _ = headers;
        true
    }

    /// Checks the certificate the proxy forwarded in `headers` (the request's, in order, names in lower case) for a browser at
    /// `client`, asking the providers in `providers` in that order, or every provider that takes a certificate when it is empty.
    /// The portal asks only for a request that came on the proxy's socket. The default refuses.
    fn sign_in_certificate(
        &self,
        headers: &[(String, String)],
        client: IpAddr,
        providers: &[String],
    ) -> SignIn {
        let _ = (headers, client, providers);
        SignIn::Refused
    }

    /// Checks `code`, a one-time code (`Method::Totp`) or a recovery code (`Method::RecoveryCode`), for the account with user id
    /// `uid`, which an earlier step named: the code is never presented with any other name. Asks only the providers in
    /// `providers`, or every one that takes codes when it is empty. The default refuses.
    fn check_code(
        &self,
        user: &str,
        uid: u32,
        method: Method,
        code: &str,
        providers: &[String],
    ) -> CodeCheck {
        let _ = (user, uid, method, code, providers);
        CodeCheck::Refused
    }

    /// Whether the account `user` with user id `uid` has a one-time code set up, and when it was set up, read together: one read
    /// that fails is [`FactorStanding::Unknown`], never a code that is not there. A code belongs to the pair, so an account made
    /// again under the same user id and another name has none, and counts as reset. A session made with a code keeps the time
    /// and ends when it changes, so a code set up again does not carry the sessions of the one it replaced.
    fn factor_record(&self, user: &str, uid: u32) -> (FactorStanding, Option<i64>) {
        let _ = (user, uid);
        (FactorStanding::Unsupported, None)
    }

    /// [`Accounts::factor_record`]'s standing.
    fn factor_standing(&self, user: &str, uid: u32) -> FactorStanding {
        self.factor_record(user, uid).0
    }

    /// When an administrator last cleared the account's failure counts (milliseconds since the epoch): the portal that sees a
    /// later time than it last acted on forgets what it counted for the account.
    fn unlock_stamp(&self, user: &str, uid: u32) -> Option<i64> {
        let _ = (user, uid);
        None
    }

    /// Begins setting up a one-time code for the account, replacing a setup already waiting; `user` is also the name the app
    /// shows. The code that is in use keeps working until a first code confirms the new one.
    ///
    /// # Errors
    ///
    /// [`FactorError`] for a portal that keeps no codes or a store that cannot be written.
    fn begin_factor(&self, user: &str, uid: u32) -> Result<Enrolment, FactorError> {
        let _ = (user, uid);
        Err(FactorError::Unsupported)
    }

    /// Confirms the setup waiting for `uid` with the first code from the app, which makes it the account's code; the id of the
    /// provider that now holds it.
    ///
    /// # Errors
    ///
    /// [`FactorError`] when nothing waits, the code is wrong or the store fails.
    fn confirm_factor(&self, user: &str, uid: u32, code: &str) -> Result<String, FactorError> {
        let _ = (user, uid, code);
        Err(FactorError::Unsupported)
    }

    /// Removes the account's one-time code and any setup waiting, and marks the account reset, so that it is not open to setting
    /// one up for itself: an administrator sets the next one up. Whether there was one.
    ///
    /// # Errors
    ///
    /// [`FactorError`] when the store cannot be written.
    fn reset_factor(&self, user: &str, uid: u32) -> Result<bool, FactorError> {
        let _ = (user, uid);
        Err(FactorError::Unsupported)
    }

    /// Whether `user` is still the account with user id `uid`, asked again each time a host is wanted, so an account that was
    /// removed or renamed since the sign-in is not given one. [`Standing::Unknown`] when the directory could not say, which
    /// ends nothing. The default says present, in no group.
    fn still(&self, user: &str, uid: u32) -> Standing {
        let _ = (user, uid);
        Standing::Present(Vec::new())
    }
}

/// What the accounts say of a signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Standing {
    /// It is there under that name and user id, in these groups.
    Present(Vec<String>),
    /// Definitely not: removed, renamed or given another id.
    Gone,
    /// The directory did not answer: the session stands until it does.
    Unknown,
}

/// The names a unit and a path are made of, as the polkit rule and the root helper in the guide write them. [`valid_user`] is this
/// and nothing else: a name that begins with a digit is never one, since systemd reads `canoa@1000` as a user id.
pub const NAME_PATTERN: &str = "[A-Za-z_][A-Za-z0-9._-]{0,31}";

/// Whether `name` matches [`NAME_PATTERN`]: 1 to 32 letters, digits, `_`, `.` and `-`, beginning with a letter or `_`. A name
/// with a `@`, a `/` or a leading digit, dot or hyphen is never one, whoever's account it is.
#[must_use]
pub fn valid_user(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::{NAME_PATTERN, valid_user};

    #[test]
    fn a_name_is_valid_only_when_it_makes_a_safe_unit_and_path() {
        for good in [
            "alice",
            "canoatesta",
            "a.b-c_d",
            "_svc",
            "A1",
            &"a".repeat(32),
        ] {
            assert!(valid_user(good), "{good}");
        }
        for bad in [
            "",
            "-x",
            ".x",
            "a/b",
            "a@b",
            "..",
            "a b",
            "a\nb",
            "a\0",
            "é",
            "a:b",
            "1000",
            "0",
            "1abc",
            "a\\b",
            &"a".repeat(33),
        ] {
            assert!(!valid_user(bad), "{bad:?}");
        }
    }

    /// The pattern in the guide's polkit rule and in the root helper is the one the validator is, so what the validator
    /// refuses the rule and the helper never let systemd start.
    #[test]
    fn the_polkit_rule_and_the_root_helper_use_the_validators_pattern() {
        let root = env!("CARGO_MANIFEST_DIR");
        let guide = std::fs::read_to_string(format!("{root}/../../docs/portal.md")).unwrap();
        assert!(
            guide.contains(&format!(r"^canoa@{NAME_PATTERN}\.service$")),
            "the polkit rule"
        );
        let helper = std::fs::read_to_string(format!("{root}/dist/canoa-instance")).unwrap();
        assert!(
            helper.contains(&format!("# {NAME_PATTERN}")),
            "the helper names the pattern it checks"
        );
        assert!(
            helper.contains("[!A-Za-z_]*|*[!A-Za-z0-9._-]*"),
            "the helper's case pattern"
        );
    }
}
