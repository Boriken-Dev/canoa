//! Gives the launcher the names of its brand, and takes the app's payload archive, named by CANOA_LAUNCH_PAYLOAD, into the launcher, with its SHA-256.
//!
//! Without the variable the launcher carries nothing and says so when started; that is the build every test and a
//! workspace-wide `cargo build` makes.

#[allow(
    dead_code,
    reason = "a shared reader: this script needs only the names"
)]
#[path = "../canoa/build/emit.rs"]
mod emit;
#[allow(
    dead_code,
    reason = "a shared reader: this script needs only the names"
)]
#[path = "../canoa/build/names.rs"]
mod names;

use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    emit::env(&emit::read(&manifest.join("../canoa/brand/brand.json")).names);
    println!("cargo:rerun-if-env-changed=CANOA_LAUNCH_PAYLOAD");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let bytes = match env::var_os("CANOA_LAUNCH_PAYLOAD") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            fs::read(&path).unwrap_or_else(|e| panic!("CANOA_LAUNCH_PAYLOAD {path:?}: {e}"))
        }
        None => Vec::new(),
    };
    let digest = if bytes.is_empty() {
        String::new()
    } else {
        Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    fs::write(out.join("payload.zip"), &bytes).expect("write payload.zip");
    fs::write(out.join("payload.sha256"), digest).expect("write payload.sha256");
    // Every DLL the launcher still imports is looked for in System32 alone, never beside the program (LOAD_LIBRARY_SEARCH_SYSTEM32).
    if env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
