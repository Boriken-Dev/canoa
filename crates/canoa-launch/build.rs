//! Takes the app's payload archive, named by CANOA_LAUNCH_PAYLOAD, into the launcher, with its SHA-256.
//!
//! Without the variable the launcher carries nothing and says so when started; that is the build every test and a
//! workspace-wide `cargo build` makes.

use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() {
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
    println!("cargo:rerun-if-changed=build.rs");
}
