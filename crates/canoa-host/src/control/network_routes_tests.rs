//! `routes/*` and `accounts/set_route` over a canoa.cfg of their own: what each writes, what each refuses, and that a refusal writes
//! nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use serde_json::{Value, json};

use super::network_routes::{list, make_default, offers, remove, set, set_account_route, test};
use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::config::Config;
use crate::config_store::FileStore;
use crate::network::tests::Dir;

const START: &str = "[host.accounts.work]\nkind = \"k\"\nid = \"id-1\"\n";

/// A canoa.cfg of its own, and the configuration it holds now.
struct Home {
    dir: Dir,
}

impl Home {
    fn new(name: &str) -> Self {
        let dir = Dir::new(name);
        std::fs::write(dir.0.join("canoa.cfg"), START).expect("writes");
        Self { dir }
    }

    fn store(&self) -> FileStore {
        FileStore::at(&self.dir.0.join("canoa.cfg"))
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.dir.0.join("canoa.cfg")).expect("reads")
    }

    fn config(&self) -> Config {
        Config::from_str_in(&self.text(), &self.dir.0).expect("parses")
    }
}

fn row(answer: &Value, name: &str) -> Value {
    answer["routes"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|each| each["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no row {name}: {answer}"))
}

#[test]
fn the_host_lists_system_and_direct_and_offers_every_provider_with_its_schema() {
    let home = Home::new("routes-list");
    let listed = list(&home.config());
    assert_eq!(listed["default"], "system");
    let system = row(&listed, "system");
    assert!(
        system["builtin"] == true && system["default"] == true,
        "{system}"
    );
    assert_eq!(row(&listed, "direct")["default"], false);
    let offered = offers(&home.config());
    let all = offered["offers"].as_array().expect("offers");
    for wanted in ["system", "direct", "manual", "pac", "command"] {
        assert!(
            all.iter().any(|each| each["provider"] == wanted),
            "{wanted}"
        );
    }
    let manual = all
        .iter()
        .find(|each| each["provider"] == "manual")
        .expect("manual");
    assert!(
        manual["schema"]["properties"]["all_proxy"].is_object(),
        "{manual}"
    );
    assert_eq!(manual["secrets"], json!(["password_env"]));
}

#[test]
fn a_route_is_made_judged_made_the_default_and_removed_only_when_nothing_uses_it() {
    let home = Home::new("routes-set");
    let made = set(
        &home.store(),
        &home.config(),
        &json!({"name": "corp", "provider": "manual", "settings": {"https_proxy": "proxy.corp:8080"}}),
    )
    .expect("made");
    assert_eq!(made["route"]["name"], "corp");
    assert!(
        home.text().contains("[network.routes.corp]") || home.text().contains("routes"),
        "{}",
        home.text()
    );
    assert!(
        home.text().contains("[host.accounts.work]"),
        "what was there stays"
    );
    let config = home.config();
    assert_eq!(config.network.routes["corp"].provider, "manual");
    // The provider judges the settings first: a route with no proxy, a name that is taken, a provider nobody offers and a setting it
    // does not take are refused, and nothing is written.
    let before = home.text();
    for bad in [
        json!({"name": "empty", "provider": "manual"}),
        json!({"name": "system", "provider": "manual", "settings": {"all_proxy": "p:1"}}),
        json!({"name": "x", "provider": "nobody", "settings": {}}),
        json!({"name": "y", "provider": "manual", "settings": {"all_proxy": "p:1", "surprise": 1}}),
    ] {
        let (code, why) = set(&home.store(), &config, &bad).expect_err("refused");
        assert!(code == REFUSED || code == BAD_PARAMS, "{bad}: {code} {why}");
    }
    assert_eq!(home.text(), before, "a refused route writes nothing");
    // An account and the host name it: it cannot be removed, and each is named.
    set_account_route(
        &home.store(),
        &config,
        &json!({"account": "work", "route": "corp"}),
    )
    .expect("set");
    make_default(&home.store(), &home.config(), &json!({"name": "corp"})).expect("default");
    let config = home.config();
    assert_eq!(config.accounts["work"].route.as_deref(), Some("corp"));
    assert_eq!(config.network.default_route(), "corp");
    let listed = list(&config);
    assert_eq!(row(&listed, "corp")["uses"]["accounts"], json!(["work"]));
    assert_eq!(row(&listed, "corp")["uses"]["host"], true);
    let (_, why) = remove(&home.store(), &config, &json!({"name": "corp"})).expect_err("in use");
    assert!(why.contains("default"), "{why}");
    make_default(&home.store(), &config, &json!({"name": "system"})).expect("default");
    let (_, why) =
        remove(&home.store(), &home.config(), &json!({"name": "corp"})).expect_err("in use");
    assert!(why.contains("work"), "{why}");
    set_account_route(&home.store(), &home.config(), &json!({"account": "work"}))
        .expect("taken back");
    assert_eq!(home.config().accounts["work"].route, None);
    remove(&home.store(), &home.config(), &json!({"name": "corp"})).expect("removed");
    assert!(home.config().network.routes.is_empty());
    // A built-in cannot be removed, a route that is not there cannot be, and an account's route must be one the host has.
    let built_in =
        remove(&home.store(), &home.config(), &json!({"name": "direct"})).expect_err("built-in");
    assert_eq!(built_in.0, REFUSED);
    let none = remove(&home.store(), &home.config(), &json!({"name": "ghost"})).expect_err("none");
    assert_eq!(none.0, NOT_FOUND);
    let ghost = set_account_route(
        &home.store(),
        &home.config(),
        &json!({"account": "work", "route": "ghost"}),
    )
    .expect_err("none");
    assert_eq!(ghost.0, NOT_FOUND);
    let nobody = set_account_route(
        &home.store(),
        &home.config(),
        &json!({"account": "nobody", "route": "direct"}),
    )
    .expect_err("none");
    assert_eq!(nobody.0, NOT_FOUND);
}

#[test]
fn a_route_is_tried_against_an_address_the_caller_names() {
    let home = Home::new("routes-test");
    let config = home.config();
    assert_eq!(
        test(&config, &json!({"name": "ghost"}))
            .expect_err("none")
            .0,
        NOT_FOUND
    );
    assert_eq!(
        test(&config, &json!({"name": "direct"}))
            .expect_err("no address")
            .0,
        BAD_PARAMS
    );
    let tried = test(
        &config,
        &json!({"name": "direct", "url": "http://127.0.0.1:9/"}),
    )
    .expect("answers");
    assert_eq!(tried["ok"], true, "{tried}");
    assert_eq!(tried["direct"], true, "{tried}");
}
