//! whisper.cpp as a Store program: the speech-to-text command line (`whisper-cli`) and one model, which the person chooses as the
//! release channel.
//!
//! ggml-org publishes the program's builds for Windows and Linux as GitHub release assets, each with a SHA-256 in the release
//! listing. The builds are attached to the `bNNNN` release made when the version's commit was built, and the `vX.Y.Z` release
//! made a few minutes later carries none of them (older ones carry their own), so a version's files are looked for on its own
//! release and else on the build release published just before it. The program reports the version of that commit, which the
//! install checks against the version it planned.
//!
//! The models are the `ggml-*.bin` files of the publisher's Hugging Face repository at one pinned revision, each checked against
//! the SHA-256 that revision's listing gives. Both the program and the model are MIT licensed.
//!
//! The program reads a WAV stream on its standard input (`-f -`) and prints the text, which is how the speech command provider
//! runs it; [`crate::WhisperModels`] says how to run the copy installed here.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and clocks are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, bad_value_error, dotted, get_json, internal_error,
    pinned, unsupported_error,
};
use canoa::store::{
    Check, Download, InstallPlan, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo,
    Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The release listing of the publisher's repository.
pub const RELEASES_API: &str = "https://api.github.com/repos/ggml-org/whisper.cpp/releases";

/// Where a release's files are served from.
pub const DOWNLOADS: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/";

/// The models' folder at the revision the digests below were read from.
pub const MODELS_ORIGIN: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/";

/// The hosts GitHub and Hugging Face send a download on to.
const ASSET_HOSTS: [&str; 7] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
    "https://cdn-lfs.hf.co",
    "https://cdn-lfs-us-1.hf.co",
    "https://cdn-lfs-eu-1.hf.co",
    "https://cas-bridge.xethub.hf.co",
    "https://us.aws.cdn.hf.co",
];

/// How many releases of the listing are read.
const LISTED: u32 = 40;

/// The longest a `vX.Y.Z` release comes after the build release that carries its files, in seconds.
const PAIRING_WINDOW: i64 = 6 * 60 * 60;

/// One model of the publisher's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// The channel that names it, and what `speech/offers` calls it.
    pub id: &'static str,
    /// What a person reads: what it is good for and how big it is.
    pub label: &'static str,
    /// The file in [`MODELS_ORIGIN`].
    pub file: &'static str,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256, as the publisher's listing gives it.
    pub sha256: &'static str,
}

/// The models offered, the default first. Multilingual ones take Spanish and English alike; the `.en` ones are English only and
/// a little better at it for their size.
pub const MODELS: [Model; 8] = [
    Model {
        id: "base",
        label: "Base, every language (148 MB)",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        id: "base.en",
        label: "Base, English only (148 MB)",
        file: "ggml-base.en.bin",
        bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    },
    Model {
        id: "tiny",
        label: "Tiny, every language (78 MB)",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        id: "tiny.en",
        label: "Tiny, English only (78 MB)",
        file: "ggml-tiny.en.bin",
        bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
    },
    Model {
        id: "small-q5_1",
        label: "Small, every language, compressed (190 MB)",
        file: "ggml-small-q5_1.bin",
        bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
    Model {
        id: "small",
        label: "Small, every language (488 MB)",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    Model {
        id: "small.en",
        label: "Small, English only (488 MB)",
        file: "ggml-small.en.bin",
        bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label: "Large v3 turbo, every language, compressed (574 MB)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
];

/// The model a channel names.
#[must_use]
pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|model| model.id == id)
}

/// Where a copy keeps its model, relative to the version's folder.
#[must_use]
pub fn model_path(model: &Model) -> String {
    format!("models/{}", model.file)
}

/// The model a copy holds, the first of [`MODELS`] whose file is in `version`, the folder of one version of the program.
#[must_use]
pub fn held_model(version: &std::path::Path) -> Option<&'static Model> {
    MODELS
        .iter()
        .find(|model| version.join(model_path(model)).is_file())
}

