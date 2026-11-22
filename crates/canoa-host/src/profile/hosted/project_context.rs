//! A project's context files (`AGENTS.md` and its like, in each directory and its `.agents` one) for the agent that does not load
//! them itself: which files a session's directory has, which of them the harness loads already, and the text of the rest, each
//! with its path.
//!
//! The rule is the specification's "Project context files": the names, their order and the two variables that change them
//! (`AGENTS_DIR`, `AGENTS_CONTEXT`) are `coqui_aos`'s. This reads the files, bounds them and says what became of each.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use coqui_aos::{ProjectContext, StdFs, context_files};
use serde_json::{Value, json};

use super::overlays::MAX_CONTEXT;
use super::{Item, State};

/// The key of a project context file in the profile's report.
const KEY: &str = "project_context";
/// The layer of such an item: the files are the project's own.
const LAYER: &str = "project";

/// The two variables that name the places and the files, as the session starts with them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::profile::hosted) struct Names {
    pub(in crate::profile::hosted) directories: Option<String>,
    pub(in crate::profile::hosted) files: Option<String>,
}

impl Names {
    /// The variables as `lookup` has them; an empty one is unset.
    pub(in crate::profile::hosted) fn read(lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        let get = |name| lookup(name).filter(|value| !value.is_empty());
        Self {
            directories: get("AGENTS_DIR"),
            files: get("AGENTS_CONTEXT"),
        }
    }
}

/// What a walk of a session's directories gave.
#[derive(Debug, Default)]
pub(in crate::profile::hosted) struct Fed {
    /// Each file to hand over, as its path and text, in the order a later one overrides an earlier one.
    pub(in crate::profile::hosted) files: Vec<(String, String)>,
    /// What the report says of each file the walk found, and of each name the variables hold that is not valid.
    pub(in crate::profile::hosted) items: Vec<Item>,
    /// The levels the walk covered: the directories whose files are now with the session.
    pub(in crate::profile::hosted) levels: Vec<String>,
    /// Whether the walk found any context file at all, handed over or left to the harness.
    pub(in crate::profile::hosted) found: bool,
    /// The bytes of text in `files`.
    pub(in crate::profile::hosted) bytes: usize,
}

/// The context files of the levels from `root` down to `cwd` that are not in `delivered`, read. `native` are the paths the harness
/// loads itself, `held` the bytes of context the session already has, and `how` what the report says of a file that is handed over.
pub(in crate::profile::hosted) fn feed(
    names: &Names,
    native: &[&str],
    root: &Path,
    cwd: &Path,
    delivered: &[String],
    held: usize,
    how: &str,
) -> Fed {
    let query = ProjectContext::new(Some(root.to_path_buf()), cwd.to_path_buf())
        .agents_dir(names.directories.clone())
        .agents_context(names.files.clone())
        .native(native.iter().copied())
        .delivered(delivered);
    let found = context_files(&StdFs, &query);
    let mut fed = Fed {
        found: !found.files.is_empty() || !found.skipped.is_empty(),
        levels: found.levels.iter().map(|level| shown(level)).collect(),
        ..Fed::default()
    };
    for entry in &found.ignored {
        fed.items.push(Item::new(
            format!("{KEY}.{entry}"),
            LAYER,
            State::Refused,
            "AGENTS_DIR or AGENTS_CONTEXT names this, which is not one directory or file name",
        ));
    }
    for path in &found.skipped {
        fed.items.push(Item::new(
            format!("{KEY}.{}", relative(root, path)),
            LAYER,
            State::Off,
            "the agent loads this file itself",
        ));
    }
    for path in &found.files {
        let name = relative(root, path);
        let key = format!("{KEY}.{name}");
        match read(path) {
            Ok(text) if held + fed.bytes + text.len() > MAX_CONTEXT => {
                fed.items.push(Item::new(
                    key,
                    LAYER,
                    State::Refused,
                    format!("the initial context passes {MAX_CONTEXT} bytes"),
                ));
            }
            Ok(text) => {
                fed.bytes += text.len();
                fed.files.push((shown(path), text));
                fed.items.push(Item::new(key, LAYER, State::Applied, how));
            }
            Err(why) => fed.items.push(Item::new(key, LAYER, State::Refused, why)),
        }
    }
    fed
}

/// The text of a context file, or why it is not handed over.
fn read(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("it cannot be read ({error})"))?;
    String::from_utf8(bytes).map_err(|_| "it is not UTF-8 text".to_owned())
}

