//! Resolves the brand a build carries: the names it gives the host and the environment variables it reads.
//!
//! The brand is the file `BRAND_FILE` names, else the copy of `app/brand.json` in `brand/`, which lets the crate build packaged on
//! its own. The names reach the code as `cargo:rustc-env` values and the file and its icon as copies in `OUT_DIR`.

#[path = "build/emit.rs"]
mod emit;
#[path = "build/names.rs"]
mod names;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let brand = emit::read(&manifest.join("brand/brand.json"));
    let icon = brand.names.icon_path(&brand.file);
    println!("cargo:rerun-if-changed={}", icon.display());
    let svg = std::fs::read_to_string(&icon)
        .unwrap_or_else(|error| panic!("the brand's icon {}: {error}", icon.display()));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    std::fs::write(out.join("brand.json"), brand.text).expect("writes the brand copy");
    std::fs::write(out.join("brand.svg"), svg).expect("writes the icon copy");
    emit::env(&brand.names);
}
