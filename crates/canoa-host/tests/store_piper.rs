//! The Piper store against the pinned release it ships (what each platform's plan fetches and checks) and against a stand-in
//! release it installs: the program lands in the version's folder and is read back. Its voices are the catalogs' (`canoa-piper`'s
//! own tests), installed on their own from the Models page. Nothing is downloaded.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use canoa::store::engine::Engine;
use canoa::store::recipe::Recipe;
use canoa::store::source::SourceFetch;
use canoa::store::{
    Action, ApplyRequest, Check, InstallPrompter, InstallTools, PlanRequest, Platform, Progress,
    STORE_GLYPHS, StoreSettings,
};
use canoa_host::store::contract::{World, WorldTransport, standin_bytes};
use canoa_host::store::tools::LocalTools;
use canoa_piper::store::{Catalog, Engine as Archive, PiperRecipe, VERSION};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{LoginAnswer, LoginForm, ProcessSpawner};
use coqui_acp_harness::LocalSpawner;
use sha2::{Digest, Sha256};

/// A fetch for a provider that reads no document: any address is refused.
struct NoFetch;

impl SourceFetch for NoFetch {
    fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            format!("no document is read: {url}").as_str(),
        ))
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_text(url, max)
    }
}

fn no_fetch() -> Object<dyn SourceFetch> {
    NoFetch.into_object()
}

fn sha256(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn tools(recipe: &PiperRecipe, world: World) -> Object<dyn InstallTools> {
    LocalTools::new(WorldTransport::new(world), recipe.info().endpoints).into_object()
}

#[test]
fn each_platforms_plan_fetches_the_pinned_archive_alone_and_checks_its_digest() {
    let recipe = PiperRecipe::default();
    let engine = Engine::new(PiperRecipe::default(), StoreSettings::default());
    let info = recipe.info();
    assert!(STORE_GLYPHS.contains(&info.icon.as_deref().unwrap_or_default()));
    assert!(info.commands.is_empty(), "Piper is on no search path");
    let expected = [
        ("windows", "x86_64", "piper_windows_amd64.zip"),
        ("windows", "aarch64", "piper_windows_amd64.zip"),
        ("linux", "x86_64", "piper_linux_x86_64.tar.gz"),
        ("linux", "aarch64", "piper_linux_aarch64.tar.gz"),
    ];
    for (os, arch, file) in expected {
        let plan = engine
            .plan(
                &PlanRequest::new(Action::Install, Platform::new(os, arch)),
                &tools(&recipe, World::new()),
            )
            .expect("a plan");
        assert_eq!(plan.version.as_deref(), Some(VERSION), "{os} {arch}");
        assert_eq!(
            plan.downloads.len(),
            1,
            "{os} {arch}: the program alone, the voices are installed on their own"
        );
        let download = &plan.downloads[0];
        assert!(download.url.ends_with(file), "{}", download.url);
        assert!(download.url.contains("/2023.11.14-2/"));
        let Check::Sha256 { value } = &download.check else {
            panic!("a download with no digest: {download:?}");
        };
        assert_eq!(value.len(), 64, "{download:?}");
        assert!(download.bytes.is_some());
        // Every file comes from an origin the store names.
        assert!(
            info.endpoints
                .iter()
                .any(|endpoint| download.url.starts_with(endpoint.as_str())),
            "{} is outside the endpoints",
            download.url
        );
        assert!(plan.changes.is_empty(), "nothing outside the folder");
        let notices = plan.notices.join("\n");
        assert!(notices.contains("last binary release"), "{notices}");
        assert!(
            notices.contains("No voice comes with the program"),
            "{notices}"
        );
        assert_eq!(
            notices.contains("emulation"),
            os == "windows" && arch == "aarch64"
        );
    }
}

#[test]
fn a_channel_a_version_or_a_platform_piper_does_not_publish_is_refused() {
    let recipe = PiperRecipe::default();
    let engine = Engine::new(PiperRecipe::default(), StoreSettings::default());
    let here = Platform::new("linux", "x86_64");
    let plan = |channel: Option<&str>, version: Option<&str>, platform: Platform| {
        let mut request = PlanRequest::new(Action::Install, platform);
        request.channel = channel.map(str::to_owned);
        request.version = version.map(str::to_owned);
        engine.plan(&request, &tools(&recipe, World::new()))
    };
    let alone = plan(None, None, here.clone()).expect("a plan");
    assert_eq!(
        recipe.channel_of(&alone),
        None,
        "the program has no channel"
    );
    let refused = plan(Some("standard"), None, here.clone()).expect_err("no channels");
    assert!(
        refused.message().contains("no channel") && refused.message().contains("Models page"),
        "{refused:?}"
    );
    assert!(plan(Some("klingon"), None, here.clone()).is_err());
    assert!(
        plan(None, Some("9.9.9"), here.clone()).is_err(),
        "no such release"
    );
    assert!(plan(None, Some(VERSION), here).is_ok());
    assert!(
        plan(None, None, Platform::new("macos", "aarch64")).is_err(),
        "no measured build"
    );
    assert!(plan(None, None, Platform::new("freebsd", "x86_64")).is_err());
}

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps dir").parent().expect("debug dir");
    let name = if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples ({})",
        path.display()
    );
    path
}