/// The folder of a copy's program, relative to the version's folder.
pub const PROGRAM_DIR: &str = "bin";

/// The program's file name on `os`.
#[must_use]
pub fn program_file(os: &str) -> &'static str {
    if os == "windows" {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    }
}

/// whisper.cpp's command line, with the models its channels name.
#[derive(Debug, Clone, Copy)]
pub struct WhisperRecipe {
    models: &'static [Model],
}

impl Default for WhisperRecipe {
    fn default() -> Self {
        Self { models: &MODELS }
    }
}

impl WhisperRecipe {
    /// The recipe over another list of models, which a test serves from a world of its own.
    #[must_use]
    pub const fn with_models(models: &'static [Model]) -> Self {
        Self { models }
    }
}

/// One version of the listing with the files its release or its build release carries.
struct Version {
    version: String,
    numeric: [u64; 3],
    /// The tag of the release the files are served under.
    tag: String,
    /// Each file's name and SHA-256.
    assets: Vec<(String, String)>,
}

fn numeric(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let found = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(found)
}

fn assets_of(release: &serde_json::Value) -> Vec<(String, String)> {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset["name"].as_str()?;
            let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
            let plain = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
            plain.then(|| (name.to_owned(), digest.to_ascii_lowercase()))
        })
        .collect()
}

fn published(release: &serde_json::Value) -> Option<i64> {
    let text = release["published_at"].as_str()?;
    time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(time::OffsetDateTime::unix_timestamp)
}

/// Whether `tag` names a build release: `b` and a number.
fn is_build(tag: &str) -> bool {
    tag.strip_prefix('b')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// The versions of a listing, in no order: each `vX.Y.Z` release with its own files, or else with those of the build release
/// published just before it. A draft, a build release no version claims and any other tag are left out.
fn versions(listing: &serde_json::Value) -> Vec<Version> {
    let all: Vec<&serde_json::Value> = listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true)
        .collect();
    all.iter()
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = tag.strip_prefix('v')?;
            let numbers = numeric(version)?;
            let own = assets_of(release);
            if !own.is_empty() {
                return Some(Version {
                    version: version.to_owned(),
                    numeric: numbers,
                    tag: tag.to_owned(),
                    assets: own,
                });
            }
            let at = published(release)?;
            let build = all
                .iter()
                .filter(|other| other["tag_name"].as_str().is_some_and(is_build))
                .filter(|other| !assets_of(other).is_empty())
                .filter_map(|other| Some((published(other)?, other)))
                .filter(|(then, _)| *then <= at && at - *then <= PAIRING_WINDOW)
                .max_by_key(|(then, _)| *then)?
                .1;
            Some(Version {
                version: version.to_owned(),
                numeric: numbers,
                tag: build["tag_name"].as_str()?.to_owned(),
                assets: assets_of(build),
            })
        })
        .collect()
}

fn listing(tools: &Object<dyn InstallTools>) -> Result<serde_json::Value, ProviderError> {
    get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))
}

/// The file of this system in a release, or why there is none.
fn asset_name(platform: &Platform) -> Result<&'static str, ProviderError> {
    match (platform.os.as_str(), platform.arch.as_str()) {
        ("windows", "x86_64") => Ok("whisper-bin-x64.zip"),
        ("windows", "aarch64") => Ok("whisper-bin-win-cpu-arm64.zip"),
        ("linux", "x86_64") => Ok("whisper-bin-ubuntu-x64.tar.gz"),
        ("linux", "aarch64") => Ok("whisper-bin-ubuntu-arm64.tar.gz"),
        _ => Err(unsupported_error(alloc::format!(
            "whisper.cpp publishes no program for {} on {}",
            platform.os,
            platform.arch
        ))),
    }
}

impl WhisperRecipe {
    fn channel(&self, channel: Option<&str>) -> Result<&'static Model, ProviderError> {
        let id = channel.or_else(|| self.models.first().map(|model| model.id));
        self.models
            .iter()
            .find(|model| Some(model.id) == id)
            .ok_or_else(|| {
                bad_value_error(format!(
                    "whisper.cpp has no model {:?}",
                    id.unwrap_or_default()
                ))
            })
    }
}

