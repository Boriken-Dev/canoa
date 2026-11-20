//! Turns on the test of a Python source entry when `CANOA_TEST_PYTHON` names an interpreter, and the test of a JavaScript source
//! entry when `CANOA_TEST_JS` is set (it needs a C compiler, since QuickJS is built into the source); without either the test is
//! ignored, which a run reports, instead of passing without having run. With the `web` feature, builds in the site
//! `CANOA_WEB_DIST` names, and refuses to build without one. Builds in Office.js and the files it loads from the deps mirror,
//! unless `CANOA_OFFICE_JS=cdn` has the add-in's pages name Microsoft's CDN instead. Nothing here runs a program.

use core::fmt::Write as _;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CANOA_TEST_PYTHON");
    println!("cargo:rerun-if-env-changed=CANOA_TEST_JS");
    println!("cargo:rustc-check-cfg=cfg(canoa_test_python)");
    println!("cargo:rustc-check-cfg=cfg(canoa_test_js)");
    if std::env::var_os("CANOA_TEST_JS").is_some_and(|js| !js.is_empty()) {
        println!("cargo:rustc-cfg=canoa_test_js");
    }
    if std::env::var_os("CANOA_TEST_PYTHON").is_some_and(|python| !python.is_empty()) {
        println!("cargo:rustc-cfg=canoa_test_python");
    }
    if std::env::var_os("CARGO_FEATURE_WEB").is_some() {
        build_in_site();
    }
    build_in_office_js();
}

/// Writes `office_js.rs` in `OUT_DIR`: one `Page` for each file Office.js loads, with its bytes. `CANOA_OFFICE_JS` is `bundled`
/// (the default: the files come from the mirror of the office-js repository, byte for byte, each checked against the digest
/// `tools/office_js.json` pins) or `cdn` (nothing is carried and the pages name Microsoft's address, which sets the
/// `office_js_cdn` cfg). The mirror is the folder `CANOA_OFFICE_JS_DIR` names, else `cdn/office-js/` of the deps checkout
/// `CANOA_DEPS` names. What is carried is the list's `carried` files: `dist/<name>` is served as `js/<name>`, and `LICENSE.md`
/// as `js/LICENSE.md`.
fn build_in_office_js() {
    println!("cargo:rerun-if-env-changed=CANOA_OFFICE_JS");
    println!("cargo:rerun-if-env-changed=CANOA_OFFICE_JS_DIR");
    println!("cargo:rerun-if-env-changed=CANOA_DEPS");
    println!("cargo:rustc-check-cfg=cfg(office_js_cdn)");
    let target = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("office_js.rs");
    let mode = std::env::var("CANOA_OFFICE_JS").unwrap_or_default();
    match mode.as_str() {
        "cdn" => {
            println!("cargo:rustc-cfg=office_js_cdn");
            std::fs::write(target, "&[]\n").expect("OUT_DIR is writable");
            return;
        }
        "" | "bundled" => {}
        _ => refuse("CANOA_OFFICE_JS is `bundled` (the default) or `cdn`"),
    }
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let list_path = manifest.join("../../tools/office_js.json");
    println!("cargo:rerun-if-changed={}", list_path.display());
    let list: serde_json::Value = std::fs::read(&list_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| refuse("tools/office_js.json cannot be read"));
    let mut carried: Vec<(String, String)> = list["carried"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|path| {
            let path = path.as_str()?;
            let sha256 = list["files"][path].as_str()?.to_owned();
            Some((path.to_owned(), sha256))
        })
        .collect();
    carried.sort();
    if !carried.iter().any(|(path, _)| path == "dist/office.js")
        || !carried.iter().any(|(path, _)| path == "LICENSE.md")
    {
        refuse("tools/office_js.json carries no dist/office.js or no LICENSE.md");
    }
    let folder = office_js_folder();
    println!("cargo:rustc-env=CANOA_OFFICE_JS_BUILT={}", folder.display());
    let mut out = String::from("&[\n");
    for (name, sha256) in &carried {
        let file = folder.join(name);
        let bytes = std::fs::read(&file).unwrap_or_else(|_| {
            refuse(&format!(
                "{name} is not in the Office.js mirror {}: refresh the deps branch (tools/mirror_deps.py fills its cdn/office-js), or build with CANOA_OFFICE_JS=cdn",
                folder.display()
            ))
        });
        let seen = Sha256::digest(&bytes)
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        if &seen != sha256 {
            refuse(&format!(
                "{} in the Office.js mirror is not the file tools/office_js.json pins (SHA-256 {seen}, pinned {sha256})",
                file.display()
            ));
        }
        let file = file
            .to_str()
            .unwrap_or_else(|| refuse("a path in the Office.js mirror is not Unicode"));
        let _ = writeln!(
            out,
            "    Page {{ name: {:?}, body: include_bytes!({file:?}), kind: Kind::Library }},",
            format!("js/{}", name.strip_prefix("dist/").unwrap_or(name))
        );
    }
    out.push_str("]\n");
    println!("cargo:rerun-if-changed={}", folder.display());
    std::fs::write(target, out).expect("OUT_DIR is writable");
}

/// The mirror of the office-js repository.
fn office_js_folder() -> PathBuf {
    let own = std::env::var_os("CANOA_OFFICE_JS_DIR").filter(|dir| !dir.is_empty());
    let deps = std::env::var_os("CANOA_DEPS").filter(|dir| !dir.is_empty());
    let folder = match (own, deps) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(deps)) => PathBuf::from(deps).join("cdn").join("office-js"),
        (None, None) => refuse(
            "the add-in carries Office.js from the deps mirror: set CANOA_DEPS to a checkout of the deps branch, whose cdn/office-js holds it (or CANOA_OFFICE_JS_DIR to that folder), \
             or build with CANOA_OFFICE_JS=cdn to have its pages load Office.js from Microsoft's CDN instead",
        ),
    };
    if !folder.is_absolute() {
        refuse("CANOA_DEPS and CANOA_OFFICE_JS_DIR must be absolute paths");
    }
    folder
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
    panic!("canoa: {why}");
}
