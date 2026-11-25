//! The programs' stores (Node, Python, Foundry Local, Git Bash and PowerShell) against stand-in archives served from a world
//! of documents: the store contract suite, what each plan fetches and checks, and the folder a managed copy puts on a search path.
//! No runtime is downloaded.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::engine::Engine;
use canoa::store::recipe::Recipe;
use canoa::store::{
    Action, ApplyOutcome, ApplyRequest, Check, InstallPlan, InstallPrompter, InstallTools,
    LatestQuery, LatestReport, PlanRequest, Platform, ProgramQuery, ProgramReport, ProgramStore,
    STORE_GLYPHS, StoreInfo, StoreSettings,
};
use canoa_bitwarden::{
    BitwardenRecipe, DOWNLOADS as BITWARDEN_DOWNLOADS, RELEASES_API as BITWARDEN_RELEASES_API,
};
use canoa_foundry::store::{DOWNLOADS, FoundryRecipe, RELEASES_API};
use canoa_git_bash::{DOWNLOADS as GIT_DOWNLOADS, GitBashRecipe, RELEASES_API as GIT_RELEASES_API};
use canoa_host::store::contract::{Fixture, Variant, World, WorldTransport, run, standin_bytes};
use canoa_host::store::runtimes::runtime_programs;
use canoa_host::store::tools::LocalTools;
use canoa_node::{NODE_ORIGIN, NodeRecipe};
use canoa_powershell::{
    DOWNLOADS as POWERSHELL_DOWNLOADS, PowerShellRecipe, RELEASES_API as POWERSHELL_RELEASES_API,
};
use canoa_python::{PYTHON_RELEASES, PythonRecipe};
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
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
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
    fn releases(&self, variant: Variant) -> Option<String> {
        let world = self.world(variant);
        let index: serde_json::Value =
            serde_json::from_str(&world.text_at(&format!("{NODE_ORIGIN}index.json"))?).ok()?;
        let heads = canoa_node::heads(&index).ok()?;
        let mut sums = std::collections::BTreeMap::new();
        for (_, version) in &heads {
            sums.insert(
                version.clone(),
                world.text_at(&format!("{NODE_ORIGIN}v{version}/SHASUMS256.txt"))?,
            );
        }
        let origin = NODE_ORIGIN.trim_end_matches('/');
        Some(
            canoa_node::catalog_of(origin, &heads, &sums)
                .ok()?
                .to_json(),
        )
    }

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
    fn releases(&self, variant: Variant) -> Option<String> {
        let sums = self
            .world(variant)
            .text_at(&format!("{PYTHON_RELEASES}latest/download/SHA256SUMS"))?;
        let origin = PYTHON_RELEASES.trim_end_matches('/');
        Some(canoa_python::catalog_of(origin, &sums).ok()?.to_json())
    }

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

struct FoundryFixture;

impl FoundryFixture {
    fn tag(version: &str) -> String {
        format!("cli-preview-{version}")
    }

    fn name(version: &str) -> String {
        let os = if cfg!(windows) { "win" } else { "linux" };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let format = if cfg!(windows) { "msix" } else { "tar.gz" };
        format!("foundry-{version}-{os}-{arch}.{format}")
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let mut program = standin_bytes(&standin(), version, exit, None);
        program.extend_from_slice(extra);
        if cfg!(windows) {
            zipped(&[("foundry.exe", &program), ("foundry_local.dll", b"library")])
        } else {
            let root = Self::name(version).trim_end_matches(".tar.gz").to_owned();
            tar_gz(&[
                (&format!("{root}/lib/foundry"), &program),
                (&format!("{root}/lib/libfoundry_local.so"), b"library"),
            ])
        }
    }
}

impl Fixture for FoundryFixture {
    fn older(&self) -> String {
        "0.13.0".to_owned()
    }

