//! The five built-in stores, run through the contract suite on worlds that serve each publisher's documents in the shape
//! the publisher serves them, with a stand-in program for the bytes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::{Action, InstallTools, PlanRequest, Platform, ProgramStore, StoreSettings};
use canoa_host::store::contract::{Fixture, Variant, World, WorldTransport, run, standin_bytes};
use canoa_host::store::extensions::agy::AgyStore;
use canoa_host::store::extensions::anthropic::ClaudeStore;
use canoa_host::store::extensions::codex::CodexStore;
use canoa_host::store::extensions::grok::GrokStore;
use canoa_host::store::extensions::pi::{self, PiStore};
use canoa_host::store::tools::LocalTools;
use coqui_acp_api::guatiao::Status;
use sha2::{Digest, Sha256, Sha512};

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps").parent().expect("debug");
    let name = if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    path
}

fn host() -> Platform {
    Platform::new(std::env::consts::OS, std::env::consts::ARCH)
}

fn windows() -> bool {
    cfg!(windows)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha512(bytes: &[u8]) -> String {
    format!("{:x}", Sha512::digest(bytes))
}

fn tar_gz(entries: &[(&str, Vec<u8>, u32)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    for (path, data, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder
            .append_data(&mut header, path, data.as_slice())
            .expect("entry");
    }
    builder.into_inner().expect("tar").finish().expect("gz")
}

fn zip(entries: &[(&str, Vec<u8>, u32)]) -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        for (path, data, mode) in entries {
            let options = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            writer.start_file(*path, options).expect("start");
            writer.write_all(data).expect("write");
        }
        writer.finish().expect("finish");
    }
    buffer.into_inner()
}

/// What a variant does to a published file's bytes: `(bytes served, bytes the digest is computed over)`.
fn tamper(variant: Variant, good: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    match variant {
        Variant::Mismatch => {
            let mut bad = good.clone();
            bad.extend_from_slice(b"tampered");
            (bad, good)
        }
        Variant::Moved => {
            let mut other = good;
            other.extend_from_slice(b"republished");
            (other.clone(), other)
        }
        _ => (good.clone(), good),
    }
}

fn exit_of(variant: Variant) -> u32 {
    if variant == Variant::WillNotRun { 7 } else { 0 }
}

fn finish(world: &mut World, variant: Variant) {
    if variant == Variant::DownloadFails {
        world.fail_big(4096);
    }
    if variant == Variant::Slow {
        world.slow();
    }
}

