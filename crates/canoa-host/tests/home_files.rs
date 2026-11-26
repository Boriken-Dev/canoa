//! Every file the host's code names by joining a literal file name to a folder is in `AGENTS.md`'s "Files the host keeps" with what it
//! holds and why it is data, not a setting. A setting is a key of canoa.cfg, read and written through the configuration store, so a
//! new file of settings, or any new file, fails here until it is written down.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What the names are looked for in: the host and every extension's own code.
fn sources() -> Vec<PathBuf> {
    let host = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(&host.join("src"), &mut files);
    if let Ok(extensions) = std::fs::read_dir(host.join("..").join("..").join("extensions")) {
        for extension in extensions.flatten() {
            walk(&extension.path().join("src"), &mut files);
        }
    }
    files
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" {
                walk(&path, files);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            files.push(path);
        }
    }
}

/// The extensions a file the host keeps may have.
const KINDS: &[&str] = &[
    "json", "jsonl", "toml", "cfg", "yaml", "yml", "pem", "lock", "log", "key", "db", "holder",
    "sock",
];

/// The file names `.join("NAME.ext")` makes in `source`, before its unit tests.
fn joined(source: &str) -> BTreeSet<String> {
    let code = source.split("\n#[cfg(test)]").next().unwrap_or(source);
    let mut names = BTreeSet::new();
    let mut rest = code;
    while let Some(at) = rest.find(".join(\"") {
        rest = &rest[at + ".join(\"".len()..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let name = &rest[..end];
        let kind = name.rsplit_once('.').map(|(_, kind)| kind);
        if !name.contains(['/', '\\', ' ', '{']) && kind.is_some_and(|kind| KINDS.contains(&kind)) {
            names.insert(name.to_owned());
        }
    }
    names
}

/// The file names the section "Files the host keeps" of the host's `AGENTS.md` lists: the backticked names in the first cell of each
/// table row.
fn listed() -> BTreeSet<String> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("AGENTS.md"))
        .expect("the host's AGENTS.md");
    let start = text
        .find("## Files the host keeps")
        .expect("AGENTS.md has a section 'Files the host keeps'");
    let section = &text[start..];
    let section = section[3..]
        .find("\n## ")
        .map_or(section, |end| &section[..end + 3]);
    section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|row| row[1..].split('|').next())
        .flat_map(|cell| {
            cell.split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn every_file_the_host_names_is_written_down_with_why_it_is_data() {
    let known = listed();
    let mut missing = BTreeSet::new();
    for file in sources() {
        let source = std::fs::read_to_string(&file).expect("a source file");
        for name in joined(&source) {
            if !known.contains(&name) {
                missing.insert(format!("{name} ({})", file.display()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "these files are named by the code and not listed in AGENTS.md's \"Files the host keeps\": a file of settings is a table of canoa.cfg, \
         so list a file here only when it is data (history, a cache, a counter, a key, a record a ceremony made), with why: {missing:#?}"
    );
}

/// The control: a file that is not written down is found, and what a unit test names is not counted.
#[test]
fn a_new_file_is_found_and_a_test_module_is_not_read() {
    let names = joined(
        "let a = dir.join(\"settings.json\");\nlet b = dir.join(\"canoa.cfg\");\n#[cfg(test)]\nmod tests { fn t() { dir.join(\"only-in-a-test.json\"); } }",
    );
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        ["canoa.cfg", "settings.json"]
    );
    assert!(!listed().contains("settings.json"));
}

/// Nothing listed is a file of settings: the files that held settings before they were tables of canoa.cfg are not kept.
#[test]
fn no_file_of_settings_is_listed() {
    for gone in [
        "trust.json",
        "viewers.json",
        "permissions.json",
        "launch.json",
        "portal.toml",
        "profile.toml",
    ] {
        assert!(!listed().contains(gone), "{gone} was a file of settings");
    }
}
