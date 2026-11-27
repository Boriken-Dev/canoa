#![allow(clippy::uninlined_format_args)]

extern crate bindgen;
extern crate semver;

use cmake::Config;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap();
    // Link C++ standard library
    if let Some(cpp_stdlib) = get_cpp_link_stdlib(&target) {
        println!("cargo:rustc-link-lib=dylib={}", cpp_stdlib);
    }
    // Link macOS Accelerate framework for matrix calculations
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=framework=Accelerate");
        #[cfg(feature = "coreml")]
        {
            println!("cargo:rustc-link-lib=framework=Foundation");
            println!("cargo:rustc-link-lib=framework=CoreML");
        }
        #[cfg(feature = "metal")]
        {
            println!("cargo:rustc-link-lib=framework=Foundation");
            println!("cargo:rustc-link-lib=framework=Metal");
            println!("cargo:rustc-link-lib=framework=MetalKit");
        }
    }

    #[cfg(feature = "coreml")]
    println!("cargo:rustc-link-lib=static=whisper.coreml");

    #[cfg(feature = "openblas")]
    {
        if let Ok(openblas_path) = env::var("OPENBLAS_PATH") {
            println!(
                "cargo::rustc-link-search={}",
                PathBuf::from(openblas_path).join("lib").display()
            );
        }
        if cfg!(windows) {
            println!("cargo:rustc-link-lib=libopenblas");
        } else {
            println!("cargo:rustc-link-lib=openblas");
        }
    }
    #[cfg(feature = "cuda")]
    {
        println!("cargo:rustc-link-lib=cublas");
        println!("cargo:rustc-link-lib=cudart");
        println!("cargo:rustc-link-lib=cublasLt");
        println!("cargo:rustc-link-lib=cuda");
        cfg_if::cfg_if! {
            if #[cfg(target_os = "windows")] {
                let cuda_path = PathBuf::from(env::var("CUDA_PATH").unwrap()).join("lib/x64");
                println!("cargo:rustc-link-search={}", cuda_path.display());
            } else {
                println!("cargo:rustc-link-lib=culibos");
                println!("cargo:rustc-link-search=/usr/local/cuda/lib64");
                println!("cargo:rustc-link-search=/usr/local/cuda/lib64/stubs");
                println!("cargo:rustc-link-search=/opt/cuda/lib64");
                println!("cargo:rustc-link-search=/opt/cuda/lib64/stubs");
            }
        }
    }
    #[cfg(feature = "hipblas")]
    {
        println!("cargo:rustc-link-lib=hipblas");
        println!("cargo:rustc-link-lib=rocblas");
        println!("cargo:rustc-link-lib=amdhip64");

        cfg_if::cfg_if! {
            if #[cfg(target_os = "windows")] {
                panic!("Due to a problem with the last revision of the ROCm 5.7 library, it is not possible to compile the library for the windows environment.\nSee https://github.com/ggerganov/whisper.cpp/issues/2202 for more details.")
            } else {
                println!("cargo:rerun-if-env-changed=HIP_PATH");

                let hip_path = match env::var("HIP_PATH") {
                    Ok(path) =>PathBuf::from(path),
                    Err(_) => PathBuf::from("/opt/rocm"),
                };
                let hip_lib_path = hip_path.join("lib");

                println!("cargo:rustc-link-search={}",hip_lib_path.display());
            }
        }
    }

    #[cfg(feature = "openmp")]
    {
        if target.contains("gnu") {
            println!("cargo:rustc-link-lib=gomp");
        } else if target.contains("apple") {
            println!("cargo:rustc-link-lib=omp");
            println!("cargo:rustc-link-search=/opt/homebrew/opt/libomp/lib");
        }
    }

    println!("cargo:rerun-if-changed=wrapper.h");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let whisper_root = out.join("whisper.cpp");

    if !whisper_root.exists() {
        std::fs::create_dir_all(&whisper_root).unwrap();
        fs_extra::dir::copy("./whisper.cpp", &out, &Default::default()).unwrap_or_else(|e| {
            panic!(
                "Failed to copy whisper sources into {}: {}",
                whisper_root.display(),
                e
            )
        });
    }

    if env::var("WHISPER_DONT_GENERATE_BINDINGS").is_ok() {
        panic!("this crate carries no pre-generated bindings: unset WHISPER_DONT_GENERATE_BINDINGS");
    } else {
        // https://github.com/rust-lang/rust-bindgen/issues/2691
        // https://github.com/rust-lang/rust-bindgen/issues/3264
        // https://gitlab.freedesktop.org/gstreamer/gst-plugins-rs/-/issues/779
        let package_msrv = match option_env!("CARGO_PKG_RUST_VERSION") {
            Some(v) if !v.is_empty() => {
                let v = semver::Version::parse(v).expect("Invalid CARGO_PKG_RUST_VERSION");
                bindgen::RustTarget::stable(v.minor, v.patch)
            }
            // as_chunks in utilities.rs requires 1.88+
            _ => bindgen::RustTarget::stable(88, 0),
        }
        .map_err(|v| v.to_string())
        .unwrap();

        let mut bindings = bindgen::Builder::default()
            .rust_edition(bindgen::RustEdition::Edition2021)
            .rust_target(package_msrv)
            .header("wrapper.h");

        #[cfg(feature = "metal")]
        {
            bindings = bindings.header("whisper.cpp/ggml/include/ggml-metal.h");
        }
        #[cfg(feature = "vulkan")]
        {
            bindings = bindings
                .header("whisper.cpp/ggml/include/ggml-vulkan.h")
                .clang_arg("-DGGML_USE_VULKAN=1");
        }

        let bindings = bindings
            .clang_arg("-I./whisper.cpp/")
            .clang_arg("-I./whisper.cpp/include")
            .clang_arg("-I./whisper.cpp/ggml/include")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate();

        match bindings {
            Ok(b) => {
                let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
                b.write_to_file(out_path.join("bindings.rs"))
                    .expect("Couldn't write bindings!");
            }
            Err(e) => panic!("Unable to generate bindings (libclang is needed): {}", e),
        }
    };

    let cpu = CpuTarget::from_env();
    println!("cargo:rustc-env=WHISPER_RS_SYS_CPU_TARGET={}", cpu.label);
    println!("cargo:WHISPER_CPU_TARGET={}", cpu.label);
    println!(
        "cargo:rustc-env=WHISPER_RS_SYS_CPP_VERSION={}",
        get_whisper_cpp_version(&whisper_root)
            .expect("Failed to read whisper.cpp CMake config")
            .expect("Could not find whisper.cpp version declaration"),
    );

    // stop if we're on docs.rs
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    let mut config = Config::new(&whisper_root);

    config
        .profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("WHISPER_ALL_WARNINGS", "OFF")
        .define("WHISPER_ALL_WARNINGS_3RD_PARTY", "OFF")
        .define("WHISPER_BUILD_TESTS", "OFF")
        .define("WHISPER_BUILD_EXAMPLES", "OFF")
        .very_verbose(true)
        .pic(true);

    if cfg!(target_os = "windows") {
        config.cxxflag("/utf-8");
        println!("cargo:rustc-link-lib=advapi32");
    }

    if cfg!(feature = "coreml") {
        config.define("WHISPER_COREML", "ON");
        config.define("WHISPER_COREML_ALLOW_FALLBACK", "1");
    }

    if cfg!(feature = "cuda") {
        config.define("GGML_CUDA", "ON");
        config.define("CMAKE_POSITION_INDEPENDENT_CODE", "ON");
        config.define("CMAKE_CUDA_FLAGS", "-Xcompiler=-fPIC");
    }

    if cfg!(feature = "hipblas") {
        config.define("GGML_HIP", "ON");
        config.define("CMAKE_C_COMPILER", "hipcc");
        config.define("CMAKE_CXX_COMPILER", "hipcc");
        println!("cargo:rerun-if-env-changed=AMDGPU_TARGETS");
        if let Ok(gpu_targets) = env::var("AMDGPU_TARGETS") {
            config.define("AMDGPU_TARGETS", gpu_targets);
        }
    }

    if cfg!(feature = "vulkan") {
        config.define("GGML_VULKAN", "ON");
        if cfg!(windows) {
            println!("cargo:rerun-if-env-changed=VULKAN_SDK");
            println!("cargo:rustc-link-lib=vulkan-1");
            let vulkan_path = match env::var("VULKAN_SDK") {
                Ok(path) => PathBuf::from(path),
                Err(_) => panic!(
                    "Please install Vulkan SDK and ensure that VULKAN_SDK env variable is set"
                ),
            };
            let vulkan_lib_path = vulkan_path.join("Lib");
            println!("cargo:rustc-link-search={}", vulkan_lib_path.display());
        } else if cfg!(target_os = "macos") {
            println!("cargo:rerun-if-env-changed=VULKAN_SDK");
            println!("cargo:rustc-link-lib=vulkan");
            let vulkan_path = match env::var("VULKAN_SDK") {
                Ok(path) => PathBuf::from(path),
                Err(_) => panic!(
                    "Please install Vulkan SDK and ensure that VULKAN_SDK env variable is set"
                ),
            };
            let vulkan_lib_path = vulkan_path.join("lib");
            println!("cargo:rustc-link-search={}", vulkan_lib_path.display());
        } else {
            println!("cargo:rustc-link-lib=vulkan");
        }
    }

    if cfg!(feature = "openblas") {
        config.define("GGML_BLAS", "ON");
        config.define("GGML_BLAS_VENDOR", "OpenBLAS");
        if env::var("BLAS_INCLUDE_DIRS").is_err() {
            panic!("BLAS_INCLUDE_DIRS environment variable must be set when using OpenBLAS");
        }
        config.define("BLAS_INCLUDE_DIRS", env::var("BLAS_INCLUDE_DIRS").unwrap());
        println!("cargo:rerun-if-env-changed=BLAS_INCLUDE_DIRS");
    }

    if cfg!(feature = "metal") {
        config.define("GGML_METAL", "ON");
        config.define("GGML_METAL_NDEBUG", "ON");
        config.define("GGML_METAL_EMBED_LIBRARY", "ON");
    } else {
        // Metal is enabled by default, so we need to explicitly disable it
        config.define("GGML_METAL", "OFF");
    }

    if cfg!(debug_assertions) || cfg!(feature = "force-debug") {
        // debug builds are too slow to even remotely be usable,
        // so we build with optimizations even in debug mode
        config.define("CMAKE_BUILD_TYPE", "RelWithDebInfo");
        config.cxxflag("-DWHISPER_DEBUG");
    } else {
        // we're in release mode, explicitly set to release mode
        // see also https://codeberg.org/tazz4843/whisper-rs/issues/226
        config.define("CMAKE_BUILD_TYPE", "Release");
    }

    if target == "aarch64-pc-windows-msvc" {
        windows_arm64(&mut config);
    }

    cpu.apply(&mut config);

    // Allow passing any WHISPER or CMAKE compile flags
    for (key, value) in env::vars() {
        let is_whisper_flag =
            key.starts_with("WHISPER_") && key != "WHISPER_DONT_GENERATE_BINDINGS";
        let is_ggml_flag = key.starts_with("GGML_");
        let is_cmake_flag = key.starts_with("CMAKE_");
        if is_whisper_flag || is_ggml_flag || is_cmake_flag {
            config.define(&key, &value);
        }
    }

    if cfg!(not(feature = "openmp")) {
        config.define("GGML_OPENMP", "OFF");
    }

    if cfg!(feature = "intel-sycl") {
        config.define("BUILD_SHARED_LIBS", "ON");
        config.define("GGML_SYCL", "ON");
        config.define("GGML_SYCL_TARGET", "INTEL");
        config.define("CMAKE_C_COMPILER", "icx");
        config.define("CMAKE_CXX_COMPILER", "icpx");
    }

    let destination = config.build();

    add_link_search_path(&out.join("build")).unwrap();

    println!("cargo:rustc-link-search=native={}", destination.display());
    if cfg!(feature = "intel-sycl") {
        println!("cargo:rustc-link-lib=whisper");
        println!("cargo:rustc-link-lib=ggml");
        println!("cargo:rustc-link-lib=ggml-base");
        println!("cargo:rustc-link-lib=ggml-cpu");
    } else {
        println!("cargo:rustc-link-lib=static=whisper");
        println!("cargo:rustc-link-lib=static=ggml");
        println!("cargo:rustc-link-lib=static=ggml-base");
        println!("cargo:rustc-link-lib=static=ggml-cpu");
    }
    if cfg!(target_os = "macos") || cfg!(feature = "openblas") {
        println!("cargo:rustc-link-lib=static=ggml-blas");
    }
    if cfg!(feature = "vulkan") {
        if cfg!(feature = "intel-sycl") {
            println!("cargo:rustc-link-lib=ggml-vulkan");
        } else {
            println!("cargo:rustc-link-lib=static=ggml-vulkan");
        }
    }

    if cfg!(feature = "hipblas") {
        println!("cargo:rustc-link-lib=static=ggml-hip");
    }

    if cfg!(feature = "metal") {
        println!("cargo:rustc-link-lib=static=ggml-metal");
    }

    if cfg!(feature = "cuda") {
        println!("cargo:rustc-link-lib=static=ggml-cuda");
    }

    if cfg!(feature = "openblas") {
        println!("cargo:rustc-link-lib=static=ggml-blas");
    }

    if cfg!(feature = "intel-sycl") {
        println!("cargo:rustc-link-lib=ggml-sycl");
    }

    println!(
        "cargo:WHISPER_CPP_VERSION={}",
        get_whisper_cpp_version(&whisper_root)
            .expect("Failed to read whisper.cpp CMake config")
            .expect("Could not find whisper.cpp version declaration"),
    );

    // for whatever reason this file is generated during build and triggers cargo complaining
    _ = std::fs::remove_file("bindings/javascript/package.json");
}

