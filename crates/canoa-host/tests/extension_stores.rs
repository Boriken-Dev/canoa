//! A store extension named in canoa.cfg adds a store type: the control channel lists the type, lists its stores, and plans,
//! installs and removes from them as it does for the built-in ones, while a bad extension is skipped and the host still runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::PathBuf;
use std::sync::Arc;

use canoa_host::store::builtin::EchoStore;
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
    providers = [EchoStore]
);

fn programs(root: PathBuf, home: PathBuf) -> Programs {
    let mut registry = Registry::new("test", "0.0.0");
    registry
        .register_local("test", library)
        .expect("registered");
    let fixture = EchoFixture {
        standin: example("version_standin", false),
        archive: true,
    };
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
    Programs::new(Shelf::unchecked(root), Arc::new(registry))
        .transport(WorldTransport::new(fixture.world(Variant::Normal)))
        .options(options)
}

#[test]
fn an_extension_adds_a_type_the_host_lists_and_installs_from() {
    let scratch = Scratch::new();
    let home = Home::at(scratch.0.join("home"));
    let _lock = home.lock("a test").expect("a free home");
    std::fs::create_dir_all(scratch.0.join("user")).expect("a home for the installs");
    let library = example("sample_extension", true);
    let config = Config::from_str_in(
        &format!(
            "[host.agents.fixture]\nprogram = {}\n\n\
             [host.extensions.sample]\nlibrary = {}\nprovider = \"sample_extension\"\n\n\
             [host.extensions.broken]\nlibrary = {}\nprovider = \"no_such_extension\"\n",
            quoted(&example("stdio_agent", false).to_string_lossy()),
            quoted(&library.to_string_lossy()),
            quoted(&library.to_string_lossy()),
        ),
        &scratch.0,
    )
    .expect("parses");
    assert_eq!(config.extensions.len(), 2);
    let identity = home.identity(Some(support::method())).expect("an identity");
    let shelf = scratch.0.join("programs");
    let serving = Serving::bind(
        &config,
        identity,
        home.store().expect("a store"),
        &ServeOptions::new(host_binary())
            .with_programs(
                Arc::new(programs(shelf, scratch.0.join("user"))),
                vec!["echo_store".to_owned()],
            )
            // A host serves its home, where the scripts that ship in it are kept: never the person's own.
            .with_state_dir(home.dir()),
    )
    .expect("it binds");
    let local = serving.local();
    let stopper = serving.stopper();
    let run = std::thread::spawn(move || serving.run());
    let link = local.open("@control").expect("a link");

    let (types, _) = ask(&link, 1, "store/extensions", json!({})).expect("the types");
    let types = types["types"].as_array().expect("a list");
    assert_eq!(
        types.len(),
        2,
        "the broken extension adds nothing: {types:?}"
    );
    assert_eq!(types[0]["icon"], "extension");
    assert_eq!(types[1]["id"], "pictured");
    assert!(
        types[1]["icon"]
            .as_str()
            .is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
        "an image icon is given as the extension wrote it: {types:?}"
    );
    assert_eq!(types[0]["id"], "echoes");
    assert_eq!(types[0]["title"], "Echoes");
    assert_eq!(types[0]["extension"], "sample");
    assert_eq!(types[0]["stores"], json!(["echo_archive_store"]));

    // The Agents type does not list it; the extension's own type does.
    let (agents, _) = ask(&link, 2, "store/list", json!({})).expect("agents");
    assert_eq!(
        agents["stores"].as_array().map(Vec::len),
        Some(1),
        "{agents}"
    );
    let (listed, _) = ask(
        &link,
        3,
        "store/list",
        json!({"kind": "extension", "type": "echoes"}),
    )
    .expect("the type's stores");
    assert_eq!(
        listed["stores"][0]["provider"], "echo_archive_store",
        "{listed}"
    );
    assert_eq!(listed["stores"][0]["program"], "echo-archive", "{listed}");
    let none = ask(
        &link,
        4,
        "store/list",
        json!({"kind": "extension", "type": "nothing"}),
    )
    .expect_err("no such type");
    assert!(none.contains("-32003"), "{none}");

    let (plan, _) = ask(
        &link,
        5,
        "store/plan",
        json!({"provider": "echo_archive_store", "action": "install", "version": "1.0.0"}),
    )
    .expect("a plan");
    let (applied, _) = ask(
        &link,
        6,
        "store/apply",
        json!({"provider": "echo_archive_store", "plan": plan}),
    )
    .expect("applied");
    assert_eq!(applied["applied"], true, "{applied}");
    let (removed, _) = ask(
        &link,
        7,
        "store/remove",
        json!({"provider": "echo_archive_store"}),
    )
    .expect("removed");
    assert_eq!(removed["removed"], true, "{removed}");

    stopper.stop();
    run.join().expect("run returned");
}
