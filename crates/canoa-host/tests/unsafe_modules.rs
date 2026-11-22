//! The modules that opt out of `deny(unsafe_code)` are the ones the documentation names: a new one has to be written down
//! with its reason where a reader looks for it, and a name that stays after its module is gone is dropped.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::fs;
use std::path::{Path, PathBuf};

/// Each module with an `allow(unsafe_code)` of its own, by the path the documentation gives it.
const MODULES: [&str; 5] = [
    "awake.rs",
    "replace.rs",
    "spawn/pty.rs",
    "terminal/pty_windows.rs",
    "usage_splice/members/win32.rs",
];

/// The source files under `dir` that hold an `allow(unsafe_code...)`, as paths relative to `base` with `/`.
fn allowing(base: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            allowing(base, &path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            let opens = text.lines().enumerate().any(|(index, line)| {
                let line = line.trim_start();
                line.contains("allow(unsafe_code")
                    || (line.starts_with("unsafe_code,")
                        && text
                            .lines()
                            .nth(index.wrapping_sub(1))
                            .is_some_and(|before| before.trim_end().ends_with("allow(")))
            });
            if opens {
                let relative = path.strip_prefix(base).unwrap_or(&path);
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// The module a file with an `allow(unsafe_code)` belongs to: `mail.rs` allows it for its `sspi_server` module,
/// and `usage_splice/members.rs` for `members/win32.rs`.
fn module_of(file: &str) -> &str {
    match file {
        "mail.rs" => "mail/sspi_server.rs",
        "usage_splice/members.rs" => "usage_splice/members/win32.rs",
        other => other,
    }
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_modules_that_allow_unsafe_code_are_the_ones_written_down() {
    let src = manifest().join("src");
    let mut files = Vec::new();
    allowing(&src, &src, &mut files);
    let mut modules: Vec<&str> = files.iter().map(|file| module_of(file)).collect();
    modules.sort_unstable();
    let mut expected = MODULES.to_vec();
    expected.sort_unstable();
    assert_eq!(
        modules, expected,
        "a module that opts out of deny(unsafe_code) is added to MODULES here, to this crate's AGENTS.md and to the root AGENTS.md"
    );
}

#[test]
fn the_crate_header_and_the_root_header_name_every_one() {
    let headers = [
        ("this crate's AGENTS.md", manifest().join("AGENTS.md")),
        (
            "the root AGENTS.md",
            manifest().join("..").join("..").join("AGENTS.md"),
        ),
    ];
    for (what, path) in headers {
        let Ok(text) = fs::read_to_string(&path) else {
            // A crate unpacked outside the repository has no root header to check.
            continue;
        };
        for module in MODULES {
            assert!(text.contains(module), "{what} does not name {module}");
        }
    }
}

#[test]
fn host_headers_have_no_overlong_lines_and_include_str_stays_inside_crate() {
    let root = manifest();
    let mut headers = vec![root.join("AGENTS.md")];
    for entry in fs::read_dir(root.join("src")).expect("src") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            let candidate = path.join("AGENTS.md");
            if candidate.is_file() {
                headers.push(candidate);
            }
        }
    }
    for header in headers {
        let text = fs::read_to_string(&header).expect("read header");
        for (index, line) in text.lines().enumerate() {
            assert!(
                line.chars().count() <= 1500,
                "{}:{} exceeds 1500 chars ({})",
                header.display(),
                index + 1,
                line.chars().count()
            );
        }
    }

    fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut rs_files = Vec::new();
    walk_rs(&root.join("src"), &mut rs_files);
    for file in rs_files {
        let text = fs::read_to_string(&file).expect("read rs");
        assert!(
            !text.contains("include_str!(\"../../../../")
                && !text.contains("include_str!(\"../../../bindings"),
            "{} includes a file outside crates/canoa-host",
            file.display()
        );
    }

    let workspace = root.join("..").join("..");
    let src_sdk = workspace.join("bindings/js/canoa-sdk/src");
    let host_sdk = root.join("extensions/sdk");
    if src_sdk.is_dir() {
        for name in [
            "kinds.js",
            "model.js",
            "profile.js",
            "secret.js",
            "source.js",
            "speech.js",
            "wait.js",
        ] {
            let want = fs::read(src_sdk.join(name)).expect("read sdk file");
            let have = fs::read(host_sdk.join(name)).expect("read host sdk copy");
            assert_eq!(
                have, want,
                "crates/canoa-host/extensions/sdk/{name} differs from bindings/js/canoa-sdk/src/{name}"
            );
        }
    }
}
