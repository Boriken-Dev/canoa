//! The linker names the COM entry points it exports without a definition file; the warning says nothing about the DLL.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-cdylib-link-arg=/IGNORE:4104");
    }
}