// From https://github.com/alexcrichton/cc-rs/blob/fba7feded71ee4f63cfe885673ead6d7b4f2f454/src/lib.rs#L2462
fn get_cpp_link_stdlib(target: &str) -> Option<&'static str> {
    if target.contains("msvc") {
        None
    } else if target.contains("apple") || target.contains("freebsd") || target.contains("openbsd") {
        Some("c++")
    } else if target.contains("android") {
        Some("c++_shared")
    } else {
        Some("stdc++")
    }
}

fn add_link_search_path(dir: &std::path::Path) -> std::io::Result<()> {
    if dir.is_dir() {
        println!("cargo:rustc-link-search={}", dir.display());
        for entry in std::fs::read_dir(dir)? {
            add_link_search_path(&entry?.path())?;
        }
    }
    Ok(())
}

/// The version whisper.cpp's CMakeLists.txt sets as `WHISPER_VERSION_MAJOR`, `_MINOR` and `_PATCH` (release 1.9 no longer has a
/// `project(... VERSION ...)` line).
fn get_whisper_cpp_version(whisper_root: &std::path::Path) -> std::io::Result<Option<String>> {
    let cmake_lists = BufReader::new(File::open(whisper_root.join("CMakeLists.txt"))?);
    let mut parts: [Option<String>; 3] = [None, None, None];
    for line in cmake_lists.lines() {
        let line = line?;
        for (index, key) in ["MAJOR", "MINOR", "PATCH"].iter().enumerate() {
            let prefix = format!("set(WHISPER_VERSION_{} ", key);
            if let Some(rest) = line.strip_prefix(&prefix) {
                parts[index] = Some(rest.trim_end_matches(')').trim().to_string());
            }
        }
    }
    Ok(match parts {
        [Some(a), Some(b), Some(c)] => Some(format!("{}.{}.{}", a, b, c)),
        _ => None,
    })
}

