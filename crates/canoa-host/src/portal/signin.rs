//! Who may sign in, and as whom: the operating system's accounts.

use coqui_access::{Attempt, Authenticator, Directory, Verdict};
use coqui_access_system::SystemAccounts;

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
    },
    /// Not an account, a wrong password, or an account the system refuses now.
    Refused,
    /// The accounts could not be asked; the reason is for the log, never for the browser.
    Failed(String),
}

/// Whoever says if a name and a password are an account.
pub trait Accounts: Send + Sync {
    /// Checks `password` for `user`. Blocks: the system may sleep after a wrong one, or ask a directory across a network.
    fn sign_in(&self, user: &str, password: &str) -> SignIn;

    /// Whether `user` is still the account with user id `uid`, asked again each time a host is wanted, so an account that was
    /// removed, renamed or disabled since the sign-in is not given one. The default says yes.
    fn still(&self, user: &str, uid: u32) -> bool {
        let _ = (user, uid);
        true
    }
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

/// The accounts of the operating system, through coqui-access's `system` provider: PAM for the password and NSS for the
/// directory, so SSSD and LDAP accounts the machine knows are there too.
pub struct System(SystemAccounts);

impl core::fmt::Debug for System {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("System").finish_non_exhaustive()
    }
}

impl System {
    /// Accounts checked under the PAM service `pam_service`, of user id `min_uid` and above, and of one of `groups` when any are
    /// named.
    ///
    /// # Errors
    ///
    /// When the provider refuses the settings.
    pub fn new(pam_service: &str, min_uid: u32, groups: &[String]) -> Result<Self, String> {
        SystemAccounts::builder()
            .pam_service(pam_service)
            .min_uid(min_uid)
            .groups(groups.iter().cloned())
            .build()
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

impl Accounts for System {
    fn still(&self, user: &str, uid: u32) -> bool {
        let found = self.0.lookup(coqui_access::Lookup {
            by: "id".to_owned(),
            value: uid.to_string(),
        });
        matches!(found, Ok(Some(person)) if person.name == user)
    }

    fn sign_in(&self, user: &str, password: &str) -> SignIn {
        if !valid_user(user) || password.is_empty() || password.len() > MAX_PASSWORD {
            return SignIn::Refused;
        }
        let attempt = Attempt {
            name: Some(user.to_owned()),
            secret: Some(password.to_owned()),
            ..Attempt::default()
        };
        let account = match self.0.authenticate(attempt) {
            Ok(Verdict::Valid(account)) => account,
            Ok(_) => return SignIn::Refused,
            Err(error) => return SignIn::Failed(error.to_string()),
        };
        // The name the system gives the account is the one a unit and a path are made of, whatever case was typed.
        match self.0.lookup(account) {
            Ok(Some(person)) if valid_user(&person.name) => match person.id.parse() {
                Ok(uid) => SignIn::Admitted {
                    user: person.name,
                    uid,
                },
                Err(_) => SignIn::Refused,
            },
            Ok(_) => SignIn::Refused,
            Err(error) => SignIn::Failed(error.to_string()),
        }
    }
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
