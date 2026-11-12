//! Which relative paths a payload may name. The same rule runs when packing, when reading the manifest and before any file
//! is created, so a payload cannot reach outside its folder or name a file Windows would treat as a device or a stream.

/// Longest path a payload may name, in bytes.
const MAX_PATH: usize = 240;
/// Longest single path component, in bytes.
const MAX_COMPONENT: usize = 120;

/// Windows device names, which name a device in any folder, with or without an extension.
const DEVICES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why `path` cannot be a payload path, or `Ok` when it can: relative, `/`-separated, plain components only.
pub fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_PATH {
        return Err(format!("path length {} is not 1..={MAX_PATH}", path.len()));
    }
    for part in path.split('/') {
        check_component(part).map_err(|why| format!("{path:?}: {why}"))?;
    }
    Ok(())
}

fn check_component(part: &str) -> Result<(), &'static str> {
    if part.is_empty() || part.len() > MAX_COMPONENT {
        return Err("an empty or overlong component");
    }
    if part == "." || part == ".." {
        return Err("a dot component");
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err("a component that ends in a dot or a space");
    }
    if part
        .chars()
        .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
    {
        return Err("a control character or one of \\ : * ? \" < > |");
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_ascii_lowercase();
    if DEVICES.contains(&stem.as_str()) {
        return Err("a Windows device name");
    }
    Ok(())
}

/// A version names a folder: letters, digits and `. _ + -`, not starting with a dot.
pub fn check_version(version: &str) -> Result<(), String> {
    let ok = !version.is_empty()
        && version.len() <= 64
        && !version.starts_with('.')
        && !version.ends_with('.')
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "version {version:?} is not 1..=64 of letters, digits and . _ + -"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_pass() {
        for ok in [
            "CanoaUI.exe",
            "data/flutter_assets/AssetManifest.bin",
            "a b/c.d",
            "x/.hidden",
        ] {
            assert!(check_path(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn paths_that_leave_the_folder_or_name_devices_fail() {
        let long = "a".repeat(241);
        for bad in [
            "",
            "/abs",
            "a//b",
            "../x",
            "a/../x",
            "./x",
            "a/.",
            "C:/x",
            "a\\b",
            "a:b",
            "x.exe:stream",
            "a/",
            "con",
            "Nul.txt",
            "dir/COM1.log",
            "trail.",
            "trail ",
            "tab\tname",
            "star*",
            &long,
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn versions_name_a_plain_folder() {
        for ok in ["0.0.0-alpha.0", "1.2.3+abc", "v1_2"] {
            assert!(check_version(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".x", "a/b", "a\\b", "..", "x.", "a b", &"v".repeat(65)] {
            assert!(check_version(bad).is_err(), "{bad:?}");
        }
    }
}
