//! A store of a made-up program published under a made-up host: the good example the suite is run against, in two shapes, and
//! the fixture that serves its world. [`EchoRecipe`] fetches one file; [`EchoArchiveRecipe`] fetches an archive of several files
//! and runs the program once with `setup`, as an install with extra steps does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a fixture names the files it builds"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::io::Write;
use std::path::PathBuf;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, Serves, StoreInfo,
    Updatable,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use sha2::{Digest, Sha256};

use super::contract::{Fixture, Variant, World, standin_bytes};
use super::engine::{Finish, Part, Recipe, Release, UnpackStep};

/// The one origin the echo program is published at.
pub const ORIGIN: &str = "https://echo.example.test";

fn platform_name() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn version_of(line: &str) -> Option<String> {
    line.strip_prefix("echo ")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn info(archive: bool) -> StoreInfo {
    let mut info = StoreInfo::new(
        if archive { "echo-archive" } else { "echo" },
        "Echo",
        "Example Publisher",
    );
    info.serves = vec![Serves::any("echo")];
    info.platforms = vec![Platform::new(std::env::consts::OS, std::env::consts::ARCH)];
    info.endpoints = vec![ORIGIN.to_owned()];
    info
}

fn latest(tools: &Object<dyn InstallTools>) -> LatestReport {
    match tools.fetch_text(&format!("{ORIGIN}/latest"), 1024) {
        Ok(text) if !text.trim().is_empty() => LatestReport::known(text.trim()),
        Ok(_) => LatestReport::unknown("the publisher's latest file is empty"),
        Err(error) => LatestReport::unknown(error.message().to_owned()),
    }
}

fn newest(
    version: Option<&str>,
    tools: &Object<dyn InstallTools>,
) -> Result<String, ProviderError> {
    if let Some(version) = version {
        return Ok(version.to_owned());
    }
    let report = latest(tools);
    report.version.ok_or_else(|| {
        ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            report
                .unknown_because
                .as_deref()
                .unwrap_or("no latest version"),
        )
    })
}

/// A program that is one file.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoRecipe;

impl Recipe for EchoRecipe {
    fn info(&self) -> StoreInfo {
        info(false)
    }

    fn names(&self) -> &'static [&'static str] {
        &["echo-program"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        version_of(line)
    }

    fn classify(&self, _path: &str) -> (&'static str, Updatable) {
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "an unknown copy".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("ECHO_NO_UPDATE".to_owned(), "1".to_owned())])
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        latest(tools)
    }

    fn release(
        &self,
        version: Option<&str>,
        _channel: Option<&str>,
        _platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let version = newest(version, tools)?;
        let base = format!("{ORIGIN}/{version}/echo-{}", platform_name());
        let digest = tools.fetch_text(&format!("{base}.sha256"), 256)?;
        let download = Download::new(
            base,
            None,
            Check::Sha256 {
                value: digest.trim().to_owned(),
            },
        );
        let name = exe("echo");
        Ok(Release::new(
            version,
            name.clone(),
            vec![Part::new(download, name).executable()],
        ))
    }
}

/// A program that is an archive of several files and runs once after they are placed.
#[derive(Debug, Default, Clone, Copy)]
pub struct EchoArchiveRecipe;

impl Recipe for EchoArchiveRecipe {
    fn info(&self) -> StoreInfo {
        info(true)
    }

    fn names(&self) -> &'static [&'static str] {
        EchoRecipe.names()
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        version_of(line)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        EchoRecipe.classify(path)
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        EchoRecipe.launch_environment()
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        latest(tools)
    }

    fn release(
        &self,
        version: Option<&str>,
        _channel: Option<&str>,
        _platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let version = newest(version, tools)?;
        let base = format!("{ORIGIN}/{version}/echo-{}.tar.gz", platform_name());
        let digest = tools.fetch_text(&format!("{base}.sha256"), 256)?;
        let download = Download::new(
            base,
            None,
            Check::Sha256 {
                value: digest.trim().to_owned(),
            },
        );
        let part = Part::new(download, "echo.tar.gz").unpacked(UnpackStep {
            format: "tar.gz",
            into: "app".to_owned(),
            strip: 0,
            skip_links: false,
        });
        let mut release = Release::new(version, format!("app/bin/{}", exe("echo")), vec![part]);
        release
            .steps
            .push("Run the program once with setup".to_owned());
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        let program = std::path::Path::new(finish.staging).join(&finish.release.launch);
        let output = finish.run.exec(
            &program.to_string_lossy(),
            &["setup"],
            &[("ECHO_NO_UPDATE", "1")],
        )?;
        if output.code != Some(0) {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                "the setup step failed",
            ));
        }
        let marker = program.with_file_name("ran");
        if !marker.exists() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                "the setup step left no marker",
            ));
        }
        Ok(())
    }
}

/// The world of the echo program, for the suite.
#[derive(Debug, Clone)]
pub struct EchoFixture {
    /// The stand-in program's built path.
    pub standin: PathBuf,
    /// Whether the program is the archive.
    pub archive: bool,
}

impl EchoFixture {
    fn payload(&self, version: &str, exit: u32, extra: &[u8]) -> Vec<u8> {
        let line = format!("echo {version}");
        let mut bytes = standin_bytes(&self.standin, &line, exit, Some("ECHO_NO_UPDATE"));
        bytes.extend_from_slice(extra);
        if !self.archive {
            return bytes;
        }
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut add = |path: &str, data: &[u8], mode: u32| {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, path, data).expect("an entry");
        };
        add(&format!("bin/{}", exe("echo")), &bytes, 0o755);
        add("data/readme.txt", b"echo\n", 0o644);
        let mut gz = tar.into_inner().expect("the tar ends");
        gz.flush().expect("flush");
        gz.finish().expect("the gzip ends")
    }

    fn publish(
        &self,
        world: &mut World,
        version: &str,
        exit: u32,
        extra: &[u8],
        claimed: Option<&[u8]>,
    ) {
        let bytes = self.payload(version, exit, extra);
        let suffix = if self.archive { ".tar.gz" } else { "" };
        let url = format!("{ORIGIN}/{version}/echo-{}{suffix}", platform_name());
        let digest = sha256(claimed.unwrap_or(&bytes));
        world.put(format!("{url}.sha256"), digest.into_bytes());
        world.put(url, bytes);
    }
}

impl Fixture for EchoFixture {
    fn older(&self) -> String {
        "1.0.0".to_owned()
    }

    fn newer(&self) -> String {
        "2.0.0".to_owned()
    }

    fn platform(&self) -> Platform {
        Platform::new(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn auto_update_var(&self) -> Option<String> {
        Some("ECHO_NO_UPDATE".to_owned())
    }

    fn world(&self, variant: Variant) -> World {
        let mut world = World::new();
        world.put(format!("{ORIGIN}/latest"), b"2.0.0\n".to_vec());
        self.publish(&mut world, "1.0.0", 0, b"", None);
        match variant {
            Variant::Normal | Variant::Slow | Variant::DownloadFails => {
                self.publish(&mut world, "2.0.0", 0, b"", None)
            }
            Variant::Mismatch => {
                let good = self.payload("2.0.0", 0, b"");
                self.publish(&mut world, "2.0.0", 0, b"tampered", Some(&good));
            }
            Variant::WillNotRun => self.publish(&mut world, "2.0.0", 7, b"", None),
            Variant::Moved => self.publish(&mut world, "2.0.0", 0, b"republished", None),
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
