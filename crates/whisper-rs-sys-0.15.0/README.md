# whisper-rs-sys (Boriken Dev fork)

Rust bindings for [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (the FFI layer of
[whisper-rs](https://codeberg.org/tazz4843/whisper-rs)), with whisper.cpp 1.9.6, a build that works on Windows on ARM and a
documented choice of the CPU the code is built for.

It is `whisper-rs-sys` 0.15.0 as published on crates.io, as the first commit of this history, with each change a commit on top, so
`git log -p` is the diff from upstream. The crate keeps its name and version so that a dependency on `whisper-rs-sys = "0.15"`
(whisper-rs's own) resolves to it:

```toml
[patch.crates-io]
whisper-rs-sys = { git = "https://github.com/Boriken-Dev/whisper-rs-sys", branch = "main" }
```

## What differs from upstream 0.15.0

- **whisper.cpp 1.9.6** (MIT) in place of 1.8.3, from the release tarball of the tag (commit
  `96ec4d9823e9d0211944172da35f9f29134af990`, SHA-256 `7ab15dc88022d2425ad94dcffd8abbf1b6d7018089b4e48d6b40780738e0b445`),
  trimmed to what compiles: no models, tests, examples or pre-generated bindings. `build.rs` reads the version from the
  `WHISPER_VERSION_*` lines 1.9 uses, and always generates the bindings (libclang is needed; `WHISPER_DONT_GENERATE_BINDINGS` is
  refused). The GPU backends (`cuda`, `hipblas`, `metal`, `vulkan`, `intel-sycl`) are not usable from this fork's trimmed source.
- **Windows on ARM builds.** ggml refuses `cl.exe` for ARM and needs `clang-cl`, which CMake finds with the Ninja generator. `build.rs`
  looks for both on `PATH`, then in LLVM's and Visual Studio's own folders, and uses them for `aarch64-pc-windows-msvc`.
- **The CPU target is an option** (below), and the crate reports it.

Everything else is upstream's. The bindings are the same, and `whisper-rs` builds on it unchanged.

## The CPU target

ggml's default is `GGML_NATIVE=ON`: the code is tuned for the machine that builds it and may not run on another. This fork builds
for a portable baseline instead, unless told otherwise.

| Target | Default baseline |
| --- | --- |
| x86-64 | `x86-64-v3`: SSE4.2, AVX, AVX2, BMI2, FMA, F16C (Intel Haswell, AMD Zen and later). whisper.cpp relies on AVX2 for its speed; `x86-64-v2` runs on processors without it. |
| AArch64 on Windows | `armv8.6-a+dotprod+i8mm`. On the Snapdragon X, `native` uses the SVE and SME instructions the processor has and ran 2.4 times slower than this baseline. |
| AArch64 elsewhere (Linux, macOS, Android) | `armv8.2-a+dotprod+fp16`: Apple's M1, AWS Graviton 2, the Raspberry Pi 5. |
| Any other architecture | ggml's own choice. |

Set `CANOA_WHISPER_CPU` in the environment of the build to change it:

| Value | Meaning |
| --- | --- |
| unset, empty, `default` | the baseline above |
| `native` | the machine that builds (`GGML_NATIVE=ON`) |
| `x86-64`, `x86-64-v2`, `x86-64-v3`, `x86-64-v4` | an x86-64 level (v4 adds AVX-512 F, CD, VL, DQ, BW) |
| any other name, on x86-64 | passed to the compiler as `-march=<name>` (not with the Microsoft compiler) |
| any other name, on AArch64 | ggml's `GGML_CPU_ARM_ARCH`, passed as `-march=<name>`, such as `armv8.4-a+dotprod` |

ggml's own switches (`GGML_NATIVE`, `GGML_CPU_ARM_ARCH`, `GGML_SSE42`, `GGML_AVX`, `GGML_AVX2`, `GGML_BMI2`, `GGML_FMA`, `GGML_F16C`,
`GGML_AVX512`, `GGML_AVX_VNNI`, `GGML_AVX512_VBMI`, `GGML_AVX512_VNNI`, `GGML_AVX512_BF16`) still work, are passed to ggml as they are
and win over `CANOA_WHISPER_CPU`; the reported target then says `overridden by`. Any other `WHISPER_*`, `GGML_*` and `CMAKE_*`
variable is passed to CMake as upstream does.

### Reading the target

- At run time: `whisper_rs_sys::CPU_TARGET` (for example `x86-64-v3`, `armv8.2-a+dotprod+fp16`, `native`) and
  `whisper_rs_sys::CPP_VERSION` (`1.9.6`).
- To a dependent's build script: the `links = "whisper"` metadata `DEP_WHISPER_WHISPER_CPU_TARGET`, beside upstream's
  `DEP_WHISPER_WHISPER_CPP_VERSION`.

## License

The crate is [The Unlicense](LICENSE), as upstream's. whisper.cpp, under `whisper.cpp/`, is MIT (`whisper.cpp/LICENSE`).