/// A path as the host writes it to the agent, with `/` on every platform.
fn shown(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `path` below `root`, written with `/`; the whole path when it is not below it.
fn relative(root: &Path, path: &Path) -> String {
    shown(path.strip_prefix(root).unwrap_or(path))
}

/// The content blocks a file is handed over as: a label the host wrote, and the file's text on its own, so nothing a file holds
/// can pass for the label.
pub(in crate::profile::hosted) fn blocks(files: &[(String, String)]) -> Vec<Value> {
    files
        .iter()
        .flat_map(|(path, text)| {
            [
                json!({"type": "text", "text": format!("The project's context file {path}, not written by the user in this chat:")}),
                json!({"type": "text", "text": text}),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let mut random = [0_u8; 6];
        let _ = getrandom::fill(&mut random);
        let path = std::env::temp_dir().join(format!(
            "canoa-project-context-{name}-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        fs::create_dir_all(&path).expect("a directory");
        path
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a parent")).expect("directories");
        fs::write(path, text).expect("a file");
    }

    fn paths(fed: &Fed, root: &Path) -> Vec<String> {
        fed.files
            .iter()
            .map(|(path, _)| {
                path.strip_prefix(&shown(root))
                    .unwrap_or(path)
                    .trim_start_matches('/')
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn a_nested_projects_files_come_broad_first_and_a_local_one_last() {
        let root = dir("order");
        put(&root.join("AGENTS.md"), "root");
        put(&root.join(".agents/AGENTS.md"), "private root");
        put(&root.join("AGENTS.local.md"), "local root");
        put(&root.join("a/AGENTS.md"), "a");
        put(&root.join("a/b/.agents/AGENTS.md"), "b");
        put(&root.join("c/AGENTS.md"), "elsewhere");
        let fed = feed(
            &Names::default(),
            &[],
            &root,
            &root.join("a/b"),
            &[],
            0,
            "fed",
        );
        assert_eq!(
            paths(&fed, &root),
            [
                "AGENTS.md",
                ".agents/AGENTS.md",
                "AGENTS.local.md",
                "a/AGENTS.md",
                "a/b/.agents/AGENTS.md"
            ]
        );
        assert_eq!(fed.files[0].1, "root");
        assert!(fed.found);
        assert_eq!(fed.levels.len(), 3);
        fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn what_the_agent_loads_itself_is_left_out_and_reported_off() {
        let root = dir("native");
        put(&root.join("AGENTS.md"), "root");
        put(&root.join(".agents/AGENTS.md"), "private");
        let fed = feed(
            &Names::default(),
            &["AGENTS.md"],
            &root,
            &root,
            &[],
            0,
            "fed",
        );
        assert_eq!(paths(&fed, &root), [".agents/AGENTS.md"]);
        let off: Vec<&Item> = fed
            .items
            .iter()
            .filter(|item| item.state == State::Off)
            .collect();
        assert_eq!(off.len(), 1);
        assert!(off[0].key.ends_with("AGENTS.md"));
        fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn a_directory_already_delivered_gives_nothing_and_a_deeper_one_gives_its_own() {
        let root = dir("delivered");
        put(&root.join("AGENTS.md"), "root");
        put(&root.join("a/AGENTS.md"), "a");
        let held = vec![shown(&root)];
        let fed = feed(
            &Names::default(),
            &[],
            &root,
            &root.join("a"),
            &held,
            0,
            "fed",
        );
        assert_eq!(paths(&fed, &root), ["a/AGENTS.md"]);
        let again = feed(&Names::default(), &[], &root, &root, &held, 0, "fed");
        assert!(again.files.is_empty() && again.levels.is_empty());
        fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn the_variables_change_the_places_and_the_names_and_a_bad_entry_is_reported() {
        let root = dir("names");
        put(&root.join(".ai/CONTEXT.md"), "ai");
        put(&root.join(".agents/AGENTS.md"), "agents");
        let names = Names {
            directories: Some(format!(".ai{}../x", if cfg!(windows) { ';' } else { ':' })),
            files: Some("CONTEXT.md".to_owned()),
        };
        let fed = feed(&names, &[], &root, &root, &[], 0, "fed");
        assert_eq!(paths(&fed, &root), [".ai/CONTEXT.md"]);
        assert!(
            fed.items
                .iter()
                .any(|item| item.state == State::Refused && item.key.ends_with("../x"))
        );
        fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn a_file_that_is_not_text_or_would_pass_the_bound_is_reported_not_handed_over() {
        let root = dir("refused");
        put(&root.join("AGENTS.md"), "fits");
        fs::write(root.join("AGENTS.a.md"), [0xff, 0xfe, 0x00]).expect("bytes");
        put(&root.join("AGENTS.b.md"), "does not");
        let fed = feed(
            &Names::default(),
            &[],
            &root,
            &root,
            &[],
            MAX_CONTEXT - 4,
            "fed",
        );
        assert_eq!(paths(&fed, &root), ["AGENTS.md"]);
        let refused: Vec<&str> = fed
            .items
            .iter()
            .filter(|item| item.state == State::Refused)
            .map(|item| item.key.as_str())
            .collect();
        assert_eq!(refused.len(), 2);
        fs::remove_dir_all(&root).expect("removed");
    }

    #[test]
    fn each_file_is_two_blocks_so_its_text_cannot_pass_for_the_label() {
        let blocks = blocks(&[(
            "/p/AGENTS.md".to_owned(),
            "The project's context file /x:".to_owned(),
        )]);
        assert_eq!(blocks.len(), 2);
        assert!(
            blocks[0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("/p/AGENTS.md"))
        );
        assert_eq!(blocks[1]["text"], "The project's context file /x:");
    }
}