/// The first of `names` that is a file in a folder of `PATH`, or else in one of `extra`.
fn find_program(names: &[&str], extra: &[PathBuf]) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .chain(extra.iter().cloned())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// ggml refuses `cl.exe` for ARM and needs `clang-cl`, which `cmake` finds with Ninja (the Visual Studio generator picks `cl.exe`).
/// Both are looked for on `PATH`, then in LLVM's and Visual Studio's own folders. The code is built for Armv8.6-A with the dot product
/// The CPU the code is built for is `CpuTarget`'s.
fn windows_arm64(config: &mut Config) {
    let program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|name| env::var_os(name).map(PathBuf::from))
        .collect();
    let llvm: Vec<PathBuf> = program_files.iter().map(|dir| dir.join("LLVM").join("bin")).collect();
    let mut visual_studio = Vec::new();
    for base in &program_files {
        for edition in ["BuildTools", "Community", "Professional", "Enterprise"] {
            for year in ["2022", "2019"] {
                visual_studio.push(
                    base.join("Microsoft Visual Studio").join(year).join(edition).join(
                        r"Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
                    ),
                );
            }
        }
    }
    config.generator("Ninja");
    if let Some(ninja) = find_program(&["ninja.exe"], &visual_studio) {
        config.define("CMAKE_MAKE_PROGRAM", ninja);
    }
    let clang = find_program(&["clang-cl.exe"], &llvm)
        .unwrap_or_else(|| PathBuf::from("clang-cl"));
    config.define("CMAKE_C_COMPILER", &clang);
    config.define("CMAKE_CXX_COMPILER", &clang);
    config.define("CMAKE_ASM_COMPILER", &clang);
    config.cxxflag("-EHsc");
    config.cflag("-Wno-error=incompatible-pointer-types");
}