fn suite(fixture: &dyn Fixture, store: Arc<dyn ProgramStore>) {
    let failures = run(store, fixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

// --- Claude Code -----------------------------------------------------------------------------

struct ClaudeFixture(PathBuf);

impl ClaudeFixture {
    fn key() -> &'static str {
        match (windows(), std::env::consts::ARCH) {
            (true, "aarch64") => "win32-arm64",
            (true, _) => "win32-x64",
            (false, "aarch64") => "linux-arm64",
            (false, _) => "linux-x64",
        }
    }
}

impl Fixture for ClaudeFixture {
    fn older(&self) -> String {
        "1.0.0".into()
    }
    fn newer(&self) -> String {
        "2.0.0".into()
    }
    fn platform(&self) -> Platform {
        host()
    }
    fn auto_update_var(&self) -> Option<String> {
        Some("DISABLE_AUTOUPDATER".into())
    }

    fn world(&self, variant: Variant) -> World {
        let base = "https://downloads.claude.ai/claude-code-releases";
        let mut world = World::new();
        world.put(format!("{base}/latest"), b"2.0.0\n".to_vec());
        world.put(format!("{base}/stable"), b"1.0.0\n".to_vec());
        for version in ["1.0.0", "2.0.0"] {
            let line = format!("{version} (Claude Code)");
            let exit = if version == "2.0.0" {
                exit_of(variant)
            } else {
                0
            };
            let good = standin_bytes(&self.0, &line, exit, Some("DISABLE_AUTOUPDATER"));
            let (served, digested) = if version == "2.0.0" {
                tamper(variant, good)
            } else {
                (good.clone(), good)
            };
            let binary = if windows() { "claude.exe" } else { "claude" };
            let manifest = serde_json::json!({
                "version": version,
                "platforms": { Self::key(): { "binary": binary, "checksum": sha256(&digested), "size": served.len() } },
            });
            world.put(
                format!("{base}/{version}/manifest.json"),
                manifest.to_string().into_bytes(),
            );
            world.put(format!("{base}/{version}/{}/{binary}", Self::key()), served);
        }
        finish(&mut world, variant);
        world
    }
}

#[test]
fn the_claude_store_passes_every_check() {
    suite(
        &ClaudeFixture(standin()),
        Arc::new(Direct::<ClaudeStore>::new()),
    );
}

// --- Codex -------------------------------------------------------------------------------------

struct CodexFixture(PathBuf);

impl CodexFixture {
    fn triple() -> String {
        let arch = if std::env::consts::ARCH == "aarch64" {
            "aarch64"
        } else {
            "x86_64"
        };
        let os = if windows() {
            "pc-windows-msvc"
        } else {
            "unknown-linux-musl"
        };
        format!("{arch}-{os}")
    }
}

impl Fixture for CodexFixture {
    fn older(&self) -> String {
        "1.0.0".into()
    }
    fn newer(&self) -> String {
        "2.0.0".into()
    }
    fn platform(&self) -> Platform {
        host()
    }
    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let asset = format!("codex-package-{}.tar.gz", Self::triple());
        let mut channel_assets = Vec::new();
        for version in ["1.0.0", "2.0.0"] {
            let exit = if version == "2.0.0" {
                exit_of(variant)
            } else {
                0
            };
            let exe = standin_bytes(&self.0, &format!("codex-cli {version}"), exit, None);
            let name = if windows() {
                "bin/codex.exe"
            } else {
                "bin/codex"
            };
            let good = tar_gz(&[
                (name, exe, 0o755),
                ("codex-package.json", b"{}".to_vec(), 0o644),
                ("codex-resources/voice/a", b"x".to_vec(), 0o644),
            ]);
            let (served, digested) = if version == "2.0.0" {
                tamper(variant, good)
            } else {
                (good.clone(), good)
            };
            let digest = sha256(&digested);
            let base = format!("https://releases.openai.com/codex/releases/{version}");
            world.put(
                format!("{base}/codex-package_SHA256SUMS"),
                format!("{digest}  {asset}\n").into_bytes(),
            );
            world.put(format!("{base}/{asset}"), served);
            if version == "2.0.0" {
                channel_assets
                    .push(serde_json::json!({"name": asset, "digest": format!("sha256:{digest}")}));
            }
        }
        let channel = serde_json::json!({"tag_name": "rust-v2.0.0", "assets": channel_assets});
        world.put(
            "https://releases.openai.com/codex/channels/latest",
            channel.to_string().into_bytes(),
        );
        finish(&mut world, variant);
        world
    }
}

#[test]
fn the_codex_store_passes_every_check() {
    suite(
        &CodexFixture(standin()),
        Arc::new(Direct::<CodexStore>::new()),
    );
}

// --- Grok --------------------------------------------------------------------------------------

struct GrokFixture(PathBuf);

impl Fixture for GrokFixture {
    fn older(&self) -> String {
        "1.0.0".into()
    }
    fn newer(&self) -> String {
        "2.0.0".into()
    }
    fn platform(&self) -> Platform {
        host()
    }
    fn auto_update_var(&self) -> Option<String> {
        Some("GROK_DISABLE_AUTOUPDATER".into())
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        world.put("https://x.ai/cli/stable", b"2.0.0\n".to_vec());
        let os = if windows() { "windows" } else { "linux" };
        let arch = if std::env::consts::ARCH == "aarch64" {
            "aarch64"
        } else {
            "x86_64"
        };
        let suffix = if windows() { ".exe" } else { "" };
        for version in ["1.0.0", "2.0.0"] {
            let exit = if version == "2.0.0" {
                exit_of(variant)
            } else {
                0
            };
            let bytes = standin_bytes(
                &self.0,
                &format!("grok {version} (abc123)"),
                exit,
                Some("GROK_DISABLE_AUTOUPDATER"),
            );
            world.put(
                format!("https://x.ai/cli/grok-{version}-{os}-{arch}{suffix}"),
                bytes,
            );
        }
        finish(&mut world, variant);
        world
    }
}

