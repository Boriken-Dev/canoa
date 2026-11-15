//! `canoa` with a made-up program Store in place of the built-in ones: one program, `claude`, published by a fake publisher and
//! installed on the user's programs shelf, where an entry that runs Claude Code looks for the Store's copy. The program installed
//! is the file `claude_standin` beside this one (a script that answers `--version` and then becomes `harness_standin`), so a real
//! browser can install a program through a portal's host, and start a session on it, without downloading anything. Its command
//! line is `canoa`'s; the portal's template unit runs it as `canoa serve --portal ...`.
//! Built by `cargo build -p canoa-host --bin canoa --examples`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use canoa_host::cli::{self, Cli, Front, Program};
use canoa_host::store::builtin::{EchoClaudeStore, EchoStore};
use canoa_host::store::contract::{Fixture as _, Variant, WorldTransport};
use canoa_host::store::echo::EchoFixture;
use canoa_host::store::programs::{Options, Programs};
use canoa_host::store::shelf::Shelf;
use clap::Parser as _;
use coqui_acp_api::guatiao::library::Registry;

coqui_acp_api::guatiao::local_providers!(
    id = "echo-stores",
    version = "0.0.0",
    providers = [EchoStore, EchoClaudeStore]
);

fn sibling(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("this program knows where it is");
    exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn echo_programs() -> Option<Programs> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let mut registry = Registry::new("test", "0.0.0");
    registry.register_local("test", library).ok()?;
    let fixture = EchoFixture {
        standin: sibling("claude_standin"),
        archive: false,
    };
    let mut options = Options::default();
    options.home = Some(home.clone());
    options.watch_registry = false;
    options.settle = Duration::from_millis(100);
    options.allow_elevated = true;
    options.environment = {
        let mut env = coqui_acp_api::Environment::new();
        for name in ["PATH", "Path", "SystemRoot"] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name, value);
            }
        }
        env
    };
    Some(
        Programs::new(Shelf::unchecked(Shelf::default_root()?), Arc::new(registry))
            .transport(WorldTransport::new(fixture.world(Variant::Normal)))
            .options(options),
    )
}

/// This process's own standard streams, as `canoa` has them, with the made-up Store.
struct Standin;

impl Front for Standin {
    fn print(&self, line: &str) -> std::io::Result<()> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|()| out.flush())
    }

    fn say(&self, line: &str) {
        eprintln!("{line}");
    }

    fn when_input_closes(&self, status: u8, then: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("canoa-stdin".to_owned())
            .spawn(move || {
                let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
                then();
                std::process::exit(i32::from(status));
            })
            .map(drop)
    }

    fn program(&self) -> std::io::Result<Program> {
        Ok(Program::new(std::env::current_exe()?, Vec::new()))
    }

    fn home(&self) -> Option<PathBuf> {
        None
    }

    fn holder(&self) -> &str {
        "portal_standin_host"
    }

    fn ask(&self, _prompt: &str, _secret: bool) -> std::io::Result<Option<String>> {
        Ok(None)
    }

    fn programs(&self) -> Option<(Arc<Programs>, Vec<String>)> {
        echo_programs().map(|programs| (Arc::new(programs), vec!["echo_claude_store".to_owned()]))
    }
}

fn main() -> ExitCode {
    // The made-up copy is installed on the programs shelf, where an entry looks for the Store's copy: a folder of this run's own, so
    // that nothing of a developer's real shelf is replaced and no real entry of theirs starts the stand-in.
    if std::env::var_os("XDG_DATA_HOME").is_none_or(|value| value.is_empty()) {
        eprintln!(
            "portal_standin_host: set XDG_DATA_HOME to a folder made for this run (the unit does: Environment=XDG_DATA_HOME=/home/%i/standin-data)"
        );
        return ExitCode::from(2);
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    match cli::run(Cli::parse(), &Standin) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failed) => {
            eprintln!("portal_standin_host: {}", failed.why);
            ExitCode::from(failed.status)
        }
    }
}