fn archive(program: &[u8]) -> (String, Vec<u8>) {
    if cfg!(windows) {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("piper/piper.exe", zip::write::SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(program).expect("bytes");
        zip.start_file(
            "piper/espeak-ng-data/en_dict",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(b"words").expect("bytes");
        (
            "piper_windows_amd64.zip".to_owned(),
            zip.finish().expect("zip").into_inner(),
        )
    } else {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (path, data) in [
            ("piper/piper", program),
            ("piper/libespeak-ng.so.1.52.0.1", &b"espeak"[..]),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, data).expect("entry");
        }
        let mut gz = tar.into_inner().expect("tar");
        gz.flush().expect("flush");
        (
            "piper_linux_x86_64.tar.gz".to_owned(),
            gz.finish().expect("gzip"),
        )
    }
}

struct Prompter(Mutex<Vec<String>>);

impl InstallPrompter for Prompter {
    fn say(&self, line: &str) -> Result<(), ProviderError> {
        self.0.lock().expect("lines").push(line.to_owned());
        Ok(())
    }
    fn progress(&self, _progress: Progress) -> Result<(), ProviderError> {
        Ok(())
    }
    fn ask(&self, _form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        Err(ProviderError::new(guatiao_unsupported(), "no one to ask"))
    }
    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(false)
    }
}

fn guatiao_unsupported() -> coqui_acp_api::guatiao::Status {
    coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED
}