#[test]
fn the_grok_store_passes_every_check_and_says_its_download_is_not_verified() {
    suite(
        &GrokFixture(standin()),
        Arc::new(Direct::<GrokStore>::new()),
    );
    let store = GrokStore::default_instance();
    let transport = WorldTransport::new(GrokFixture(standin()).world(Variant::Normal));
    let tools = LocalTools::new(transport, store.describe().expect("info").endpoints).into_object();
    let plan = store
        .plan(PlanRequest::new(Action::Install, host()), tools)
        .expect("plan");
    assert!(
        plan.notices.iter().any(|n| n.starts_with("Not verified")),
        "{:?}",
        plan.notices
    );
    assert!(
        plan.steps.iter().any(|s| s.contains("not verified")),
        "{:?}",
        plan.steps
    );
}

// --- Antigravity -----------------------------------------------------------------------------

struct AgyFixture(PathBuf);

impl AgyFixture {
    fn manifest_name() -> String {
        let os = if windows() { "windows" } else { "linux" };
        let arch = if std::env::consts::ARCH == "aarch64" {
            "arm64"
        } else {
            "amd64"
        };
        format!("{os}_{arch}")
    }
}

impl Fixture for AgyFixture {
    fn older(&self) -> String {
        "1.0.0".into()
    }
    fn newer(&self) -> String {
        "2.0.0".into()
    }
    fn platform(&self) -> Platform {
        host()
    }
    fn auto_update_var(&self) -> Option<String> {
        Some("AGY_CLI_DISABLE_AUTO_UPDATE".into())
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let exit = exit_of(variant);
        let exe = standin_bytes(&self.0, "2.0.0", exit, Some("AGY_CLI_DISABLE_AUTO_UPDATE"));
        let good = if windows() {
            exe
        } else {
            tar_gz(&[("antigravity", exe, 0o755)])
        };
        let (served, digested) = tamper(variant, good);
        let url = format!(
            "https://storage.googleapis.com/antigravity-public/antigravity-cli/2.0.0-1/{}",
            if windows() { "cli.exe" } else { "cli.tar.gz" }
        );
        let manifest =
            serde_json::json!({"version": "2.0.0", "url": url, "sha512": sha512(&digested)});
        world.put(format!("https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{}.json", Self::manifest_name()), manifest.to_string().into_bytes());
        world.put(url, served);
        finish(&mut world, variant);
        world
    }
}

