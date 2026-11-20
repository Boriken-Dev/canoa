//! The Piper store: a local neural text-to-speech engine, with a voice for English and one for Spanish, so
//! the host can read replies aloud with nothing leaving the computer.
//!
//! Piper's last binary release is `2023.11.14-2` (the program prints `1.2.0`): the project was archived in 2025 and its successor
//! is a Python package under another licence. The release is MIT licensed and publishes no checksums, so the digests here are the
//! SHA-256 of the published files, measured once and pinned, and each voice is a file of one commit of the voices repository, so
//! what is downloaded is what was measured. Every voice has the licence of its own data, which the plan names.
//!
//! The folder is `piper/` (the program and its libraries) and `voices/` (each `NAME.onnx` with its `NAME.onnx.json`); the
//! type's `speech_say_piper` provider is given both through `PiperModels::service`. A Linux archive carries its libraries' short names
//! as links, which the unpack leaves out, so the install writes them as copies.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use canoa::status::{bad_value, internal, unsupported};
use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, dotted, measured_platforms, pinned,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The folder of a version's copy that holds the program and its libraries.
pub const PROGRAM_DIR: &str = "piper";

/// The folder of a version's copy that holds the voices.
pub const VOICES_DIR: &str = "voices";

/// The version Piper prints, and the one this store offers.
pub const VERSION: &str = "1.2.0";

/// Where the release's files are, and the commit of the voices repository the voices are pinned to.
pub const RELEASE: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/";
const VOICES: &str =
    "https://huggingface.co/rhasspy/piper-voices/resolve/4a409e3cbfd0a189930b38f127a38e107677e8ac/";
/// The same commit's small files, which Hugging Face answers where they are rather than through a redirect.
const VOICES_RAW: &str =
    "https://huggingface.co/rhasspy/piper-voices/raw/4a409e3cbfd0a189930b38f127a38e107677e8ac/";

/// The origins a download or a redirect may reach: the release, GitHub's asset hosts, the voices repository and the hosts
/// Hugging Face sends a model on to.
const ENDPOINTS: [&str; 8] = [
    RELEASE,
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
    VOICES,
    VOICES_RAW,
    "https://us.aws.cdn.hf.co",
    "https://cdn-lfs.hf.co",
    "https://cas-bridge.xethub.hf.co",
];

/// One archive of the program.
#[derive(Debug, Clone)]
pub struct Engine {
    /// `windows` or `linux`.
    pub os: &'static str,
    /// `x86_64` or `aarch64`.
    pub arch: &'static str,
    /// The file name, under the release.
    pub file: &'static str,
    /// Its SHA-256.
    pub sha256: &'static str,
    /// Its size.
    pub bytes: u64,
}

