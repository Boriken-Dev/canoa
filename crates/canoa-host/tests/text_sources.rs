//! The host's sources are text: a NUL byte in one makes grep call the file binary and hides it from a search.

use std::fs;
use std::path::Path;

fn walk(dir: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("a folder").flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "rs" || ext == "md" || ext == "toml")
            && fs::read(&path).expect("a file").contains(&0)
        {
            found.push(path.display().to_string());
        }
    }
}

#[test]
fn no_source_file_holds_a_nul_byte() {
    let mut found = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"),
        &mut found,
    );
    assert!(found.is_empty(), "NUL bytes in {found:?}");
}
