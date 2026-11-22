//! The names of the environment variables the host reads, with the prefix of the build's brand.
//!
//! The prefix is the build's `BRAND_ENV_PREFIX` (`CANOA` without one), so a rebranded build reads `<PREFIX>_CONFIG` where this
//! one reads `CANOA_CONFIG`. A program that starts another passes these names, never literals.

macro_rules! variable {
    ($(#[$doc:meta])* $name:ident, $suffix:literal) => {
        $(#[$doc])*
        pub const $name: &str = concat!(env!("CANOA_RESOLVED_ENV_PREFIX"), "_", $suffix);
    };
}

/// The prefix, without the underscore.
pub const ENV_PREFIX: &str = env!("CANOA_RESOLVED_ENV_PREFIX");

variable!(
    /// The list of configuration files, below `--config` and above the platform's default.
    CONFIG, "CONFIG"
);
variable!(
    /// The folder that counts as beside the program, when it is not the executable's own.
    INSTALLED_DIR, "INSTALLED_DIR"
);
variable!(
    /// The tracing filter for what is logged on standard error.
    LOG, "LOG"
);
variable!(
    /// The file the host's log also goes to while it runs inside an application.
    HOST_LOG, "HOST_LOG"
);
variable!(
    /// `memory` keeps every credential in the process's memory, out of the operating system's store.
    CREDENTIAL_STORE, "CREDENTIAL_STORE"
);
variable!(
    /// The tool server's address for an agent the host starts.
    DELEGATE_URL, "DELEGATE_URL"
);
variable!(
    /// The bearer of the session an agent the host starts belongs to.
    DELEGATE_TOKEN, "DELEGATE_TOKEN"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_the_prefix_then_its_own_suffix() {
        for (name, suffix) in [
            (CONFIG, "CONFIG"),
            (INSTALLED_DIR, "INSTALLED_DIR"),
            (LOG, "LOG"),
            (HOST_LOG, "HOST_LOG"),
            (CREDENTIAL_STORE, "CREDENTIAL_STORE"),
            (DELEGATE_URL, "DELEGATE_URL"),
            (DELEGATE_TOKEN, "DELEGATE_TOKEN"),
        ] {
            assert_eq!(name, format!("{ENV_PREFIX}_{suffix}"));
        }
        assert!(
            ENV_PREFIX
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }
}
