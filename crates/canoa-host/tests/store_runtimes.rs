//! The Runtimes type's stores (Node and Python) against stand-in archives served from a world of documents: the store contract
//! suite, what each plan fetches and checks, and the folder a managed copy puts on a search path. No runtime is downloaded.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::recipe::Recipe;
use canoa::store::{
    Action, ApplyOutcome, ApplyRequest, Check, InstallPlan, InstallPrompter, InstallTools,
    LatestQuery, LatestReport, PlanRequest, Platform, ProgramQuery, ProgramReport, ProgramStore,
    StoreInfo, StoreSettings,
};
use canoa_host::store::contract::{Fixture, Variant, World, WorldTransport, run, standin_bytes};
use canoa_host::store::engine::Engine;
use canoa_host::store::runtimes::{NODE_ORIGIN, NodeRecipe, PYTHON_RELEASES, PythonRecipe};
use canoa_host::store::tools::LocalTools;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use sha2::{Digest, Sha256};

/// The suite watches the real registry key for the user's environment, so two suites never run at once.
static ONE_SUITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn here() -> Platform {
    Platform::new(std::env::consts::OS, std::env::consts::ARCH)
}

fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, path, *data).expect("an entry");
    }
    let mut gz = tar.into_inner().expect("the tar ends");
    gz.flush().expect("flush");
    gz.finish().expect("the gzip ends")
}

fn zipped(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, data) in entries {
        zip.start_file(*path, zip::write::SimpleFileOptions::default())
            .expect("an entry");
        zip.write_all(data).expect("its bytes");
    }
    zip.finish().expect("the zip ends").into_inner()
}

/// How a published version differs in a variant.
fn bytes_for(variant: Variant, version: &str, newer: bool) -> (u32, &'static [u8], bool) {
    match (variant, newer) {
        (Variant::WillNotRun, true) => (7, b"", false),
        (Variant::Moved, true) => (0, b"republished", false),
        (Variant::Mismatch, true) => (0, b"tampered", true),
        _ => {
            let _ = version;
            (0, b"", false)
        }
    }
}

struct NodeFixture;

impl NodeFixture {
    fn archive_name(version: &str) -> String {
        let os = if cfg!(windows) { "win" } else { "linux" };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let format = if cfg!(windows) { "zip" } else { "tar.gz" };
        format!("node-v{version}-{os}-{arch}.{format}")
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let mut program = standin_bytes(&standin(), &format!("v{version}"), exit, None);
        program.extend_from_slice(extra);
        let root = Self::archive_name(version);
        let root = root.trim_end_matches(".zip").trim_end_matches(".tar.gz");
        if cfg!(windows) {
            zipped(&[(&format!("{root}/node.exe"), &program)])
        } else {
            tar_gz(&[(&format!("{root}/bin/node"), &program)])
        }
    }
}

impl Fixture for NodeFixture {
    fn older(&self) -> String {
        "1.0.0".to_owned()
    }

    fn newer(&self) -> String {
        "2.0.0".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        world.put(
            format!("{NODE_ORIGIN}index.json"),
            br#"[{"version":"v2.0.0","lts":"Two"},{"version":"v1.0.0","lts":"One"}]"#.to_vec(),
        );
        for (version, newer) in [("1.0.0", false), ("2.0.0", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let name = Self::archive_name(version);
            let base = format!("{NODE_ORIGIN}v{version}");
            world.put(
                format!("{base}/SHASUMS256.txt"),
                format!("{claimed}  {name}\n{}  other-file.txt\n", "0".repeat(64)).into_bytes(),
            );
            world.put(format!("{base}/{name}"), bytes);
        }
        if variant == Variant::DownloadFails {
            world.fail_big(1024);
        }
        if variant == Variant::Slow {
            world.slow();
        }
        world
    }
}

struct PythonFixture;

const TAG: &str = "20250101";

impl PythonFixture {
    fn triple() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            ("windows", _) => "aarch64-pc-windows-msvc",
            (_, "x86_64") => "x86_64-unknown-linux-gnu",
            _ => "aarch64-unknown-linux-gnu",
        }
    }

    fn file(version: &str) -> String {
        format!(
            "cpython-{version}+{TAG}-{}-install_only.tar.gz",
            Self::triple()
        )
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let mut program = standin_bytes(&standin(), &format!("Python {version}"), exit, None);
        program.extend_from_slice(extra);
        let path = if cfg!(windows) {
            "python/python.exe".to_owned()
        } else {
            "python/bin/python3.13".to_owned()
        };
        tar_gz(&[(&path, &program)])
    }
}

impl Fixture for PythonFixture {
    fn older(&self) -> String {
        "3.13.1".to_owned()
    }