/// One voice: a model and its description.
#[derive(Debug, Clone)]
pub struct VoiceFile {
    /// The voice's id, `en_US-ljspeech-medium`.
    pub name: &'static str,
    /// The folder of its files under the voices repository.
    pub folder: &'static str,
    /// The language it speaks: `en` or `es`, which the channels choose by.
    pub language: &'static str,
    /// What its data's licence is, for the plan.
    pub licence: &'static str,
    /// The model's SHA-256 and size.
    pub onnx: (&'static str, u64),
    /// The model's description file, its SHA-256 and size.
    pub json: (&'static str, u64),
}

/// What a Piper release is made of: the archives and the voices it is offered with.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// The version the program prints.
    pub version: String,
    /// Where the archives are, ending in a slash.
    pub release: String,
    /// Where the voices' models are, ending in a slash.
    pub voices_at: String,
    /// Where their description files are, ending in a slash.
    pub raw_at: String,
    /// The origins the downloads may come from.
    pub endpoints: Vec<String>,
    /// The archives.
    pub engines: Vec<Engine>,
    /// The voices.
    pub voices: Vec<VoiceFile>,
}

impl Default for Catalog {
    fn default() -> Self {
        let windows = |arch| Engine {
            os: "windows",
            arch,
            file: "piper_windows_amd64.zip",
            sha256: "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea",
            bytes: 22_477_236,
        };
        Self {
            version: VERSION.to_owned(),
            release: RELEASE.to_owned(),
            voices_at: VOICES.to_owned(),
            raw_at: VOICES_RAW.to_owned(),
            endpoints: ENDPOINTS.map(str::to_owned).to_vec(),
            // Windows on ARM runs the x64 build under the system's emulation, which was measured.
            engines: vec![
                windows("x86_64"),
                windows("aarch64"),
                Engine {
                    os: "linux",
                    arch: "x86_64",
                    file: "piper_linux_x86_64.tar.gz",
                    sha256: "a50cb45f355b7af1f6d758c1b360717877ba0a398cc8cbe6d2a7a3a26e225992",
                    bytes: 26_460_462,
                },
                Engine {
                    os: "linux",
                    arch: "aarch64",
                    file: "piper_linux_aarch64.tar.gz",
                    sha256: "fea0fd2d87c54dbc7078d0f878289f404bd4d6eea6e7444a77835d1537ab88eb",
                    bytes: 26_004_717,
                },
            ],
            voices: vec![
                VoiceFile {
                    name: "en_US-ljspeech-medium",
                    folder: "en/en_US/ljspeech/medium/",
                    language: "en",
                    licence: "public domain (the LJ Speech recordings)",
                    onnx: (
                        "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22",
                        63_531_379,
                    ),
                    json: (
                        "141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860",
                        4972,
                    ),
                },
                VoiceFile {
                    name: "es_MX-claude-high",
                    folder: "es/es_MX/claude/high/",
                    language: "es",
                    licence: "Apache License 2.0",
                    onnx: (
                        "3ef40a71ea63852cd8ab7e6fa7d2ecdcfa67a0b47c9c48e3f10e02ee02083ea0",
                        63_122_309,
                    ),
                    json: (
                        "1afc81f703c0e4cb3b4d7c0dca096b8b54a98806807f0170cf5eb5557723c12d",
                        4963,
                    ),
                },
            ],
        }
    }
}

/// Piper.
#[derive(Debug, Default, Clone)]
pub struct PiperRecipe {
    catalog: Catalog,
}

impl PiperRecipe {
    /// A recipe over `catalog`: the one this host ships, or a stand-in's.
    #[must_use]
    pub const fn over(catalog: Catalog) -> Self {
        Self { catalog }
    }
}

impl Recipe for PiperRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("piper", "Piper", "Rhasspy");
        info.homepage = Some("https://github.com/rhasspy/piper".to_owned());
        info.summary = Some(
            "A local neural voice that reads replies aloud on this computer, with an English and a Spanish voice; the text is sent nowhere."
                .to_owned(),
        );
        info.icon = Some("speaker".to_owned());
        info.platforms = measured_platforms();
        info.channels = ["standard", "english", "spanish"]
            .map(str::to_owned)
            .to_vec();
        info.endpoints.clone_from(&self.catalog.endpoints);
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["piper"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        let line = line.trim();
        dotted(line).filter(|version| version == line)
    }