impl Recipe for WhisperRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("whisper", "whisper.cpp", "ggml-org");
        info.homepage = Some("https://github.com/ggml-org/whisper.cpp".to_owned());
        info.summary = Some(
            "Turns speech into text on this computer, for dictation: the audio never leaves the host. The channel is the model.".to_owned(),
        );
        info.icon = Some("chip".to_owned());
        info.platforms = ["windows", "linux"]
            .into_iter()
            .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
            .collect();
        info.channels = self
            .models
            .iter()
            .map(|model| model.id.to_owned())
            .collect();
        info.endpoints = [
            RELEASES_API.to_owned(),
            DOWNLOADS.to_owned(),
            MODELS_ORIGIN.to_owned(),
        ]
        .into_iter()
        .chain(ASSET_HOSTS.iter().map(|host| (*host).to_owned()))
        .collect();
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["whisper-cli"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        let rest = line.trim().strip_prefix("whisper.cpp version:")?.trim();
        dotted(rest).filter(|version| version == rest)
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

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match listing(tools).map(|doc| versions(&doc)).and_then(|all| {
            all.into_iter()
                .max_by_key(|each| each.numeric)
                .ok_or_else(|| internal_error("the repository lists no release with a program"))
        }) {
            Ok(found) => LatestReport::known(found.version),
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let name = asset_name(platform)?;
        let chosen = self.channel(channel)?;
        let wanted = version.map(pinned).transpose()?;
        let doc = listing(tools)?;
        let (found, digest) = versions(&doc)
            .into_iter()
            .filter(|each| {
                wanted
                    .as_deref()
                    .is_none_or(|wanted| each.version == wanted)
            })
            .filter_map(|each| {
                let digest = each
                    .assets
                    .iter()
                    .find(|(asset, _)| asset == name)
                    .map(|(_, digest)| digest.clone())?;
                Some((each, digest))
            })
            .max_by_key(|(each, _)| each.numeric)
            .ok_or_else(|| {
                unsupported_error(
                    "the repository lists no such release with a program for this system",
                )
            })?;
        let windows = platform.os == "windows";
        let program = Download::new(
            format!("{DOWNLOADS}{}/{name}", found.tag),
            None,
            Check::Sha256 { value: digest },
        );
        let weights = Download::new(
            format!("{MODELS_ORIGIN}{}", chosen.file),
            Some(chosen.bytes),
            Check::Sha256 {
                value: chosen.sha256.to_owned(),
            },
        );
        let step = UnpackStep::new(if windows { "zip" } else { "tar.gz" }, PROGRAM_DIR, 1, true);
        let launch = format!("{PROGRAM_DIR}/{}", program_file(&platform.os));
        let mut release = Release::new(
            found.version.clone(),
            launch,
            vec![
                Part::new(program, "whisper-archive").unpacked(step),
                Part::new(weights, model_path(chosen)),
            ],
        );
        release
            .facts
            .insert("model".to_owned(), chosen.id.to_owned());
        release.notices.push(format!(
            "whisper.cpp {} is checked against the SHA-256 GitHub lists for the release's file, and the model {} against the SHA-256 its publisher's listing gives; neither publisher's signature is checked by this store.",
            found.version, chosen.id
        ));
        release.notices.push(
            "Dictation then runs on this host's processor: the audio is turned into text here and goes nowhere else. A larger model is slower and uses more memory."
                .to_owned(),
        );
        Ok(release)
    }

    fn channel_of(&self, plan: &InstallPlan) -> Option<String> {
        plan.downloads.iter().find_map(|download| {
            let file = download.url.strip_prefix(MODELS_ORIGIN)?;
            self.models
                .iter()
                .find(|model| model.file == file)
                .map(|model| model.id.to_owned())
        })
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // The archive's own modes are kept when it carries them; the program is the one file that has to run.
            let path = std::path::Path::new(finish.staging)
                .join(PROGRAM_DIR)
                .join("whisper-cli");
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| internal_error(format!("whisper-cli: {error}")))?;
            }
        }
        #[cfg(not(unix))]
        let _ = finish;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> serde_json::Value {
        let sha = "ab".repeat(32);
        serde_json::json!([
            {"tag_name": "v1.9.5", "draft": false, "published_at": "2026-10-06T17:39:17Z", "assets": []},
            {"tag_name": "b5454", "draft": false, "published_at": "2026-10-06T16:51:27Z", "assets": [
                {"name": "whisper-bin-x64.zip", "digest": format!("sha256:{sha}")}]},
            {"tag_name": "v1.9.2", "draft": false, "published_at": "2026-08-04T15:31:37Z", "assets": [
                {"name": "whisper-bin-x64.zip", "digest": format!("sha256:{sha}")},
                {"name": "whisper-bin-ubuntu-x64.tar.gz", "digest": "sha1:nothing"}]},
            {"tag_name": "v1.9.3", "draft": false, "published_at": "2026-08-20T11:39:36Z", "assets": []},
            {"tag_name": "b4938", "draft": false, "published_at": "2026-08-20T11:33:19Z", "assets": [
                {"name": "whisper-bin-x64.zip", "digest": format!("sha256:{sha}")}]},
            {"tag_name": "v9.9.9", "draft": true, "published_at": "2026-12-01T00:00:00Z", "assets": []},
            {"tag_name": "v1.9.1", "draft": false, "published_at": "2026-06-19T05:53:19Z", "assets": []},
            {"tag_name": "b9999", "draft": false, "published_at": "2026-11-01T00:00:00Z", "assets": [
                {"name": "whisper-bin-x64.zip", "digest": format!("sha256:{sha}")}]}
        ])
    }

    #[test]
    fn a_version_has_its_own_files_or_those_of_the_build_made_just_before_it() {
        let found = versions(&listing());
        let by_version = |wanted: &str| found.iter().find(|each| each.version == wanted);
        assert_eq!(
            by_version("1.9.5").map(|each| each.tag.as_str()),
            Some("b5454")
        );
        assert_eq!(
            by_version("1.9.3").map(|each| each.tag.as_str()),
            Some("b4938")
        );
        assert_eq!(
            by_version("1.9.2").map(|each| each.tag.as_str()),
            Some("v1.9.2")
        );
        assert!(
            by_version("1.9.1").is_none(),
            "no build was made within hours before it"
        );
        assert!(by_version("9.9.9").is_none(), "a draft is not a release");
        assert_eq!(found.len(), 3, "a build no version claims is not one");
    }

    #[test]
    fn only_a_plain_sha_256_digest_is_a_digest() {
        let found = versions(&listing());
        let old = found.iter().find(|each| each.version == "1.9.2").unwrap();
        assert_eq!(old.assets.len(), 1, "{:?}", old.assets);
    }

    #[test]
    fn the_version_is_what_the_program_prints_and_nothing_around_it() {
        assert_eq!(
            WhisperRecipe::default()
                .parse_version("whisper.cpp version: 1.9.5\n")
                .as_deref(),
            Some("1.9.5")
        );
        assert_eq!(WhisperRecipe::default().parse_version("1.9.5"), None);
        assert_eq!(
            WhisperRecipe::default().parse_version("other.cpp version: 1.9.5"),
            None
        );
    }

    #[test]
    fn every_model_is_a_plain_file_with_a_full_digest_and_the_default_is_first() {
        assert_eq!(MODELS[0].id, "base");
        for each in MODELS {
            assert_eq!(each.sha256.len(), 64, "{}", each.id);
            assert!(
                each.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "{}",
                each.id
            );
            assert!(each.file.starts_with("ggml-") && each.file.ends_with(".bin"));
            assert!(
                MODELS.iter().filter(|other| other.id == each.id).count() == 1,
                "{} is listed once",
                each.id
            );
        }
    }
}
