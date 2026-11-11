//! Holds a `program_store` provider in a library to the contract suite, on a world of documents a JSON file describes
//! (`canoa_host::store::contract::FileFixture`) and a stand-in program built by `cargo build --examples`.
//!
//! Exit status: 0 when every check passes, 1 when some rule is broken (each is listed), 2 when the check could not run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a command line names files and the process"
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use canoa::store::{ProgramStore, StoreSettings};
use canoa_host::store::contract::{FileFixture, run};
use clap::Parser;
use coqui_acp_api::guatiao::library::{ImportSearch, Registry};
use coqui_acp_api::guatiao::{Map, ToValue, Value};

/// Runs the program store contract suite against one provider.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// The library file that offers the provider.
    #[arg(long)]
    library: PathBuf,
    /// The provider's id.
    #[arg(long)]
    provider: String,
    /// The fixture: JSON naming the documents and files the publisher serves (see `FileFixture`).
    #[arg(long)]
    fixture: PathBuf,
    /// The stand-in program (`examples/version_standin`).
    #[arg(long)]
    standin: PathBuf,
    /// The provider's `channel` setting.
    #[arg(long)]
    channel: Option<String>,
    /// The provider's `version` setting.
    #[arg(long)]
    version_pin: Option<String>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match check(&args) {
        Ok(failures) if failures.is_empty() => {
            println!("ok: {} passes every check", args.provider);
            ExitCode::SUCCESS
        }
        Ok(failures) => {
            for failure in &failures {
                println!("FAIL {failure}");
            }
            ExitCode::from(1)
        }
        Err(why) => {
            eprintln!("could not run the suite: {why}");
            ExitCode::from(2)
        }
    }
}

fn check(args: &Args) -> Result<Vec<canoa_host::store::contract::Failure>, String> {
    let mut registry = Registry::new("canoa-store-check", env!("CARGO_PKG_VERSION"));
    registry.set_import_search(ImportSearch::Library);
    registry
        .load_file(&args.library)
        .map_err(|e| format!("{}: {e}", args.library.display()))?;
    let offer = registry
        .offer::<dyn ProgramStore>(&args.provider)
        .ok_or_else(|| format!("the library offers no provider {}", args.provider))?
        .map_err(|_| format!("{} is not a program_store", args.provider))?;
    let mut settings = StoreSettings::default();
    settings.channel.clone_from(&args.channel);
    settings.version.clone_from(&args.version_pin);
    let config: Value = if offer.builds_instances() {
        settings.to_value().map_err(|e| e.to_string())?
    } else {
        Value::from(Map::new())
    };
    let store: Arc<dyn ProgramStore> =
        Arc::new(offer.instantiate(&config).map_err(|e| e.to_string())?);
    let fixture = FileFixture::load(&args.fixture, args.standin.clone())?;
    Ok(run(store, &fixture))
}