    fn classify(&self, _path: &str) -> (&'static str, Updatable) {
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "its origin is not known".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn latest(&self, channel: Option<&str>, _tools: &Object<dyn InstallTools>) -> LatestReport {
        let mut report = LatestReport::known(self.catalog.version.clone());
        report.channel = Some(channel.unwrap_or("standard").to_owned());
        report
    }

    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        _tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let catalog = &self.catalog;
        if let Some(version) = version {
            let version = pinned(version)?;
            if version != catalog.version {
                return Err(unsupported(format!(
                    "Piper has no release {version}: its last is {}",
                    catalog.version
                )));
            }
        }
        let wanted = match channel.unwrap_or("standard") {
            "standard" => None,
            "english" => Some("en"),
            "spanish" => Some("es"),
            other => {
                return Err(bad_value(format!(
                    "Piper has no channel {other:?}: standard, english or spanish"
                )));
            }
        };
        let engine = catalog
            .engines
            .iter()
            .find(|engine| engine.os == platform.os && engine.arch == platform.arch)
            .ok_or_else(|| unsupported("Piper is not published for this system"))?;
        let windows = platform.os == "windows";
        let archive = Download::new(
            format!("{}{}", catalog.release, engine.file),
            Some(engine.bytes),
            Check::Sha256 {
                value: engine.sha256.to_owned(),
            },
        );
        let format = if windows { "zip" } else { "tar.gz" };
        let mut parts = vec![
            Part::new(archive, "piper-archive").unpacked(UnpackStep::new(
                format,
                PROGRAM_DIR,
                1,
                true,
            )),
        ];
        let mut notices = vec![format!(
            "Piper {} is its last binary release: the project is archived, and its MIT-licensed files publish no checksums, so Canoa checks them against SHA-256 digests it measured from the published files.",
            catalog.version
        )];
        if windows && platform.arch == "aarch64" {
            notices.push(
                "Piper has no build for Windows on ARM: it runs the x64 build under the system's emulation.".to_owned(),
            );
        }
        for voice in catalog
            .voices
            .iter()
            .filter(|voice| wanted.is_none_or(|language| voice.language == language))
        {
            let base = format!("{}{}{}", catalog.voices_at, voice.folder, voice.name);
            let raw = format!("{}{}{}", catalog.raw_at, voice.folder, voice.name);
            parts.push(Part::new(
                Download::new(
                    format!("{base}.onnx"),
                    Some(voice.onnx.1),
                    Check::Sha256 {
                        value: voice.onnx.0.to_owned(),
                    },
                ),
                format!("{VOICES_DIR}/{}.onnx", voice.name),
            ));
            parts.push(Part::new(
                Download::new(
                    format!("{raw}.onnx.json"),
                    Some(voice.json.1),
                    Check::Sha256 {
                        value: voice.json.0.to_owned(),
                    },
                ),
                format!("{VOICES_DIR}/{}.onnx.json", voice.name),
            ));
            notices.push(format!(
                "The voice {} is licensed as {}; a voice carries the licence of the recordings it learned from.",
                voice.name, voice.licence
            ));
        }
        let launch = if windows {
            "piper/piper.exe"
        } else {
            "piper/piper"
        };
        let mut release = Release::new(catalog.version.clone(), launch, parts);
        release.notices = notices;
        if !windows {
            release.steps.push(
                "Write the libraries' short names, which the archive carries as links, as copies"
                    .to_owned(),
            );
        }
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        if finish.release.launch.ends_with(".exe") {
            return Ok(());
        }
        link_names(&Path::new(finish.staging).join("piper"))
    }
}

/// For each `libNAME.so.A.B.C` in `dir`, the `libNAME.so.A` and `libNAME.so` the program asks for by name, as copies: the unpack
/// leaves links out, and a library the program cannot find by name is a program that will not start.
fn link_names(dir: &Path) -> Result<(), ProviderError> {
    let entries =
        std::fs::read_dir(dir).map_err(|error| internal(format!("{}: {error}", dir.display())))?;
    for entry in entries.flatten() {
        let Some(file) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some((base, rest)) = file.split_once(".so.") else {
            continue;
        };
        // `libname.so.1.2.0` has a major version and more: `libname.so.1` is already the whole name of a library without.
        let Some((major, _)) = rest.split_once('.') else {
            continue;
        };
        for name in [format!("{base}.so.{major}"), format!("{base}.so")] {
            let target = dir.join(&name);
            if !target.exists() {
                std::fs::copy(entry.path(), &target)
                    .map_err(|error| internal(format!("{}: {error}", target.display())))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_gets_the_names_the_program_asks_for() {
        let dir = std::env::temp_dir().join(format!("canoa-piper-libs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        std::fs::write(dir.join("libespeak-ng.so.1.52.0.1"), b"espeak").expect("lib");
        std::fs::write(dir.join("libonnxruntime.so.1.14.1"), b"onnx").expect("lib");
        std::fs::write(dir.join("libsingle.so.3"), b"single").expect("lib");
        std::fs::write(dir.join("piper"), b"program").expect("program");
        link_names(&dir).expect("written");
        for name in [
            "libespeak-ng.so.1",
            "libespeak-ng.so",
            "libonnxruntime.so.1",
            "libonnxruntime.so",
        ] {
            assert!(dir.join(name).is_file(), "{name}");
        }
        assert_eq!(
            std::fs::read(dir.join("libespeak-ng.so.1")).expect("read"),
            b"espeak"
        );
        assert!(
            !dir.join("libsingle.so").exists(),
            "a name with no version after its major is left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn piper_prints_its_version_alone() {
        let recipe = PiperRecipe::default();
        assert_eq!(recipe.parse_version("1.2.0\n").as_deref(), Some("1.2.0"));
        assert_eq!(recipe.parse_version("piper 1.2.0"), None);
    }
}
