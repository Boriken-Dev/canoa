//! The program Store over the control channel: a plan the host made is handed back and applied, a copy installed is reported and
//! rolled back and removed, and what the host did not make is refused. The store is a made-up program under a made-up publisher.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::PathBuf;
use std::sync::Arc;

use canoa_host::store::builtin::{EchoArchiveStore, EchoStore};
use canoa_host::store::contract::{Fixture, Variant, WorldTransport};
use canoa_host::store::echo::EchoFixture;
use canoa_host::store::programs::{Options, Programs};
use canoa_host::store::shelf::Shelf;
use canoa_host::{Config, Home, ServeOptions, Serving};
use coqui_acp_api::guatiao::library::Registry;
use linked::ask;
use serde_json::json;
use support::{Scratch, example, host_binary, quoted};

coqui_acp_api::guatiao::local_providers!(
    id = "echo-stores",
    version = "0.0.0",
    providers = [EchoStore, EchoArchiveStore]
);

fn registry() -> Arc<Registry> {
    let mut registry = Registry::new("test", "0.0.0");
    registry
        .register_local("test", library)
        .expect("registered");
    Arc::new(registry)
}

fn standin() -> PathBuf {
    example("version_standin", false)
}

fn programs(root: PathBuf, home: PathBuf) -> Programs {
    let fixture = EchoFixture {
        standin: standin(),
        archive: false,
    };
    let transport = WorldTransport::new(fixture.world(Variant::Normal));
    let mut options = Options::default();
    options.home = Some(home);
    options.watch_registry = false;
    options.settle = std::time::Duration::from_millis(100);
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
    Programs::new(Shelf::unchecked(root), registry())
        .transport(transport)
        .options(options)
}

#[test]
fn a_program_is_planned_installed_reported_rolled_back_and_removed_over_the_link() {
    let scratch = Scratch::new();
    let home = Home::at(scratch.0.join("home"));
    let _lock = home.lock("a test").expect("a free home");
    std::fs::create_dir_all(scratch.0.join("user")).expect("a home for the installs");
    let config = Config::from_str_in(
        &format!(
            "[host.agents.fixture]\nprogram = {}\n",
            quoted(&example("stdio_agent", false).to_string_lossy())
        ),
        &scratch.0,
    )
    .expect("parses");
    let identity = home.identity(Some(support::method())).expect("an identity");
    let shelf = scratch.0.join("programs");
    let serving = Serving::bind(
        &config,
        identity,
        home.store().expect("a store"),
        &ServeOptions::new(host_binary()).with_programs(
            Arc::new(programs(shelf.clone(), scratch.0.join("user"))),
            vec!["echo_store".to_owned()],
        ),
    )
    .expect("it binds");
    let local = serving.local();
    let stopper = serving.stopper();
    let run = std::thread::spawn(move || serving.run());
    let link = local.open("@control").expect("a link");

    let (listed, _) = ask(&link, 1, "store/list", json!({})).expect("listed");
    assert_eq!(listed["stores"][0]["provider"], "echo_store", "{listed}");
    assert_eq!(listed["stores"][0]["program"], "echo", "{listed}");

    let (plan, _) = ask(
        &link,
        2,
        "store/plan",
        json!({"provider": "echo_store", "action": "install", "version": "1.0.0"}),
    )
    .expect("a plan");
    assert_eq!(plan["version"], "1.0.0", "{plan}");
    assert!(
        plan["token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "{plan}"
    );

    // A plan the host did not make is refused; the one it made is carried out.
    let mut forged = plan.clone();
    forged["token"] = json!("not-a-token");
    let refused = ask(
        &link,
        3,
        "store/apply",
        json!({"provider": "echo_store", "plan": forged}),
    )
    .expect_err("refused");
    assert!(refused.contains("-32002"), "{refused}");
    let (applied, _) = ask(
        &link,
        4,
        "store/apply",
        json!({"provider": "echo_store", "plan": plan}),
    )
    .expect("applied");
    assert_eq!(applied["applied"], true, "{applied}");
    assert_eq!(applied["version"], "1.0.0", "{applied}");

    let (report, _) = ask(
        &link,
        5,
        "store/report",
        json!({"provider": "echo_store", "probe": true}),
    )
    .expect("a report");
    assert_eq!(report["copies"][0]["version"], "1.0.0", "{report}");
    assert_eq!(report["copies"][0]["managed"], true, "{report}");

    let (latest, _) =
        ask(&link, 6, "store/latest", json!({"provider": "echo_store"})).expect("the newest");
    assert_eq!(latest["version"], "2.0.0", "{latest}");

    let (update, _) = ask(
        &link,
        7,
        "store/plan",
        json!({"provider": "echo_store", "action": "update"}),
    )
    .expect("an update plan");
    ask(
        &link,
        8,
        "store/apply",
        json!({"provider": "echo_store", "plan": update}),
    )
    .expect("updated");
    let (back, _) = ask(
        &link,
        9,
        "store/rollback",
        json!({"provider": "echo_store"}),
    )
    .expect("rolled back");
    assert_eq!(back["version"], "1.0.0", "{back}");
    let (removed, _) =
        ask(&link, 10, "store/remove", json!({"provider": "echo_store"})).expect("removed");
    assert_eq!(removed["removed"], true, "{removed}");

    let unknown = ask(
        &link,
        11,
        "store/report",
        json!({"provider": "no_such_store"}),
    )
    .expect_err("no such store");
    assert!(unknown.contains("-32003"), "{unknown}");

    stopper.stop();
    run.join().expect("run returned");
}
