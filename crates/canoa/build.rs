//! Resolves the brand a build carries: the names it gives the host and the environment variables it reads.
//!
//! The brand is the file `BRAND_FILE` names, else the copy of `app/brand.json` in `brand/`, which lets the crate build packaged on
//! its own. The names reach the code as `cargo:rustc-env` values and the file and its icon as copies in `OUT_DIR`.

#[path = "build/names.rs"]
mod names;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=BRAND_FILE");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let file = std::env::var_os("BRAND_FILE")
        .filter(|path| !path.is_empty())
        .map_or_else(|| manifest.join("brand/brand.json"), PathBuf::from);
    println!("cargo:rerun-if-changed={}", file.display());
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("the brand file {}: {error}", file.display()));
    let brand = names::Names::read(&text)
        .unwrap_or_else(|error| panic!("the brand file {}: {error}", file.display()));
    let icon = brand.icon_path(&file);
    println!("cargo:rerun-if-changed={}", icon.display());
    let svg = std::fs::read_to_string(&icon)
        .unwrap_or_else(|error| panic!("the brand's icon {}: {error}", icon.display()));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    std::fs::write(out.join("brand.json"), text).expect("writes the brand copy");
    std::fs::write(out.join("brand.svg"), svg).expect("writes the icon copy");
    for (key, value) in [
        ("NAME", &brand.name),
        ("SHORT_NAME", &brand.short_name),
        ("EXECUTABLE", &brand.executable),
        ("ID", &brand.id),
        ("PROGRAM", &brand.program),
        ("CONFIG_FILE", &brand.config_file),
        ("FOLDER", &brand.folder),
        ("HOME_DIR", &brand.home_dir),
    ] {
        println!("cargo:rustc-env=CANOA_BRAND_{key}={value}");
    }
    println!(
        "cargo:rustc-env=CANOA_RESOLVED_ENV_PREFIX={}",
        brand.env_prefix
    );
}
