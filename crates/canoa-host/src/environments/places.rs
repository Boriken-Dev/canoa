//! Whether a path an environment's tools are given names a place the person protects, judged by that environment's own path rules
//! (its operating system as it describes itself), not this computer's. What cannot be judged is refused: a path that cannot be
//! placed, a Windows name in a form another one opens (a stream, trailing dots or spaces, a short 8.3 name), and a relative path
//! when the list has lines anchored at a root or a home, since the folder it is relative to is the environment's. Links are
//! followed only for an environment that shares this computer's files; elsewhere they are the environment's own and a path is
//! judged as written.

use std::path::{Path, PathBuf};

use canoa::store::protect::Protected;

/// The path rules of an environment.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rules {
    /// Its paths are Windows paths.
    pub(crate) windows: bool,
    /// It shares this computer's files, so links can be followed here.
    pub(crate) local: bool,
    /// The names of the folder `~/` means there, when it is known.
    pub(crate) home: Option<Vec<String>>,
}

/// Whether `name` is in the form of a short (8.3) name, `NAME~1.EXT`.
fn short_name(name: &str) -> bool {
    name.char_indices()
        .any(|(at, c)| c == '~' && name[at + 1..].starts_with(|next: char| next.is_ascii_digit()))
}

/// The names of `path` by Windows' rules or POSIX ones (drive first, a share's server and share as two names), and whether it is
/// absolute.
///
/// # Errors
///
/// Why it cannot be placed: a `..` part, and on Windows a stream or device (a `:` past the drive), a name that is only dots and
/// spaces, a short name, or a path relative to a drive's own folder.
pub(crate) fn names(path: &str, windows: bool) -> Result<(Vec<String>, bool), String> {
    let cannot = |why: &str| Err(format!("{path:?} cannot be placed ({why})"));
    if !windows {
        let mut out = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => return cannot("a .. part"),
                name => out.push(name.to_owned()),
            }
        }
        return Ok((out, path.starts_with('/')));
    }
    let mut rest = path;
    for verbatim in [r"\\?\", "//?/", r"\\.\", "//./"] {
        if let Some(after) = rest.strip_prefix(verbatim) {
            rest = after;
            if let Some(share) = after
                .strip_prefix("UNC\\")
                .or_else(|| after.strip_prefix("UNC/"))
            {
                let mut out = Vec::new();
                return walk(share, &mut out).map(|()| (out, true));
            }
            break;
        }
    }
    let mut out = Vec::new();
    let separator = |c: char| c == '/' || c == '\\';
    let absolute;
    if rest.starts_with(separator) && rest[1..].starts_with(separator) {
        // A share: its server and share are the first two names.
        absolute = true;
        rest = &rest[2..];
    } else if let [drive, b':', after @ ..] = rest.as_bytes()
        && drive.is_ascii_alphabetic()
    {
        if after.first().is_some_and(|c| *c == b'/' || *c == b'\\') {
            absolute = true;
            out.push(rest[..2].to_owned());
            rest = &rest[2..];
        } else {
            return cannot("relative to a drive's own folder");
        }
    } else {
        absolute = rest.starts_with(separator);
    }
    walk(rest, &mut out).map(|()| (out, absolute))
}

/// Adds the names of a Windows path's `rest` to `out`, each as Windows opens it.
fn walk(rest: &str, out: &mut Vec<String>) -> Result<(), String> {
    for part in rest.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(format!("{rest:?} cannot be placed (a .. part)"));
        }
        if part.contains(':') {
            return Err(format!(
                "{part:?} cannot be placed (a stream or a device: Windows opens another name than the one written)"
            ));
        }
        // Windows drops trailing dots and spaces, so `package.json.` is `package.json`.
        let name = part.trim_end_matches(['.', ' ']);
        if name.is_empty() {
            return Err(format!("{part:?} cannot be placed (only dots and spaces)"));
        }
        if short_name(name) {
            return Err(format!(
                "{part:?} cannot be placed (a short 8.3 name, which cannot be told here from the long name it stands for)"
            ));
        }
        out.push(name.to_owned());
    }
    Ok(())
}

/// `path` with the links of its longest existing part followed, on this computer, and the rest as written.
fn resolved(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        match at.try_exists() {
            Ok(true) => {
                let mut real = at.canonicalize().ok()?;
                real.extend(rest.iter().rev());
                return Some(real);
            }
            Ok(false) => {
                rest.push(at.file_name()?);
                at = at.parent()?;
            }
            Err(_) => return None,
        }
    }
}