#[test]
fn a_stand_in_release_installs_the_program_and_the_program_is_read_back() {
    let program = standin_bytes(&standin(), VERSION, 0, None);
    let (file, bytes) = archive(&program);
    // The platform the engine reads from the system it runs on.
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let catalog = Catalog {
        version: VERSION.to_owned(),
        release: "https://piper.example/release/".to_owned(),
        endpoints: vec!["https://piper.example/".to_owned()],
        engines: vec![Archive {
            os,
            arch,
            file: leak(file.clone()),
            sha256: leak(sha256(&bytes)),
            bytes: bytes.len() as u64,
        }],
    };
    let recipe = PiperRecipe::over(catalog);
    let mut world = World::new();
    world.put(format!("https://piper.example/release/{file}"), bytes);
    let engine = Engine::new(recipe.clone(), StoreSettings::default());
    let staging = std::env::temp_dir().join(format!("canoa-piper-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).expect("a staging folder");
    let tools = LocalTools::new(WorldTransport::new(world), recipe.info().endpoints)
        .granted(&staging)
        .into_object();
    let ask = PlanRequest::new(Action::Install, Platform::new(os, arch));
    let plan = engine.plan(&ask, &tools).expect("a plan");
    assert_eq!(plan.downloads.len(), 1);
    let request = ApplyRequest::new(plan, staging.to_string_lossy());
    let spawner = LocalSpawner::new().files(&staging).into_object();
    let prompter = Prompter(Mutex::new(Vec::new())).into_object();
    let outcome = engine
        .apply(&request, &spawner, &tools, &prompter)
        .unwrap_or_else(|error| panic!("the install failed: {}", error.message));
    assert_eq!(
        outcome.version.as_deref(),
        Some(VERSION),
        "the program's own version was read back"
    );
    let launch = outcome.launch.expect("a launch").path;
    let launch = PathBuf::from(launch);
    assert!(
        launch.is_file() && launch.starts_with(&staging),
        "{}",
        launch.display()
    );
    let copy = launch
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the version's folder");
    if cfg!(unix) {
        assert!(
            copy.join("piper/libespeak-ng.so.1").is_file(),
            "the short names are written"
        );
    }
    // A file that is not the one measured is not installed.
    let mut tampered = World::new();
    let (file, bytes) = archive(&standin_bytes(&standin(), VERSION, 0, None));
    tampered.put(
        format!("https://piper.example/release/{file}"),
        [bytes.as_slice(), b"x"].concat(),
    );
    let second = std::env::temp_dir().join(format!("canoa-piper-store-b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&second);
    std::fs::create_dir_all(&second).expect("a staging folder");
    let tools = LocalTools::new(WorldTransport::new(tampered), recipe.info().endpoints)
        .granted(&second)
        .into_object();
    let ask = PlanRequest::new(Action::Install, Platform::new(os, arch));
    let plan = engine.plan(&ask, &tools).expect("a plan");
    let request = ApplyRequest::new(plan, second.to_string_lossy());
    let spawner: Object<dyn ProcessSpawner> = LocalSpawner::new().files(&second).into_object();
    let prompter = Prompter(Mutex::new(Vec::new())).into_object();
    assert!(engine.apply(&request, &spawner, &tools, &prompter).is_err());
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&second);
}

/// Downloads the real release and the two voices of the shipped catalog (about 150 MB) and has the installed program speak. It reaches the network, so it
/// runs on request: `cargo test -p canoa-host --test store_piper -- --ignored`.
#[test]
#[ignore = "downloads the real release from GitHub and Hugging Face"]
fn the_real_release_installs_and_speaks() {
    use canoa_host::store::tools::HttpTransport;
    use std::sync::Arc;

    let recipe = PiperRecipe::default();
    let engine = Engine::new(PiperRecipe::default(), StoreSettings::default());
    let staging = std::env::temp_dir().join(format!("canoa-piper-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).expect("a staging folder");
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), recipe.info().endpoints)
        .granted(&staging)
        .into_object();
    let platform = Platform::new(std::env::consts::OS, std::env::consts::ARCH);
    let ask = PlanRequest::new(Action::Install, platform);
    let plan = engine.plan(&ask, &tools).expect("a plan");
    let request = ApplyRequest::new(plan, staging.to_string_lossy());
    let spawner = LocalSpawner::new().files(&staging).into_object();
    let prompter = Prompter(Mutex::new(Vec::new())).into_object();
    let outcome = engine
        .apply(&request, &spawner, &tools, &prompter)
        .unwrap_or_else(|error| panic!("the install failed: {}", error.message));
    assert_eq!(outcome.version.as_deref(), Some(VERSION));
    let launch = PathBuf::from(outcome.launch.expect("a launch").path);
    let copy = launch
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the version's folder")
        .to_path_buf();
    println!("installed at {}", copy.display());
    // The voices are the shipped catalog's, fetched as the Models page does, each checked by the digest the catalog gives.
    let include = [
        "en_US-ljspeech-medium".to_owned(),
        "es_MX-claude-high".to_owned(),
    ];
    let catalog = canoa_piper::catalog_of(
        canoa_piper::catalog::DEFAULT_REPO,
        "4a409e3cbfd0a189930b38f127a38e107677e8ac",
        include_str!("../../../extensions/canoa-piper/tests/fixtures/voices_all_4a409e3c.json"),
        &include,
        &[],
        |folder| {
            let tree =
                include_str!("../../../extensions/canoa-piper/tests/fixtures/trees_4a409e3c.json");
            let trees: serde_json::Value = serde_json::from_str(tree).expect("JSON");
            Ok(trees[folder].to_string())
        },
    )
    .expect("the catalog");
    let inside = copy
        .strip_prefix(&staging)
        .expect("inside the staging folder");
    for file in catalog.items.iter().flat_map(|item| &item.files) {
        let check = match &file.check {
            Some(canoa::store::catalog::FileCheck::Sha256(value)) => Check::Sha256 {
                value: value.clone(),
            },
            Some(canoa::store::catalog::FileCheck::GitBlob(value)) => Check::GitBlob {
                value: value.clone(),
            },
            other => panic!("a voice file with no digest this reads: {other:?}"),
        };
        let at = inside.join("voices").join(&file.file);
        tools
            .fetch(canoa::store::FetchRequest::new(
                file.url.clone(),
                at.to_string_lossy().replace('\\', "/"),
                check,
                1 << 30,
            ))
            .unwrap_or_else(|error| panic!("{} was not fetched: {}", file.file, error.message));
    }
    assert!(copy.join("voices/en_US-ljspeech-medium.onnx").is_file());
    assert!(copy.join("voices/es_MX-claude-high.onnx.json").is_file());

    // The provider speaks with what was installed: both voices, the audio a sentence at a time.
    use canoa::speech::output::{READ_TIMED_OUT, SayRequest, SpeechOutput, VoiceQuery};
    use canoa_piper::{PiperSettings, PiperSpeech};
    let provider = PiperSpeech::from(PiperSettings::new(
        launch.to_string_lossy(),
        copy.join("voices").to_string_lossy(),
    ));
    let voices = provider
        .voices(
            VoiceQuery::default(),
            LocalSpawner::new().into_object(),
            no_fetch(),
        )
        .expect("listed")
        .voices;
    assert_eq!(
        voices.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        ["en_US-ljspeech-medium", "es_MX-claude-high"]
    );
    for (voice, text) in [
        (
            "en_US-ljspeech-medium",
            "Hello, this is a test. And a second sentence.",
        ),
        (
            "es_MX-claude-high",
            "Hola, esto es una prueba. Y una segunda frase.",
        ),
    ] {
        let mut request = SayRequest::new(text);
        request.voice = Some(voice.to_owned());
        let utterance = provider
            .say(request, LocalSpawner::new().into_object())
            .expect("it speaks");
        let rate = utterance.format().expect("format").rate;
        let (mut total, mut chunk) = (0_usize, vec![0_u8; 8192]);
        loop {
            match utterance.read(&mut chunk, 10_000).expect("audio") {
                0 => break,
                READ_TIMED_OUT => panic!("{voice} fell silent"),
                count => total += usize::try_from(count).expect("count"),
            }
        }
        // 22050 Hz 16-bit mono: a second is 44100 bytes, and two sentences are over two seconds.
        assert_eq!(rate, 22_050);
        assert!(total > 80_000, "{voice} spoke {total} bytes");
    }
}