#[test]
fn the_agy_store_passes_every_check_and_refuses_a_version_the_publisher_no_longer_serves() {
    suite(&AgyFixture(standin()), Arc::new(Direct::<AgyStore>::new()));
    let store = AgyStore::default_instance();
    let transport = WorldTransport::new(AgyFixture(standin()).world(Variant::Normal));
    let tools = LocalTools::new(transport, store.describe().expect("info").endpoints).into_object();
    let mut request = PlanRequest::new(Action::Install, host());
    request.version = Some("1.0.0".to_owned());
    let error = store.plan(request, tools).expect_err("pinned");
    assert_eq!(error.status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert!(error.message().contains("only the newest build is served"));
}

// --- Pi ----------------------------------------------------------------------------------------

struct PiFixture(PathBuf);

impl Fixture for PiFixture {
    fn older(&self) -> String {
        "1.0.0".into()
    }
    fn newer(&self) -> String {
        "2.0.0".into()
    }
    fn platform(&self) -> Platform {
        host()
    }
    fn auto_update_var(&self) -> Option<String> {
        Some("PI_SKIP_VERSION_CHECK".into())
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        world.put(
            "https://registry.npmjs.org/@earendil-works/pi-coding-agent/latest",
            br#"{"version":"2.0.0"}"#.to_vec(),
        );
        let node_v = pi::NODE_VERSION;
        let os = if windows() { "win" } else { "linux" };
        let arch = if std::env::consts::ARCH == "aarch64" {
            "arm64"
        } else {
            "x64"
        };
        let top = format!("node-v{node_v}-{os}-{arch}");
        // The stand-in is the Node: it prints Pi's version for any arguments, and exits 0 for npm's.
        let node = standin_bytes(&self.0, "2.0.0", exit_of(variant), None);
        let (archive, name) = if windows() {
            (
                zip(&[
                    (&format!("{top}/node.exe"), node, 0o755),
                    (
                        &format!("{top}/node_modules/npm/bin/npm-cli.js"),
                        b"//".to_vec(),
                        0o644,
                    ),
                ]),
                format!("{top}.zip"),
            )
        } else {
            (
                tar_gz(&[
                    (&format!("{top}/bin/node"), node, 0o755),
                    (
                        &format!("{top}/lib/node_modules/npm/bin/npm-cli.js"),
                        b"//".to_vec(),
                        0o644,
                    ),
                ]),
                format!("{top}.tar.gz"),
            )
        };
        let (served, digested) = tamper(variant, archive);
        let base = format!("https://nodejs.org/dist/v{node_v}");
        world.put(
            format!("{base}/SHASUMS256.txt"),
            format!("{}  {name}\n", sha256(&digested)).into_bytes(),
        );
        world.put(format!("{base}/{name}"), served);
        finish(&mut world, variant);
        world
    }
}

#[test]
fn the_pi_store_passes_every_check() {
    suite(&PiFixture(standin()), Arc::new(Direct::<PiStore>::new()));
}

// --- Plumbing ----------------------------------------------------------------------------------

/// A built-in store used directly, as the host holds it.
struct Direct<S>(S);

trait DefaultInstance: Sized {
    fn default_instance() -> Self;
}

macro_rules! direct {
    ($store:ty) => {
        impl DefaultInstance for $store {
            fn default_instance() -> Self {
                <$store>::from(StoreSettings::default())
            }
        }
    };
}

direct!(ClaudeStore);
direct!(CodexStore);
direct!(GrokStore);
direct!(AgyStore);
direct!(PiStore);

impl<S: DefaultInstance> Direct<S> {
    fn new() -> Self {
        Self(S::default_instance())
    }
}

impl<S: ProgramStore + DefaultInstance> ProgramStore for Direct<S> {
    fn describe(
        &self,
    ) -> Result<canoa::store::StoreInfo, coqui_acp_api::guatiao::library::ProviderError> {
        self.0.describe()
    }
    fn program(
        &self,
        q: canoa::store::ProgramQuery,
        s: coqui_acp_api::guatiao::library::Object<dyn coqui_acp_api::ProcessSpawner>,
        t: coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
    ) -> Result<canoa::store::ProgramReport, coqui_acp_api::guatiao::library::ProviderError> {
        self.0.program(q, s, t)
    }
    fn latest(
        &self,
        q: canoa::store::LatestQuery,
        t: coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
    ) -> Result<canoa::store::LatestReport, coqui_acp_api::guatiao::library::ProviderError> {
        self.0.latest(q, t)
    }
    fn plan(
        &self,
        r: PlanRequest,
        t: coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
    ) -> Result<canoa::store::InstallPlan, coqui_acp_api::guatiao::library::ProviderError> {
        self.0.plan(r, t)
    }
    fn apply(
        &self,
        r: canoa::store::ApplyRequest,
        s: coqui_acp_api::guatiao::library::Object<dyn coqui_acp_api::ProcessSpawner>,
        t: coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
        p: coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallPrompter>,
    ) -> Result<canoa::store::ApplyOutcome, coqui_acp_api::guatiao::library::ProviderError> {
        self.0.apply(r, s, t, p)
    }
}