/// Whether `list` protects `path`, an environment's path judged by that environment's `rules`.
///
/// # Errors
///
/// Why the path cannot be judged, for the agent: it cannot be placed, or it is relative where the list has lines anchored at a root
/// or a home.
pub(crate) fn protected(list: &Protected, rules: &Rules, path: &str) -> Result<bool, String> {
    if path.is_empty() || path.contains('\0') || path.starts_with('~') {
        return Err(format!(
            "{path:?} cannot be placed (empty, or starting with ~): give the path plainly"
        ));
    }
    let (parts, absolute) = names(path, rules.windows).map_err(|why| {
        format!("{why}, so it is kept as a protected place is: give the path plainly")
    })?;
    if !absolute && list.anchored() {
        return Err(format!(
            "{path} is relative, and the person's protected places include lines anchored at a root or a home, which a path relative to the environment's own folder cannot be judged by: give the full path"
        ));
    }
    if list.protects_parts(&parts, rules.home.as_deref()) {
        return Ok(true);
    }
    if rules.local && absolute {
        let real = resolved(Path::new(path)).ok_or_else(|| {
            format!("{path} cannot be followed to the place it names, so it is kept as a protected place is")
        })?;
        let home = std::env::home_dir();
        if list.protects(&real, home.as_deref()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the tools of an environment with `rules` may act on `path`, which `list` may protect.
///
/// # Errors
///
/// Why not, for the agent: protected, or not a path that can be judged.
pub(crate) fn judge(list: &Protected, rules: &Rules, path: &str) -> Result<(), String> {
    if protected(list, rules, path)? {
        return Err(format!(
            "{path} is on the person's list of protected places ([permissions] protected), which this session's tools read or change only in the Bypass mode"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(lines: &[&str]) -> Protected {
        Protected::new(lines.iter().copied())
    }

    fn remote(windows: bool) -> Rules {
        Rules {
            windows,
            local: false,
            home: None,
        }
    }

    #[test]
    fn a_windows_environment_is_judged_by_windows_rules_wherever_the_host_runs() {
        let protected = list(&[".*", "package.json"]);
        let windows = remote(true);
        for path in [
            r"C:\proj\.git\config",
            r"C:\proj\package.json",
            r"c:/proj/PACKAGE.JSON",
            r"C:\proj\package.json.",
            r"C:\proj\package.json  ",
            r"\\server\share\.env",
            r"\\?\C:\proj\package.json",
            r"\\?\UNC\server\share\.ssh\id",
        ] {
            let why = judge(&protected, &windows, path).expect_err(path);
            assert!(why.contains("protected places"), "{path}: {why}");
        }
        for path in [
            r"C:\proj\package.json::$DATA",
            r"C:\proj\PACKAG~1.JSO",
            r"C:\proj\...",
            r"C:proj\package.json",
            r"C:\proj\..\x",
        ] {
            let why = judge(&protected, &windows, path).expect_err(path);
            assert!(why.contains("cannot be placed"), "{path}: {why}");
        }
        judge(&protected, &windows, r"C:\proj\src\main.rs").expect("not protected");
        // The control: by POSIX rules a backslash is part of a name, so the same text names one file.
        judge(&protected, &remote(false), r"C:\proj\package.json.").expect("one name");
    }

    #[test]
    fn a_relative_path_cannot_be_judged_by_an_anchored_line_and_home_lines_need_the_environments_home()
     {
        let anchored = list(&["/etc", "~/.ssh"]);
        let linux = remote(false);
        let why = judge(&anchored, &linux, "etc/passwd").expect_err("relative");
        assert!(why.contains("give the full path"), "{why}");
        assert!(judge(&anchored, &linux, "/etc/passwd").is_err());
        judge(&anchored, &linux, "/home/me/.ssh/id").expect("its home is not known");
        let known = Rules {
            home: Some(vec!["home".to_owned(), "me".to_owned()]),
            ..linux
        };
        assert!(judge(&anchored, &known, "/home/me/.ssh/id").is_err());
        // Lines that are not anchored judge a relative path as written.
        assert!(judge(&list(&[".env"]), &remote(false), "app/.env").is_err());
        judge(&list(&[".env"]), &remote(false), "app/main.rs").expect("fine");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_into_a_protected_place_is_followed_where_the_environment_shares_this_computers_files()
    {
        let dir = std::env::temp_dir().join(format!("canoa-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("secret")).expect("a folder");
        std::os::unix::fs::symlink(dir.join("secret"), dir.join("plain")).expect("a link");
        let rules = Rules {
            local: true,
            ..Rules::default()
        };
        let path = dir.join("plain").join("x").to_string_lossy().into_owned();
        let judged = judge(&list(&["secret"]), &rules, &path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(judged.is_err());
    }
}