/// The variable that names the CPU the code is built for. Unset, empty or `default`: the portable baseline of the architecture.
/// `native`: the machine that builds. Anything else: an x86-64 level (`x86-64`, `x86-64-v2`, `x86-64-v3`, `x86-64-v4`) or a
/// compiler's name for the target (`-march=` on x86-64, ggml's `GGML_CPU_ARM_ARCH` on AArch64).
const CPU_VAR: &str = "CANOA_WHISPER_CPU";

/// ggml's own switches for the instruction sets. One set in the environment is passed to ggml as it is and wins over `CPU_VAR`.
const GGML_CPU_VARS: &[&str] = &[
    "GGML_NATIVE",
    "GGML_CPU_ARM_ARCH",
    "GGML_SSE42",
    "GGML_AVX",
    "GGML_AVX_VNNI",
    "GGML_AVX2",
    "GGML_BMI2",
    "GGML_AVX512",
    "GGML_AVX512_VBMI",
    "GGML_AVX512_VNNI",
    "GGML_AVX512_BF16",
    "GGML_FMA",
    "GGML_F16C",
];

/// What the build tells ggml about the CPU, and the name the library reports for it at run time.
struct CpuTarget {
    defines: Vec<(&'static str, String)>,
    flags: Vec<String>,
    label: String,
}

impl CpuTarget {
    fn from_env() -> Self {
        println!("cargo:rerun-if-env-changed={}", CPU_VAR);
        for name in GGML_CPU_VARS {
            println!("cargo:rerun-if-env-changed={}", name);
        }
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let msvc = env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|e| e == "msvc");
        let requested = env::var(CPU_VAR).unwrap_or_default();
        let overrides: Vec<String> = GGML_CPU_VARS
            .iter()
            .filter_map(|name| env::var(name).ok().map(|value| format!("{}={}", name, value)))
            .collect();
        let mut target = Self::plan(&arch, &os, msvc, requested.trim());
        if !overrides.is_empty() {
            target.label = format!("{} overridden by {}", target.label, overrides.join(" "));
        }
        target
    }

