# Changelog

## Unreleased

- The CPU target is an option: `CANOA_WHISPER_CPU` names it (`native`, an x86-64 level, or an AArch64 `-march` value), and the
  default is a documented portable baseline per architecture (x86-64-v3, `armv8.6-a+dotprod+i8mm` on Windows on ARM,
  `armv8.2-a+dotprod+fp16` on the other AArch64 targets) instead of ggml's `GGML_NATIVE=ON`. `whisper_rs_sys::CPU_TARGET` and
  `CPP_VERSION` report what the build used; ggml's `GGML_*` switches still override.
- Builds on Windows on ARM: `build.rs` finds `clang-cl` and Ninja.
- whisper.cpp 1.9.6 in place of 1.8.3, trimmed to what compiles.

## 0.15.0

whisper-rs-sys 0.15.0 as published on crates.io (whisper.cpp 1.8.3).
