//! `canoa`: serves the agents and providers canoa.cfg names to paired keys, on loopback.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a binary names what it uses from std"
)]

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use canoa_host::cli::{self, Cli, Front, Program};
use clap::Parser as _;

/// This process's own standard streams.
struct Standard;

impl Front for Standard {
    fn print(&self, line: &str) -> std::io::Result<()> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|()| out.flush())
    }

    fn say(&self, line: &str) {
        eprintln!("{line}");
    }

    /// Ends this process when standard input closes, logging nothing first: a parent that died can leave standard error a
    /// pipe nobody reads, and on Windows a write to it then never returns.
    fn when_input_closes(&self, status: u8, then: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("canoa-stdin".to_owned())
            .spawn(move || {
                // Whatever arrives is not read as anything: only its end matters.
                let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
                // Should what runs first wait like that, the process ends anyway.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(5));
                    std::process::exit(i32::from(status));
                });
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
        "canoa"
    }
}

fn main() -> ExitCode {
    let parsed = Cli::parse();
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let filter = tracing_subscriber::EnvFilter::try_from_env("CANOA_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .with_target(false)
        .init();
    match cli::run(parsed, &Standard) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failed) => {
            eprintln!("canoa: {}", failed.why);
            ExitCode::from(failed.status)
        }
    }
}
