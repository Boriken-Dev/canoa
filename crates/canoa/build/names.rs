//! Reads and checks a brand file: the names a build carries. Shared by the build script and the tests that hold the rules.
//!
//! The rules are the ones `app/tool/brand.py` holds; `tests/brand_names.rs` runs both against the same table of good and bad
//! values.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Windows' reserved device names, which no file or folder may be called, with or without an extension.
const DEVICES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Every key a brand file may hold.
const KEYS: &[&str] = &[
    "name",
    "short_name",
    "executable",
    "id",
    "env_prefix",
    "icon",
    "accent",
    "accent_on_dark",
    "program",
    "config_file",
    "folder",
    "home_dir",
];

/// A brand's names, each as the build carries it.
#[derive(Debug, PartialEq, Eq)]
pub struct Names {
    pub name: String,
    pub short_name: String,
    pub executable: String,
    pub id: String,
    pub env_prefix: String,
    pub program: String,
    pub config_file: String,
    pub folder: String,
    pub home_dir: String,
    /// The icon's path, relative to the brand file.
    pub icon: String,
}

impl Names {
    /// The brand `text` holds, or why it is not one.
    pub fn read(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let object = json.as_object().ok_or("not an object")?;
        if let Some(key) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
            return Err(format!("unknown key {key:?}"));
        }
        let text = |key: &str| -> Result<Option<&str>, String> {
            match object.get(key) {
                None => Ok(None),
                Some(Value::String(value)) => Ok(Some(value.as_str())),
                Some(_) => Err(format!("{key} is not text")),
            }
        };
        let need = |key: &str| -> Result<String, String> {
            match text(key)? {
                Some(value) if !value.trim().is_empty() => Ok(value.to_owned()),
                _ => Err(format!("{key} is missing or not text")),
            }
        };
        let name = need("name")?;
        let short_name = need("short_name")?;
        let executable = need("executable")?;
        let id = need("id")?;
        let env_prefix = need("env_prefix")?;
        let icon = need("icon")?;
        need("accent")?;
        if !executable.starts_with(|c: char| c.is_ascii_alphabetic())
            || !executable.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("executable is letters and digits, starting with a letter".to_owned());
        }
        let labels = id.split('.').collect::<Vec<_>>();
        if labels.len() < 2
            || !labels.iter().all(|label| {
                label.starts_with(|c: char| c.is_ascii_lowercase())
                    && label
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
        {
            return Err(
                "id is a reverse-DNS name in lower case, such as com.example.app".to_owned(),
            );
        }
        if !env_prefix.starts_with(|c: char| c.is_ascii_uppercase())
            || !env_prefix
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(
                "env_prefix is upper-case letters and digits, such as MYAPP".to_owned(),
            );
        }
        for key in ["accent", "accent_on_dark"] {
            if let Some(color) = text(key)?
                && !is_color(color)
            {
                return Err(format!("{key} is #RRGGBB"));
            }
        }
        if name.chars().chain(short_name.chars()).any(|c| {
            matches!(c, '"' | '\\' | '\'' | '<' | '>' | '&' | '|' | '^' | '%') || c.is_control()
        }) {
            return Err(
                "name and short_name cannot hold quotes, backslashes or shell and resource markers"
                    .to_owned(),
            );
        }
        let program = match text("program")? {
            Some(program) => program.to_owned(),
            None => name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                .collect(),
        };
        if program.len() > 31
            || !program.starts_with(|c: char| c.is_ascii_lowercase())
            || !program
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("program is lower-case letters, digits and hyphens, starting with a letter (a unit and a user name)".to_owned());
        }
        let config_file = text("config_file")?
            .map_or_else(|| format!("{program}.cfg"), str::to_owned);
        let folder = text("folder")?.map_or_else(|| name.clone(), str::to_owned);
        let home_dir = text("home_dir")?.map_or_else(|| format!(".{program}"), str::to_owned);
        for (key, value) in [
            ("config_file", &config_file),
            ("folder", &folder),
            ("home_dir", &home_dir),
        ] {
            if let Some(why) = not_a_component(value) {
                return Err(format!("{key} {why}"));
            }
        }
        Ok(Self {
            name,
            short_name,
            executable,
            id,
            env_prefix,
            program,
            config_file,
            folder,
            home_dir,
            icon,
        })
    }

    /// Where the icon is, given the brand file's own path.
    pub fn icon_path(&self, file: &Path) -> PathBuf {
        file.parent().unwrap_or_else(|| Path::new(".")).join(&self.icon)
    }
}

fn is_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Why `value` cannot be one file or folder name on every operating system, or `None` when it can.
fn not_a_component(value: &str) -> Option<&'static str> {
    if value.is_empty() {
        return Some("is empty");
    }
    if value.contains("..")
        || value
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
    {
        return Some(
            "is one file or folder name: no separators, colons, wildcards, quotes or control characters",
        );
    }
    if value != value.trim() || value.ends_with('.') {
        return Some("cannot start or end with a space, or end with a dot");
    }
    let stem = value.split('.').next().unwrap_or(value).to_lowercase();
    if DEVICES.contains(&stem.as_str()) {
        return Some("is a device name on Windows");
    }
    None
}