    fn newer(&self) -> String {
        "0.14.0".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let mut releases = vec![
            // The SDK's own release shares the listing and is not the command line.
            serde_json::json!({"tag_name": "v2.4.1", "draft": false, "assets": []}),
        ];
        for (version, newer) in [("0.13.0", false), ("0.14.0", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let name = Self::name(version);
            releases.push(serde_json::json!({
                "tag_name": Self::tag(version),
                "draft": false,
                "assets": [
                    {"name": name, "digest": format!("sha256:{claimed}")},
                    {"name": "foundry-other.zip", "digest": format!("sha256:{}", "0".repeat(64))},
                ],
            }));
            world.put(format!("{DOWNLOADS}{}/{name}", Self::tag(version)), bytes);
        }
        world.put(
            format!("{RELEASES_API}?per_page=30"),
            serde_json::Value::Array(releases).to_string().into_bytes(),
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

/// A copy of the stand-in that, given `-o<folder>`, writes `extract` under it: what a self-extracting archive does.
fn self_extracting(
    standin: &std::path::Path,
    extract: &serde_json::Value,
    extra: &[u8],
) -> Vec<u8> {
    let mut bytes = std::fs::read(standin).expect("the stand-in is built");
    let trailer =
        serde_json::json!({"line": "", "exit": 0, "auto_update_var": null, "extract": extract});
    bytes.extend_from_slice(b"\nCOQUI-STANDIN:");
    bytes.extend_from_slice(trailer.to_string().as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(extra);
    bytes
}

fn github_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// Git for Windows' portable release: a self-extracting archive that holds `bin/bash.exe` and `bin/git.exe`.
struct GitBashFixture;

impl GitBashFixture {
    fn tag(version: &str) -> String {
        format!("v{version}.windows.1")
    }

    fn name(version: &str) -> String {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "64-bit"
        };
        format!("PortableGit-{version}-{arch}.7z.exe")
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let extract = serde_json::json!([
            {"path": "bin/bash.exe", "line": "GNU bash, version 5.3.15(2)-release (x86_64-pc-cygwin)", "exit": 0},
            {"path": "bin/git.exe", "line": format!("git version {version}.windows.1"), "exit": exit},
        ]);
        self_extracting(&standin(), &extract, extra)
    }
}

impl Fixture for GitBashFixture {
    fn releases(&self, variant: Variant) -> Option<String> {
        let listing: serde_json::Value = serde_json::from_str(
            &self
                .world(variant)
                .text_at(&format!("{GIT_RELEASES_API}?per_page=30"))?,
        )
        .ok()?;
        Some(canoa_git_bash::catalog_of(&listing).ok()?.to_json())
    }

    fn older(&self) -> String {
        "2.55.0".to_owned()
    }

    fn newer(&self) -> String {
        "2.56.0".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let mut releases = vec![
            serde_json::json!({"tag_name": "v2.57.0-rc1.windows.1", "draft": false, "prerelease": true, "assets": []}),
        ];
        for (version, newer) in [("2.55.0", false), ("2.56.0", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let name = Self::name(version);
            releases.push(serde_json::json!({
                "tag_name": Self::tag(version),
                "draft": false,
                "prerelease": false,
                "assets": [
                    {"name": name, "digest": format!("sha256:{claimed}")},
                    {"name": format!("MinGit-{version}-64-bit.zip"), "digest": format!("sha256:{}", "0".repeat(64))},
                ],
            }));
            world.put(
                format!("{GIT_DOWNLOADS}{}/{name}", Self::tag(version)),
                bytes,
            );
        }
        world.put(
            format!("{GIT_RELEASES_API}?per_page=30"),
            serde_json::Value::Array(releases).to_string().into_bytes(),
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

/// PowerShell 7: a zip on Windows and a `tar.gz` on Linux, each with the program at its root.
struct PowerShellFixture;

impl PowerShellFixture {
    fn tag(version: &str) -> String {
        format!("v{version}")
    }

    fn name(version: &str) -> String {
        if cfg!(windows) {
            format!("PowerShell-{version}-win-{}.zip", github_arch())
        } else {
            format!("powershell-{version}-linux-{}.tar.gz", github_arch())
        }
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let mut program = standin_bytes(&standin(), &format!("PowerShell {version}"), exit, None);
        program.extend_from_slice(extra);
        if cfg!(windows) {
            zipped(&[("pwsh.exe", &program), ("pwsh.dll", b"library")])
        } else {
            tar_gz(&[("./pwsh", &program), ("./pwsh.dll", b"library")])
        }
    }
}

impl Fixture for PowerShellFixture {
    fn releases(&self, variant: Variant) -> Option<String> {
        let listing: serde_json::Value = serde_json::from_str(
            &self
                .world(variant)
                .text_at(&format!("{POWERSHELL_RELEASES_API}?per_page=30"))?,
        )
        .ok()?;
        Some(canoa_powershell::catalog_of(&listing).ok()?.to_json())
    }

    fn older(&self) -> String {
        "7.6.1".to_owned()
    }

    fn newer(&self) -> String {
        "7.7.0".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        let mut releases = vec![
            serde_json::json!({"tag_name": "v7.8.0-rc.1", "draft": false, "prerelease": true, "assets": []}),
        ];
        for (version, newer) in [("7.6.1", false), ("7.7.0", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let name = Self::name(version);
            releases.push(serde_json::json!({
                "tag_name": Self::tag(version),
                "draft": false,
                "prerelease": false,
                "assets": [
                    {"name": name, "digest": format!("sha256:{claimed}")},
                    {"name": "hashes.sha256", "digest": format!("sha256:{}", "0".repeat(64))},
                ],
            }));
            world.put(
                format!("{POWERSHELL_DOWNLOADS}{}/{name}", Self::tag(version)),
                bytes,
            );
        }
        world.put(
            format!("{POWERSHELL_RELEASES_API}?per_page=30"),
            serde_json::Value::Array(releases).to_string().into_bytes(),
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

/// Bitwarden's command line: a zip of the one program, on the release list of the repository that publishes every Bitwarden client.
struct BitwardenFixture;

impl BitwardenFixture {
    fn tag(version: &str) -> String {
        format!("cli-v{version}")
    }

    fn name(version: &str) -> String {
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" if cfg!(target_arch = "aarch64") => "macos-arm64",
            "macos" => "macos",
            _ if cfg!(target_arch = "aarch64") => "linux-arm64",
            _ => "linux",
        };
        format!("bw-{os}-{version}.zip")
    }

    fn archive(version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let mut program = standin_bytes(&standin(), version, exit, None);
        program.extend_from_slice(extra);
        zipped(&[(if cfg!(windows) { "bw.exe" } else { "bw" }, &program)])
    }
}

impl Fixture for BitwardenFixture {
    fn releases(&self, variant: Variant) -> Option<String> {
        let listing: serde_json::Value = serde_json::from_str(
            &self
                .world(variant)
                .text_at(&format!("{BITWARDEN_RELEASES_API}?per_page=100"))?,
        )
        .ok()?;
        Some(canoa_bitwarden::catalog_of(&listing).ok()?.to_json())
    }

    fn older(&self) -> String {
        "2026.9.0".to_owned()
    }

    fn newer(&self) -> String {
        "2026.10.1".to_owned()
    }

    fn platform(&self) -> Platform {
        here()
    }

    fn auto_update_var(&self) -> Option<String> {
        None
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        // The listing holds every client's releases; only the command line's are its own.
        let mut releases = vec![
            serde_json::json!({"tag_name": "web-v2026.11.0", "draft": false, "prerelease": false, "assets": []}),
            serde_json::json!({"tag_name": "cli-v2026.11.0-rc1", "draft": false, "prerelease": true, "assets": []}),
        ];
        for (version, newer) in [("2026.9.0", false), ("2026.10.1", true)] {
            let (exit, extra, claim_good) = bytes_for(variant, version, newer);
            let bytes = Self::archive(version, exit, extra);
            let claimed = if claim_good {
                sha256(&Self::archive(version, 0, b""))
            } else {
                sha256(&bytes)
            };
            let name = Self::name(version);
            releases.push(serde_json::json!({
                "tag_name": Self::tag(version),
                "draft": false,
                "prerelease": false,
                "assets": [
                    {"name": name, "digest": format!("sha256:{claimed}")},
                    {"name": format!("bitwarden-cli-{version}-npm-build.zip"), "digest": format!("sha256:{}", "0".repeat(64))},
                ],
            }));
            world.put(
                format!("{BITWARDEN_DOWNLOADS}{}/{name}", Self::tag(version)),
                bytes,
            );
        }
        world.put(
            format!("{BITWARDEN_RELEASES_API}?per_page=100"),
            serde_json::Value::Array(releases).to_string().into_bytes(),
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
forwarding!(Foundry, FoundryRecipe);
forwarding!(GitBash, GitBashRecipe);
forwarding!(PowerShell, PowerShellRecipe);
forwarding!(Bitwarden, BitwardenRecipe);

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

#[test]
fn the_foundry_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> = Arc::new(Foundry(Engine::new(
        FoundryRecipe,
        StoreSettings::default(),
    )));
    let failures = run(store, &FoundryFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Git for Windows' portable release is a self-extracting program, which the host runs once to unpack it: Windows only.
#[cfg(windows)]
#[test]
fn the_git_bash_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> = Arc::new(GitBash(Engine::new(
        GitBashRecipe,
        StoreSettings::default(),
    )));
    let failures = run(store, &GitBashFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_powershell_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> = Arc::new(PowerShell(Engine::new(
        PowerShellRecipe,
        StoreSettings::default(),
    )));
    let failures = run(store, &PowerShellFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_bitwarden_store_passes_every_check() {
    let _alone = ONE_SUITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store: Arc<dyn ProgramStore> = Arc::new(Bitwarden(Engine::new(
        BitwardenRecipe,
        StoreSettings::default(),
    )));
    let failures = run(store, &BitwardenFixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_bitwarden_plan_fetches_the_zip_of_this_system_and_checks_the_digest_github_lists() {
    let bw = Bitwarden(Engine::new(BitwardenRecipe, StoreSettings::default()));
    let plan = plan_of(&bw, &BitwardenFixture, None);
    assert_eq!(
        plan.version.as_deref(),
        Some("2026.10.1"),
        "the newest of the command line's own, never a prerelease or another client's"
    );
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert_eq!(
        download.url,
        format!(
            "{BITWARDEN_DOWNLOADS}cli-v2026.10.1/{}",
            BitwardenFixture::name("2026.10.1")
        )
    );
    assert_eq!(
        download.check,
        Check::Sha256 {
            value: sha256(&BitwardenFixture::archive("2026.10.1", 0, b""))
        }
    );
    assert!(plan.changes.is_empty(), "it reads and writes nothing else");
    assert_eq!(
        plan_of(&bw, &BitwardenFixture, Some("2026.9.0"))
            .version
            .as_deref(),
        Some("2026.9.0")
    );
    let tools = tools_for(&bw, BitwardenFixture.world(Variant::Normal));
    let freebsd = PlanRequest::new(Action::Install, Platform::new("freebsd", "x86_64"));
    assert!(bw.plan(freebsd, tools).is_err(), "not published for it");
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
fn a_platform_the_store_was_not_measured_on_is_for_the_publishers_release_to_accept_or_refuse() {
    let node = Node(Engine::new(NodeRecipe, StoreSettings::default()));
    let mut world = World::new();
    world.put(
        format!("{NODE_ORIGIN}index.json"),
        br#"[{"version":"v2.0.0","lts":"Two"}]"#.to_vec(),
    );
    let sums = format!(
        "{}  node-v2.0.0-darwin-arm64.tar.gz
",
        "ab".repeat(32)
    );
    world.put(
        format!("{NODE_ORIGIN}v2.0.0/SHASUMS256.txt"),
        sums.into_bytes(),
    );
    let plan = node
        .plan(
            PlanRequest::new(Action::Install, Platform::new("macos", "aarch64")),
            tools_for(&node, world),
        )
        .expect("the publisher lists a build for macOS");
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert!(
        download.url.ends_with("node-v2.0.0-darwin-arm64.tar.gz"),
        "{}",
        download.url
    );
    let refused = node.plan(
        PlanRequest::new(Action::Install, Platform::new("freebsd", "x86_64")),
        tools_for(&node, NodeFixture.world(Variant::Normal)),
    );
    assert!(
        refused
            .expect_err("no build")
            .message()
            .contains("not published"),
        "the recipe says why"
    );
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
fn the_foundry_plan_fetches_the_one_file_of_this_system_and_checks_the_digest_github_lists() {
    let foundry = Foundry(Engine::new(FoundryRecipe, StoreSettings::default()));
    let plan = plan_of(&foundry, &FoundryFixture, None);
    assert_eq!(
        plan.version.as_deref(),
        Some("0.14.0"),
        "the newest command line release"
    );
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert_eq!(
        download.url,
        format!(
            "{DOWNLOADS}cli-preview-0.14.0/{}",
            FoundryFixture::name("0.14.0")
        )
    );
    assert_eq!(
        download.check,
        Check::Sha256 {
            value: sha256(&FoundryFixture::archive("0.14.0", 0, b""))
        }
    );
    assert!(plan.changes.is_empty(), "nothing outside the folder");
    let older = plan_of(&foundry, &FoundryFixture, Some("0.13.0"));
    assert_eq!(older.version.as_deref(), Some("0.13.0"));
    assert!(
        plan.notices
            .iter()
            .any(|n| n.contains("foundry server stop")),
        "the plan says a loaded model stays in memory: {:?}",
        plan.notices
    );

    let tools = tools_for(&foundry, FoundryFixture.world(Variant::Normal));
    let mut request = PlanRequest::new(Action::Install, here());
    request.version = Some("9.9.9".to_owned());
    assert!(foundry.plan(request, tools).is_err(), "no such release");
}

#[test]
fn the_git_bash_plan_fetches_the_portable_release_of_windows_and_checks_the_digest_github_lists() {
    // A plan is made for a platform it is asked for: Windows, whatever system the test runs on.
    let git = GitBash(Engine::new(GitBashRecipe, StoreSettings::default()));
    let fixture = GitBashFixture;
    let tools = tools_for(&git, fixture.world(Variant::Normal));
    let windows = Platform::new(
        "windows",
        if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x86_64"
        },
    );
    let plan = git
        .plan(PlanRequest::new(Action::Install, windows.clone()), tools)
        .expect("a plan");
    assert_eq!(
        plan.version.as_deref(),
        Some("2.56.0"),
        "the newest, never the prerelease"
    );
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert_eq!(
        download.url,
        format!(
            "{GIT_DOWNLOADS}v2.56.0.windows.1/{}",
            GitBashFixture::name("2.56.0")
        )
    );
    assert_eq!(
        download.check,
        Check::Sha256 {
            value: sha256(&GitBashFixture::archive("2.56.0", 0, b""))
        }
    );
    assert!(plan.changes.is_empty(), "nothing outside the folder");
    assert!(
        plan.notices.iter().any(|n| n.contains("self-extracting")),
        "the plan says the host runs the archive once: {:?}",
        plan.notices
    );
    assert!(
        plan.steps
            .iter()
            .any(|step| step.contains("self-extracting")),
        "{:?}",
        plan.steps
    );
    let tools = tools_for(&git, fixture.world(Variant::Normal));
    let mut older = PlanRequest::new(Action::Install, windows.clone());
    older.version = Some("2.55.0".to_owned());
    assert_eq!(
        git.plan(older, tools).expect("a plan").version.as_deref(),
        Some("2.55.0")
    );
    for refused in [
        Platform::new("linux", "x86_64"),
        Platform::new("macos", "aarch64"),
    ] {
        let tools = tools_for(&git, fixture.world(Variant::Normal));
        assert!(
            git.plan(PlanRequest::new(Action::Install, refused), tools)
                .is_err(),
            "Git Bash is Git for Windows'"
        );
    }
    let tools = tools_for(&git, fixture.world(Variant::Normal));
    let mut unlisted = PlanRequest::new(Action::Install, windows);
    unlisted.version = Some("9.9.9".to_owned());
    assert!(git.plan(unlisted, tools).is_err(), "no such release");
}

#[test]
fn the_powershell_plan_fetches_the_zip_or_the_archive_of_this_system_and_checks_the_digest_github_lists()
 {
    let pwsh = PowerShell(Engine::new(PowerShellRecipe, StoreSettings::default()));
    let plan = plan_of(&pwsh, &PowerShellFixture, None);
    assert_eq!(
        plan.version.as_deref(),
        Some("7.7.0"),
        "the newest, never the prerelease"
    );
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert_eq!(
        download.url,
        format!(
            "{POWERSHELL_DOWNLOADS}v7.7.0/{}",
            PowerShellFixture::name("7.7.0")
        )
    );
    assert_eq!(
        download.check,
        Check::Sha256 {
            value: sha256(&PowerShellFixture::archive("7.7.0", 0, b""))
        }
    );
    assert!(plan.changes.is_empty());
    assert_eq!(
        plan_of(&pwsh, &PowerShellFixture, Some("7.6.1"))
            .version
            .as_deref(),
        Some("7.6.1")
    );
    let tools = tools_for(&pwsh, PowerShellFixture.world(Variant::Normal));
    let mut mac = PlanRequest::new(Action::Install, Platform::new("macos", "aarch64"));
    mac.version = None;
    assert!(
        pwsh.plan(mac, tools).is_err(),
        "published here for Windows and Linux"
    );
}

#[test]
fn foundry_local_is_offered_for_apple_silicon_as_the_zip_and_for_no_other_mac() {
    let foundry = Foundry(Engine::new(FoundryRecipe, StoreSettings::default()));
    let sha = "ab".repeat(32);
    let listing = serde_json::json!([{
        "tag_name": "cli-preview-0.14.0", "draft": false,
        "assets": [
            {"name": "foundry-0.14.0-osx-arm64.zip", "digest": format!("sha256:{sha}")},
            {"name": "foundry-0.14.0-osx-arm64.pkg", "digest": format!("sha256:{}", "cd".repeat(32))},
        ]
    }]);
    let mut world = World::new();
    world.put(
        format!("{RELEASES_API}?per_page=30"),
        listing.to_string().into_bytes(),
    );
    let tools = tools_for(&foundry, world);
    let plan = foundry
        .plan(
            PlanRequest::new(Action::Install, Platform::new("macos", "aarch64")),
            tools,
        )
        .expect("a plan for Apple silicon");
    let [download] = plan.downloads.as_slice() else {
        panic!("one download: {plan:#?}");
    };
    assert_eq!(
        download.url,
        format!("{DOWNLOADS}cli-preview-0.14.0/foundry-0.14.0-osx-arm64.zip")
    );
    assert_eq!(download.check, Check::Sha256 { value: sha });
    let tools = tools_for(&foundry, World::new());
    assert!(
        foundry
            .plan(
                PlanRequest::new(Action::Install, Platform::new("macos", "x86_64")),
                tools
            )
            .is_err(),
        "no Intel Mac build is published"
    );
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
    for info in [
        NodeRecipe.info(),
        PythonRecipe.info(),
        FoundryRecipe.info(),
        GitBashRecipe.info(),
        PowerShellRecipe.info(),
        BitwardenRecipe.info(),
    ] {
        assert!(!info.endpoints.is_empty());
        assert!(
            info.endpoints.iter().all(|e| e.starts_with("https://")),
            "{:?}",
            info.endpoints
        );
    }
    assert_eq!(NodeRecipe.info().endpoints, [NODE_ORIGIN]);
}

#[test]
fn each_runtime_says_what_it_is_names_a_glyph_and_the_commands_it_puts_on_a_path() {
    for info in [
        NodeRecipe.info(),
        PythonRecipe.info(),
        FoundryRecipe.info(),
        GitBashRecipe.info(),
        PowerShellRecipe.info(),
        BitwardenRecipe.info(),
    ] {
        let program = &info.program;
        let icon = info.icon.as_deref().unwrap_or_default();
        assert!(
            STORE_GLYPHS.contains(&icon),
            "{program} names {icon:?}, which the application does not draw"
        );
        assert!(
            info.summary
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty()),
            "{program} has no summary"
        );
        // Foundry Local's folder holds libraries a program should not find, so it is on no `PATH`: it provides no command.
        let exposed = runtime_programs();
        assert_eq!(
            !info.commands.is_empty(),
            exposed.iter().any(|each| each == program),
            "{program} provides {:?}",
            info.commands
        );
    }
    assert_eq!(
        NodeRecipe.info().commands,
        ["node", "npm", "npx", "corepack"]
    );
    assert_eq!(GitBashRecipe.info().commands, ["bash", "sh", "git"]);
    assert_eq!(PowerShellRecipe.info().commands, ["pwsh"]);
    assert!(BitwardenRecipe.info().commands.is_empty());
    assert_eq!(GitBashRecipe.info().icon.as_deref(), Some("branch"));
    assert_eq!(PowerShellRecipe.info().icon.as_deref(), Some("prompt"));
}