    /// The target for one architecture. `requested` is `CPU_VAR`'s value.
    fn plan(arch: &str, os: &str, msvc: bool, requested: &str) -> Self {
        let on = |yes: bool| (if yes { "ON" } else { "OFF" }).to_string();
        let name = if requested.is_empty() || requested.eq_ignore_ascii_case("default") {
            None
        } else {
            Some(requested)
        };
        let mut target = CpuTarget {
            defines: Vec::new(),
            flags: Vec::new(),
            label: String::new(),
        };
        if name.is_some_and(|n| n.eq_ignore_ascii_case("native")) {
            target.defines.push(("GGML_NATIVE", on(true)));
            target.label = "native".into();
            return target;
        }
        match arch {
            // ggml enables AVX2, FMA, BMI2 and F16C by itself once GGML_NATIVE is off; the levels say it outright.
            "x86_64" => {
                let level = name.unwrap_or("x86-64-v3");
                // (SSE4.2, AVX, AVX2, BMI2, FMA, F16C, AVX-512)
                let sets = match level {
                    "x86-64" => Some([false; 7]),
                    "x86-64-v2" => Some([true, false, false, false, false, false, false]),
                    "x86-64-v3" => Some([true, true, true, true, true, true, false]),
                    "x86-64-v4" => Some([true; 7]),
                    _ => None,
                };
                let names = [
                    "GGML_SSE42",
                    "GGML_AVX",
                    "GGML_AVX2",
                    "GGML_BMI2",
                    "GGML_FMA",
                    "GGML_F16C",
                    "GGML_AVX512",
                ];
                target.defines.push(("GGML_NATIVE", on(false)));
                for (index, define) in names.iter().enumerate() {
                    target.defines.push((define, on(sets.is_some_and(|s| s[index]))));
                }
                if sets.is_none() {
                    if msvc {
                        panic!(
                            "{}={}: the Microsoft compiler has no -march; name an x86-64 level (x86-64, x86-64-v2, x86-64-v3, x86-64-v4) or native",
                            CPU_VAR, level
                        );
                    }
                    target.flags.push(format!("-march={}", level));
                }
                target.label = level.to_string();
            }
            "aarch64" => {
                // Windows on ARM: the processors it runs on have the dot product and int8 matrix multiply. Elsewhere Armv8.2-A with the
                // dot product and half-precision arithmetic runs on Apple's M1, Graviton 2 and the Raspberry Pi 5; i8mm is left out.
                let arch_name = name.unwrap_or(if os == "windows" {
                    "armv8.6-a+dotprod+i8mm"
                } else {
                    "armv8.2-a+dotprod+fp16"
                });
                target.defines.push(("GGML_NATIVE", on(false)));
                target.defines.push(("GGML_CPU_ARM_ARCH", arch_name.to_string()));
                target.label = arch_name.to_string();
            }
            _ => {
                if let Some(other) = name {
                    panic!(
                        "{}={}: only x86_64 and aarch64 have a CPU target here (native works everywhere); {} builds with ggml's own choice",
                        CPU_VAR, other, arch
                    );
                }
                target.label = "ggml default".into();
            }
        }
        target
    }

    fn apply(&self, config: &mut Config) {
        for (key, value) in &self.defines {
            config.define(key, value);
        }
        for flag in &self.flags {
            config.cflag(flag);
            config.cxxflag(flag);
        }
    }
}
