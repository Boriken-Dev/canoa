//! Gives the add-in the names of its brand. The linker names the COM entry points it exports without a definition file; the
//! warning says nothing about the DLL.

#[allow(
    dead_code,
    reason = "a shared reader: this script needs only the names"
)]
#[path = "../../crates/canoa/build/emit.rs"]
mod emit;
#[allow(
    dead_code,
    reason = "a shared reader: this script needs only the names"
)]
#[path = "../../crates/canoa/build/names.rs"]
mod names;

fn main() {
    let manifest = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    emit::env(&emit::read(&manifest.join("../../crates/canoa/brand/brand.json")).names);
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-cdylib-link-arg=/IGNORE:4104");
    }
}
