//! Turns on the test of a Python source entry when `CANOA_TEST_PYTHON` names an interpreter; without it the test is
//! ignored, which a run reports, instead of passing without having run. With the `web` feature, builds in the site
//! `CANOA_WEB_DIST` names, and refuses to build without one. Nothing here runs a program.

use core::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CANOA_TEST_PYTHON");
    println!("cargo:rustc-check-cfg=cfg(canoa_test_python)");
    if std::env::var_os("CANOA_TEST_PYTHON").is_some_and(|python| !python.is_empty()) {
        println!("cargo:rustc-cfg=canoa_test_python");
    }
    if std::env::var_os("CARGO_FEATURE_WEB").is_some() {
        build_in_site();
    }
}

/// Writes `web_site.rs` in `OUT_DIR`: every file under `CANOA_WEB_DIST`, by its path under it with `/` between segments,
/// in byte order, with its bytes and an entity tag of its contents.
fn build_in_site() {
    println!("cargo:rerun-if-env-changed=CANOA_WEB_DIST");
    let dist = std::env::var_os("CANOA_WEB_DIST")
        .filter(|dist| !dist.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            refuse("the web feature builds a site in: set CANOA_WEB_DIST to one, such as `flutter build web`'s output")
        });
    if !dist.is_absolute() {
        refuse("CANOA_WEB_DIST must be an absolute path");
    }
    let root = std::fs::canonicalize(&dist)
        .unwrap_or_else(|_| refuse("CANOA_WEB_DIST names no directory that can be read"));
    if !root.join("index.html").is_file() {
        refuse("CANOA_WEB_DIST holds no index.html");
    }
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rustc-env=CANOA_WEB_DIST_BUILT={}", root.display());
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    files.sort();
    let mut out = String::from("&[\n");
    for (path, file) in &files {
        let bytes = std::fs::read(file)
            .unwrap_or_else(|_| refuse("a file under CANOA_WEB_DIST cannot be read"));
        let file = file
            .to_str()
            .unwrap_or_else(|| refuse("a path under CANOA_WEB_DIST is not Unicode"));
        let _ = writeln!(
            out,
            "    ({path:?}, include_bytes!({file:?}), \"\\\"{:016x}\\\"\"),",
            fnv1a(&bytes)
        );
    }
    out.push_str("]\n");
    let target =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("web_site.rs");
    std::fs::write(target, out).expect("OUT_DIR is writable");
}

/// Collects every regular file under `dir`, with its path relative to `root`, leaving out names that start with a dot,
/// which the server never serves (`flutter build web` leaves `.last_build_id`). A link is followed only to a file inside
/// `root`; one that leads out stops the build.
fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|_| refuse("a directory under CANOA_WEB_DIST cannot be read"));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|_| refuse("a directory under CANOA_WEB_DIST cannot be read"))
            .path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let real = std::fs::canonicalize(&path)
            .unwrap_or_else(|_| refuse("a link under CANOA_WEB_DIST leads nowhere"));
        if !real.starts_with(root) {
            refuse("a link under CANOA_WEB_DIST leads out of it");
        }
        if real.is_dir() {
            walk(root, &path, files);
        } else if real.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|_| refuse("a file under CANOA_WEB_DIST is outside it"))
                .components()
                .map(|part| {
                    part.as_os_str()
                        .to_str()
                        .unwrap_or_else(|| refuse("a path under CANOA_WEB_DIST is not Unicode"))
                })
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, real));
        }
    }
}

/// FNV-1a, 64 bits: a tag that changes with the contents, for a browser to ask whether its copy is current.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn refuse(why: &str) -> ! {
    panic!("canoa-host: {why}");
}
