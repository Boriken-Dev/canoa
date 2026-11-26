//! Reads the brand a build carries and hands its names to the compiler. Shared by the build scripts of the crates that cannot depend
//! on `canoa` (the launcher and the Office add-in) and carry their own copy of the names.
//!
//! The brand is the file `BRAND_FILE` names, else `fallback`. Each name reaches the code as `cargo:rustc-env=CANOA_BRAND_<KEY>`,
//! and the prefix of the environment variables as `CANOA_RESOLVED_ENV_PREFIX`.

use std::path::{Path, PathBuf};

use crate::names::Names;

/// The brand file this build reads, its text and its names.
pub struct Read {
    pub file: PathBuf,
    pub text: String,
    pub names: Names,
}

/// Reads the brand, and tells cargo to build again when the file or the variable that names it changes.
pub fn read(fallback: &Path) -> Read {
    println!("cargo:rerun-if-env-changed=BRAND_FILE");
    let file = std::env::var_os("BRAND_FILE")
        .filter(|path| !path.is_empty())
        .map_or_else(|| fallback.to_owned(), PathBuf::from);
    println!("cargo:rerun-if-changed={}", file.display());
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("the brand file {}: {error}", file.display()));
    let names = Names::read(&text)
        .unwrap_or_else(|error| panic!("the brand file {}: {error}", file.display()));
    Read { file, text, names }
}

/// Gives the compiler every name of `names`.
pub fn env(names: &Names) {
    for (key, value) in [
        ("NAME", &names.name),
        ("SHORT_NAME", &names.short_name),
        ("EXECUTABLE", &names.executable),
        ("ID", &names.id),
        ("PROGRAM", &names.program),
        ("CONFIG_FILE", &names.config_file),
        ("FOLDER", &names.folder),
        ("HOME_DIR", &names.home_dir),
    ] {
        println!("cargo:rustc-env=CANOA_BRAND_{key}={value}");
    }
    println!(
        "cargo:rustc-env=CANOA_RESOLVED_ENV_PREFIX={}",
        names.env_prefix
    );
}