    fn newer(&self) -> String {
        "3.13.2".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let mut sums = String::new();
        for (version, newer) in [("3.13.1", false), ("3.13.2", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let file = Self::file(version);
            sums.push_str(&format!("{claimed}  {file}\n"));
            world.put(
                format!(
                    "{PYTHON_RELEASES}download/{TAG}/{}",
                    file.replace('+', "%2B")
                ),
                bytes,
            );
        }
        world.put(
            format!("{PYTHON_RELEASES}latest/download/SHA256SUMS"),
            sums.into_bytes(),
        );
        if variant == Variant::DownloadFails {
            world.fail_big(1024);
        }
        if variant == Variant::Slow {
            world.slow();
        }
        world
    }
}

macro_rules! forwarding {
    ($name:ident, $recipe:ty) => {
        struct $name(Engine<$recipe>);

        impl ProgramStore for $name {
            fn describe(&self) -> Result<StoreInfo, ProviderError> {
                self.0.describe()
            }
            fn program(
                &self,
                q: ProgramQuery,
                s: Object<dyn ProcessSpawner>,
                t: Object<dyn InstallTools>,
            ) -> Result<ProgramReport, ProviderError> {
                self.0.program(&q, &s, &t)
            }
            fn latest(
                &self,
                q: LatestQuery,
                t: Object<dyn InstallTools>,
            ) -> Result<LatestReport, ProviderError> {
                self.0.latest(&q, &t)
            }
            fn plan(
                &self,
                r: PlanRequest,
                t: Object<dyn InstallTools>,
            ) -> Result<InstallPlan, ProviderError> {
                self.0.plan(&r, &t)
            }
            fn apply(
                &self,
                r: ApplyRequest,
                s: Object<dyn ProcessSpawner>,
                t: Object<dyn InstallTools>,
                p: Object<dyn InstallPrompter>,
            ) -> Result<ApplyOutcome, ProviderError> {
                self.0.apply(&r, &s, &t, &p)
            }
        }
    };
}

forwarding!(Node, NodeRecipe);
forwarding!(Python, PythonRecipe);

#[test]
fn the_node_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> =
        Arc::new(Node(Engine::new(NodeRecipe, StoreSettings::default())));
    let failures = run(store, &NodeFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_python_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> =
        Arc::new(Python(Engine::new(PythonRecipe, StoreSettings::default())));
    let failures = run(store, &PythonFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

fn tools_for(store: &dyn ProgramStore, world: World) -> Object<dyn InstallTools> {
    let endpoints = store.describe().expect("described").endpoints;
    LocalTools::new(WorldTransport::new(world), endpoints).into_object()
}

fn plan_of(store: &dyn ProgramStore, fixture: &dyn Fixture, version: Option<&str>) -> InstallPlan {
    let tools = tools_for(store, fixture.world(Variant::Normal));
    let mut request = PlanRequest::new(Action::Install, fixture.platform());
    request.version = version.map(str::to_owned);
    store.plan(request, tools).expect("a plan")
}

#[test]
fn a_plan_fetches_one_file_from_the_publisher_and_checks_its_published_digest() {
    let node = Node(Engine::new(NodeRecipe, StoreSettings::default()));
    let plan = plan_of(&node, &NodeFixture, None);
    assert_eq!(
        plan.version.as_deref(),
        Some("2.0.0"),
        "the newest long-term release"
    );
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert!(download.url.starts_with(NODE_ORIGIN), "{}", download.url);
    let expected = sha256(&NodeFixture::archive("2.0.0", 0, b""));
    assert_eq!(
        download.check,
        Check::Sha256 { value: expected },
        "the digest is the one nodejs.org lists for this very file"
    );
    assert!(plan.changes.is_empty(), "nothing outside the folder");

    let python = Python(Engine::new(PythonRecipe, StoreSettings::default()));
    let plan = plan_of(&python, &PythonFixture, Some("3.13.1"));
    assert_eq!(plan.version.as_deref(), Some("3.13.1"));
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert!(
        download.url.starts_with(PYTHON_RELEASES),
        "{}",
        download.url
    );
    assert!(
        download.url.contains("%2B"),
        "the plus is spelled as GitHub does"
    );
    assert_eq!(
        download.check,
        Check::Sha256 {
            value: sha256(&PythonFixture::archive("3.13.1", 0, b""))
        }
    );
    assert!(plan.changes.is_empty());
}

#[test]
fn a_version_the_publisher_does_not_list_is_refused_and_a_digest_it_lacks_too() {
    let python = Python(Engine::new(PythonRecipe, StoreSettings::default()));
    let tools = tools_for(&python, PythonFixture.world(Variant::Normal));
    let mut request = PlanRequest::new(Action::Install, here());
    request.version = Some("3.13.99".to_owned());
    assert!(python.plan(request, tools).is_err());

    let node = Node(Engine::new(NodeRecipe, StoreSettings::default()));
    let tools = tools_for(&node, NodeFixture.world(Variant::Normal));
    let mut request = PlanRequest::new(Action::Install, here());
    request.version = Some("9.9.9".to_owned());
    assert!(node.plan(request, tools).is_err(), "no digest list for it");
}

#[test]
fn the_runtimes_stores_name_the_publishers_endpoints_and_nothing_wider() {
    for info in [NodeRecipe.info(), PythonRecipe.info()] {
        assert!(!info.endpoints.is_empty());
        assert!(
            info.endpoints.iter().all(|e| e.starts_with("https://")),
            "{:?}",
            info.endpoints
        );
    }
    assert_eq!(NodeRecipe.info().endpoints, [NODE_ORIGIN]);
}
