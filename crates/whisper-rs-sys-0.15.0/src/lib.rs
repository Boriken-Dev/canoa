#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// The CPU the bundled whisper.cpp was compiled for: `native`, an x86-64 level such as `x86-64-v3`, or an AArch64 `-march` value such
/// as `armv8.2-a+dotprod+fp16`. It follows the `CANOA_WHISPER_CPU` variable and ggml's own `GGML_*` switches at build time.
pub const CPU_TARGET: &str = env!("WHISPER_RS_SYS_CPU_TARGET");

/// The version of the bundled whisper.cpp, such as `1.9.6`.
pub const CPP_VERSION: &str = env!("WHISPER_RS_SYS_CPP_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn the_build_reports_what_it_built_for() {
        assert_eq!(super::CPP_VERSION, "1.9.6");
        assert!(!super::CPU_TARGET.is_empty());
    }
}
