//! Turns on the test of a Python source entry when `CANOA_TEST_PYTHON` names an interpreter; without it the test is
//! ignored, which a run reports, instead of passing without having run. Nothing here runs a program.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CANOA_TEST_PYTHON");
    println!("cargo:rustc-check-cfg=cfg(canoa_test_python)");
    if std::env::var_os("CANOA_TEST_PYTHON").is_some_and(|python| !python.is_empty()) {
        println!("cargo:rustc-cfg=canoa_test_python");
    }
}
