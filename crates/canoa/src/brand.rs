//! The names a build carries: what the product is called, the files and folders it keeps, and the environment variables it reads.
//!
//! They come from the brand file `BRAND_FILE` names at build time (the copy of `app/brand.json` in this crate's `brand/` without
//! one), so a rebranded build keeps `agents.cfg` in `~/.agentui` and reads `AGENTUI_CONFIG` where this one keeps `canoa.cfg` in
//! `~/.canoa` and reads `CANOA_CONFIG`. A program that starts another passes these names, never literals.

/// What the product is called, as a person reads it.
pub const NAME: &str = env!("CANOA_BRAND_NAME");
/// The short form of the name.
pub const SHORT_NAME: &str = env!("CANOA_BRAND_SHORT_NAME");
/// The desktop program's name, without its extension.
pub const EXECUTABLE: &str = env!("CANOA_BRAND_EXECUTABLE");
/// The reverse-DNS identifier: the native messaging host's name, the credential store's label.
pub const ID: &str = env!("CANOA_BRAND_ID");
/// The command line's name: what messages call the program and what its units are named for.
pub const PROGRAM: &str = env!("CANOA_BRAND_PROGRAM");
/// The configuration file's name in every layer.
pub const CONFIG_FILE: &str = env!("CANOA_BRAND_CONFIG_FILE");
/// The one folder under each operating system's configuration, system and data roots.
pub const FOLDER: &str = env!("CANOA_BRAND_FOLDER");
/// The home under the person's own folder, and the per-project folder: a dot first.
pub const HOME_DIR: &str = env!("CANOA_BRAND_HOME_DIR");
/// The user layer's path under a configuration root: the folder, then the file.
pub const USER_CONFIG: &str = concat!(
    env!("CANOA_BRAND_FOLDER"),
    "/",
    env!("CANOA_BRAND_CONFIG_FILE")
);
/// The brand file this build was made from.
pub const JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/brand.json"));
/// The brand's icon, an SVG.
pub const ICON_SVG: &str = include_str!(concat!(env!("OUT_DIR"), "/brand.svg"));

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

    #[test]
    fn the_names_are_the_brand_files() {
        let json: serde_json::Value = serde_json::from_str(JSON).expect("the brand file is JSON");
        let spelled = |key: &str| json[key].as_str().map(str::to_owned);
        assert_eq!(spelled("name").as_deref(), Some(NAME));
        assert_eq!(spelled("id").as_deref(), Some(ID));
        assert_eq!(spelled("env_prefix").as_deref(), Some(ENV_PREFIX));
        for (key, value) in [
            ("program", PROGRAM),
            ("config_file", CONFIG_FILE),
            ("folder", FOLDER),
            ("home_dir", HOME_DIR),
        ] {
            if let Some(given) = spelled(key) {
                assert_eq!(given, value, "{key}");
            }
        }
        assert_eq!(USER_CONFIG, format!("{FOLDER}/{CONFIG_FILE}"));
        assert!(HOME_DIR.starts_with('.'));
        assert!(ICON_SVG.contains("<svg"));
    }

    /// The crate builds packaged on its own, so it carries a copy of the app's brand; a checkout holds both.
    #[test]
    fn the_crates_copy_is_the_apps_brand() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (copy, original) in [
            ("brand/brand.json", "../../app/brand.json"),
            (
                "brand/assets/icons/canoa.svg",
                "../../app/assets/icons/canoa.svg",
            ),
        ] {
            let Ok(app) = std::fs::read_to_string(root.join(original)) else {
                continue;
            };
            let ours =
                std::fs::read_to_string(root.join(copy)).expect("the crate carries its copy");
            assert_eq!(
                ours.replace("\r\n", "\n"),
                app.replace("\r\n", "\n"),
                "{copy} is stale: copy {original} over it"
            );
        }
    }
}
