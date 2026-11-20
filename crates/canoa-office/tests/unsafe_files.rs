//! The files allowed to hold `unsafe` are the review surface of this crate. A test lists them, and fails when another file gains
//! `unsafe` or a listed one stops needing it.

#![cfg(windows)]

use std::path::Path;

/// File and why it needs `unsafe`.
const ALLOWED: &[(&str, &str)] = &[
    (
        "addin.rs",
        "calls through Office's interface pointers and reads the strings and out-slots Office passes",
    ),
    (
        "exports.rs",
        "the DLL's two C entry points, and the class factory's out-slot",
    ),
    (
        "interfaces.rs",
        "COM interface declarations; the macro writes `unsafe` vtables for interfaces only Office's type libraries define",
    ),
    (
        "pane.rs",
        "window creation and painting, and the OLE container calls",
    ),
];

fn code_without_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn only_the_listed_files_hold_unsafe() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut holding = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let has_unsafe = code_without_comments(&text).contains("unsafe");
        if has_unsafe {
            holding.push(name.clone());
            assert!(
                text.contains("#![allow(unsafe_code"),
                "{name} holds unsafe but does not say so with `#![allow(unsafe_code, ..)]`"
            );
        }
        // A listed file that no longer needs it is a stale entry.
        if ALLOWED.iter().any(|(listed, _)| *listed == name) {
            assert!(
                has_unsafe,
                "{name} is listed as holding unsafe and no longer does"
            );
        }
    }
    holding.sort();
    let allowed: Vec<_> = ALLOWED.iter().map(|(name, _)| (*name).to_owned()).collect();
    assert_eq!(
        holding, allowed,
        "the files with unsafe differ from the list"
    );
}

#[test]
fn every_unsafe_block_says_why() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let opens_block = trimmed.contains("unsafe {")
                || trimmed.ends_with("unsafe {")
                || trimmed.contains("= unsafe")
                || trimmed.contains("unsafe { ");
            if opens_block && !trimmed.starts_with("//") {
                let above = lines[..index]
                    .iter()
                    .rev()
                    .take(4)
                    .any(|l| l.trim_start().starts_with("// SAFETY:"));
                let same = line.contains("SAFETY:");
                assert!(
                    above || same,
                    "{}:{}: an unsafe block with no `// SAFETY:` comment",
                    path.display(),
                    index + 1
                );
            }
        }
    }
}
