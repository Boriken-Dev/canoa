//! Whether a hook's allow may answer a permission request for a call: the places the person protects still decide.
//!
//! The agent asks permission only for a call its own rules and mode have left to the person, so an allow answers what the
//! person would have been asked. What the host can add is the person's list of protected paths (`[permissions] protected`,
//! else the recommended list the app uses too): a call that names one, or a path the host cannot place, goes to the person.

use alloc::string::String;
use alloc::vec::Vec;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::store::protect::Protected;

/// What the host protects when the person wrote no list: the app's recommended list, one file both read.
pub(crate) const RECOMMENDED: &str = include_str!("../../../protected_default.txt");

/// The keys of a call's input that name a file.
const PATH_KEYS: [&str; 4] = ["file_path", "path", "notebook_path", "filePath"];

/// The person's protected paths.
#[derive(Debug)]
pub(super) struct Refusals {
    list: Protected,
    home: Option<PathBuf>,
}

impl Refusals {
    /// The list `lines` write, or the host's recommended places when the person wrote none.
    pub(super) fn new(lines: Option<&[String]>) -> Self {
        Self {
            list: lines.map_or_else(|| Protected::new(RECOMMENDED.lines()), Protected::new),
            home: std::env::home_dir(),
        }
    }

    /// Whether `call` (a permission request's `toolCall`) names a protected place, or one that cannot be placed, resolved
    /// against `cwd`.
    pub(super) fn touches(&self, call: &Value, cwd: &Path) -> bool {
        named(call).iter().any(|path| !self.allows(path, cwd))
    }

    fn allows(&self, path: &str, cwd: &Path) -> bool {
        let Some(placed) = place(cwd, path) else {
            return false;
        };
        let home = self.home.as_deref();
        if self.list.protects(&placed, home) {
            return false;
        }
        // Links resolved too: the name the agent wrote is not where the file is.
        match resolved(&placed) {
            Some(real) => !self.list.protects(&real, home),
            None => false,
        }
    }
}

/// The file paths a call names: its locations and the path keys of its input.
fn named(call: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for location in call
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = location.get("path").and_then(Value::as_str) {
            out.push(path.to_owned());
        }
    }
    if let Some(input) = call.get("rawInput").and_then(Value::as_object) {
        for key in PATH_KEYS {
            if let Some(path) = input.get(key).and_then(Value::as_str) {
                out.push(path.to_owned());
            }
        }
        for path in input
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(path) = path.as_str() {
                out.push(path.to_owned());
            }
        }
    }
    out
}

/// `path` made absolute against `cwd` with `.` and `..` worked out; none for one that cannot be placed: empty, holding a
/// zero byte, starting with `~` (which a shell expands and this does not), or climbing above its root.
fn place(cwd: &Path, path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\0') || path.starts_with('~') {
        return None;
    }
    let joined = cwd.join(path);
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(part.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
        }
    }
    out.is_absolute().then_some(out)
}

/// `path` with the links of its longest existing part followed, and the rest as written; none when that part cannot be
/// resolved.
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn refusals(lines: &[&str]) -> Refusals {
        Refusals::new(Some(
            &lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>(),
        ))
    }

    fn folder() -> PathBuf {
        std::env::temp_dir()
    }

    fn edit(path: &str) -> Value {
        json!({"kind": "edit", "rawInput": {"file_path": path}})
    }

    #[test]
    fn a_path_the_list_protects_is_touched_whether_it_is_relative_or_absolute() {
        let list = refusals(&["calc.js", ".env"]);
        let cwd = folder();
        assert!(list.touches(&edit("src/calc.js"), &cwd));
        assert!(list.touches(
            &edit(&cwd.join("a").join("CALC.JS").to_string_lossy()),
            &cwd
        ));
        assert!(list.touches(&edit("./sub/../.env"), &cwd));
        assert!(!list.touches(&edit("src/report.js"), &cwd));
        assert!(!list.touches(
            &json!({"kind": "execute", "rawInput": {"command": "ls"}}),
            &cwd
        ));
    }

    #[test]
    fn a_path_that_cannot_be_placed_is_touched() {
        let list = refusals(&[]);
        let cwd = folder();
        for path in ["", "~/x", "a\0b"] {
            assert!(list.touches(&edit(path), &cwd), "{path:?}");
        }
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(list.touches(
            &edit("../../../../../../../../../../../../.."),
            Path::new(root)
        ));
    }

    #[test]
    fn locations_and_paths_of_a_call_count_too() {
        let list = refusals(&["calc.js"]);
        let cwd = folder();
        assert!(list.touches(&json!({"locations": [{"path": "calc.js"}]}), &cwd));
        assert!(list.touches(&json!({"rawInput": {"paths": ["a", "calc.js"]}}), &cwd));
        assert!(!list.touches(
            &json!({"locations": [{"path": "a.js"}], "rawInput": {}}),
            &cwd
        ));
    }

    #[test]
    fn the_recommended_list_applies_when_the_person_wrote_no_list() {
        let list = Refusals::new(None);
        let cwd = folder();
        for path in [
            ".env",
            "src/.git/config",
            "package.json",
            "a/Cargo.toml",
            "node_modules/x/y.js",
            "CLAUDE.md",
        ] {
            assert!(list.touches(&edit(path), &cwd), "{path}");
        }
        assert!(!list.touches(&edit("src/main.rs"), &cwd));
    }

    #[test]
    fn the_recommended_list_is_the_one_the_app_reads() {
        // The app's constant must equal this file; its own test (`protected_paths_test.dart`) fails when it drifts.
        assert!(RECOMMENDED.lines().any(|line| line == ".*"));
        assert!(RECOMMENDED.ends_with('\n') && !RECOMMENDED.contains('\r'));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_protected_place_is_touched() {
        let list = refusals(&["target-dir"]);
        let dir = folder().join(format!("canoa-allow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target-dir")).expect("a folder");
        std::os::unix::fs::symlink(dir.join("target-dir"), dir.join("plain")).expect("a link");
        let touched = list.touches(&edit("plain/file.txt"), &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(touched);
    }
}
