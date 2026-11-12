//! The host's control channel over standard streams, with the echo program's stores (a made-up program under a made-up publisher,
//! installed from a fake publisher into a temporary folder) as its only Store. The app's tests use it to run its Store code
//! against the real `store/*` methods. A line on standard input is a link's name, a space and either one message to `@control`
//! or the word `close`; a line on standard output is a link's name, a space and one message from it. Each name is its own link
//! (opened by its first line), as the app opens one for an install, so closing one ends only what that link was doing. It
//! serves until standard input closes.
//!
//! Usage: `store_control <directory>`; the directory holds the host's home, the programs folder and the user's folders, and is
//! the caller's to remove.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::collections::HashMap;
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa_host::store::builtin::{EchoArchiveStore, EchoStore};
use canoa_host::store::contract::{Fixture as _, Variant, WorldTransport};
use canoa_host::store::echo::EchoFixture;
use canoa_host::store::programs::{Options, Programs};
use canoa_host::store::shelf::Shelf;
use canoa_host::{Config, Home, Link, LinkError, ServeOptions, Serving};
use coqui_acp_api::guatiao::library::Registry;

coqui_acp_api::guatiao::local_providers!(
    id = "echo-stores",
    version = "0.0.0",
    providers = [EchoStore, EchoArchiveStore]
);

fn sibling(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("this program knows where it is");
    exe.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn programs(root: PathBuf, home: PathBuf) -> Programs {
    let mut registry = Registry::new("test", "0.0.0");
    registry
        .register_local("test", library)
        .expect("registered");
    let fixture = EchoFixture {
        standin: sibling("version_standin"),
        archive: false,
    };
    let mut options = Options::default();
    options.home = Some(home);
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
    Programs::new(Shelf::unchecked(root), Arc::new(registry))
        .transport(WorldTransport::new(fixture.world(Variant::Normal)))
        .options(options)
}

fn serve(dir: &Path) -> Result<(), String> {
    let home = Home::at(dir.join("home"));
    let _lock = home
        .lock("store_control")
        .map_err(|error| error.to_string())?;
    std::fs::create_dir_all(dir.join("user")).map_err(|error| error.to_string())?;
    let agent = sibling("stdio_agent");
    let config = Config::from_str_in(
        &format!(
            "[host.agents.fixture]\nprogram = {}\n",
            serde_json::to_string(&agent.to_string_lossy()).map_err(|error| error.to_string())?
        ),
        dir,
    )
    .map_err(|error| error.to_string())?;
    let identity = home.identity(None).map_err(|error| error.to_string())?;
    let store = home.store().map_err(|error| error.to_string())?;
    let stores = Arc::new(programs(dir.join("programs"), dir.join("user")));
    let options = ServeOptions::new(sibling("canoa").to_string_lossy().into_owned())
        .with_programs(stores, vec!["echo_store".to_owned()]);
    let serving =
        Serving::bind(&config, identity, store, &options).map_err(|error| error.to_string())?;
    let local = serving.local();
    let stopper = serving.stopper();
    let run = std::thread::spawn(move || serving.run());
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let mut links: HashMap<String, Arc<Link>> = HashMap::new();
    let mut readers = Vec::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Some((name, rest)) = line.trim().split_once(' ') else {
            continue;
        };
        if rest == "close" {
            if let Some(link) = links.remove(name) {
                link.close();
            }
            continue;
        }
        if !links.contains_key(name) {
            let link = Arc::new(local.open("@control").map_err(|error| error.to_string())?);
            let (link_name, out, reading) =
                (name.to_owned(), Arc::clone(&stdout), Arc::clone(&link));
            readers.push(std::thread::spawn(move || {
                loop {
                    match reading.receive(Some(Duration::from_millis(100))) {
                        Ok(Some(message)) => {
                            let mut out = out.lock().unwrap_or_else(PoisonError::into_inner);
                            let _ = out.write_all(link_name.as_bytes());
                            let _ = out.write_all(b" ");
                            let _ = out.write_all(&message);
                            let _ = out.write_all(b"\n");
                            let _ = out.flush();
                        }
                        Ok(None) | Err(LinkError::Ended) => break,
                        Err(_) => {}
                    }
                }
            }));
            links.insert(name.to_owned(), link);
        }
        if let Some(link) = links.get(name) {
            let _ = link.send(rest.as_bytes());
        }
    }
    for link in links.values() {
        link.close();
    }
    for reader in readers {
        let _ = reader.join();
    }
    stopper.stop();
    let _ = run.join();
    Ok(())
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: store_control <directory>");
        return ExitCode::from(2);
    };
    match serve(Path::new(&dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("store_control: {why}");
            ExitCode::FAILURE
        }
    }
}
