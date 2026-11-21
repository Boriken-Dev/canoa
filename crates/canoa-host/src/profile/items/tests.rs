use canoa::profile_item::{ItemEffect, NetworkEffect, NetworkMode, Setting, Variable};
use coqui_acp_harness::{Approvals, Sandbox};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::standin::{registry, with_extension};
use super::{Applied, ItemCatalog, Outcome, SETTING_IDS, network_of, session_layer, variables_of};

fn values(value: &Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

fn one(id: &str, held: &Value) -> IndexMap<String, Map<String, Value>> {
    let mut all = IndexMap::new();
    all.insert(id.to_owned(), values(held));
    all
}

fn done(applied: &[Applied], id: &str) -> ItemEffect {
    match &applied
        .iter()
        .find(|each| each.id == id)
        .expect("applied")
        .outcome
    {
        Outcome::Done(effect) => effect.clone(),
        Outcome::Not(why) => panic!("{id} was not applied: {why}"),
    }
}

fn not(applied: &[Applied], id: &str) -> String {
    match &applied
        .iter()
        .find(|each| each.id == id)
        .expect("applied")
        .outcome
    {
        Outcome::Not(why) => why.clone(),
        Outcome::Done(_) => panic!("{id} was applied"),
    }
}

#[test]
fn the_items_are_the_providers_the_registries_offer_the_built_in_ones_first() {
    let catalog = with_extension();
    let listed = catalog.list();
    let seen: Vec<(&str, &str)> = listed
        .iter()
        .map(|each| (each.info.id.as_str(), each.origin.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("access", "built-in"),
            ("context", "built-in"),
            ("effort", "built-in"),
            ("mode", "built-in"),
            ("model", "built-in"),
            ("identity", "bundled"),
            ("proxy", "bundled"),
            ("region", "regions"),
        ]
    );
    for each in &listed {
        assert_eq!(each.info.problem(), None, "{}", each.info.id);
    }
    // The session's own settings are kept under their own names, the rest under `items`.
    let stored: Vec<(String, String)> = listed
        .iter()
        .map(|each| {
            (
                each.info.id.clone(),
                each.json()["stored"].as_str().expect("text").to_owned(),
            )
        })
        .collect();
    for (id, how) in stored {
        assert_eq!(
            how,
            if SETTING_IDS.contains(&id.as_str()) {
                "setting"
            } else {
                "item"
            },
            "{id}"
        );
    }
}

#[test]
fn an_item_whose_id_an_earlier_provider_took_is_offered_once() {
    let twice = ItemCatalog::from_registries(vec![
        ("regions".to_owned(), registry()),
        ("again".to_owned(), registry()),
    ]);
    let regions: Vec<_> = twice
        .list()
        .into_iter()
        .filter(|each| each.info.id == "region")
        .collect();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].origin, "regions");
}

#[test]
fn values_that_fit_an_items_form_are_turned_into_what_its_provider_says() {
    let catalog = with_extension();
    let applied = catalog.apply_all(
        &one(
            "region",
            &json!({"name": "north", "hosts": ["a.example", "b.example"], "tier": "fast"}),
        ),
        Some("claude-code"),
    );
    let effect = done(&applied, "region");
    assert_eq!(
        effect.env,
        [
            Variable::new("REGION", "north"),
            Variable::new("REGION_HOSTS", "a.example,b.example"),
        ]
    );
    assert_eq!(effect.settings, [Setting::new("model", "fast-model")]);
    assert_eq!(
        applied[0].how(),
        "puts the session in a region; gives the agent's programs REGION, REGION_HOSTS",
        "what it became is said with names and no value"
    );
    // A setting an item sets is the session's own.
    let (layer, refused) = session_layer(&applied);
    assert_eq!(layer.model.as_deref(), Some("fast-model"));
    assert!(refused.is_empty());
}

#[test]
fn values_that_do_not_fit_the_form_or_a_harness_it_is_not_for_are_not_applied_and_say_why() {
    let catalog = with_extension();
    let try_it = |held: Value, harness: Option<&str>| {
        let applied = catalog.apply_all(&one("region", &held), harness);
        not(&applied, "region")
    };
    let ok = json!({"name": "n"});
    assert!(try_it(json!({"nom": "n"}), Some("claude-code")).contains("unknown field 'nom'"));
    assert!(try_it(json!({}), Some("claude-code")).contains("'name'"));
    assert!(try_it(json!({"name": 3}), Some("claude-code")).contains("'name'"));
    assert!(try_it(json!({"name": "n", "tier": "slow"}), Some("claude-code")).contains("'tier'"));
    assert!(try_it(json!({"name": "n", "hosts": [1]}), Some("claude-code")).contains("hosts"));
    assert!(try_it(ok.clone(), Some("codex")).contains("claude-code only"));
    assert!(try_it(ok, None).contains("claude-code only"));
}

#[test]
fn an_item_no_provider_offers_keeps_its_values_and_is_not_applied() {
    // The same profile on a host whose extension was taken away.
    let held = one("region", &json!({"name": "north"}));
    let applied = ItemCatalog::built_in().apply_all(&held, Some("claude-code"));
    assert!(not(&applied, "region").contains("no provider offers this item"));
    assert_eq!(applied[0].origin, None);
    assert_eq!(
        held["region"]["name"], "north",
        "the profile still holds them"
    );
    // And with it back, they are applied.
    let applied = with_extension().apply_all(&held, Some("claude-code"));
    assert_eq!(
        done(&applied, "region").env[0],
        Variable::new("REGION", "north")
    );
}

#[test]
fn the_built_in_items_turn_a_profile_into_the_sessions_own_settings() {
    let mut all = IndexMap::new();
    all.insert("model".to_owned(), values(&json!({"model": "opus"})));
    all.insert("effort".to_owned(), values(&json!({"effort": "high"})));
    all.insert("mode".to_owned(), values(&json!({"mode": "plan"})));
    all.insert(
        "context".to_owned(),
        values(&json!({"context": "Be brief."})),
    );
    all.insert(
        "access".to_owned(),
        values(&json!({"sandbox": "read-only", "network": false, "approvals": "never"})),
    );
    let applied = ItemCatalog::built_in().apply_all(&all, None);
    let (layer, refused) = session_layer(&applied);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(layer.model.as_deref(), Some("opus"));
    assert_eq!(layer.effort.as_deref(), Some("high"));
    assert_eq!(layer.mode.as_deref(), Some("plan"));
    assert_eq!(layer.access.sandbox, Some(Sandbox::ReadOnly));
    assert_eq!(layer.access.network, Some(false));
    assert_eq!(layer.access.approvals, Some(Approvals::Never));
    assert_eq!(layer.contexts.len(), 1);
    // A value the form does not allow is not applied, whatever the file said.
    let bad = ItemCatalog::built_in().apply_all(&one("access", &json!({"sandbox": "open"})), None);
    assert!(not(&bad, "access").contains("'sandbox'"));
}

#[test]
fn a_setting_a_session_does_not_have_is_refused_by_key_and_the_rest_still_applies() {
    let applied = vec![Applied {
        id: "odd".to_owned(),
        title: "Odd".to_owned(),
        origin: Some("x".to_owned()),
        outcome: Outcome::Done({
            let mut effect = ItemEffect::none();
            effect.settings = vec![
                Setting::new("temperature", "1"),
                Setting::new("model", "m"),
                Setting::new("access.network", "maybe"),
                Setting::new("access.sandbox", "open"),
            ];
            effect
        }),
    }];
    let (layer, refused) = session_layer(&applied);
    assert_eq!(layer.model.as_deref(), Some("m"));
    let keys: Vec<&str> = refused.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["temperature", "access.network", "access.sandbox"]);
}

#[test]
fn the_first_item_to_set_a_variable_keeps_it_and_the_last_to_ask_decides_the_network() {
    let mut first = ItemEffect::none();
    first.env = vec![Variable::new("A", "1"), Variable::new("B", "1")];
    first.network = Some(NetworkEffect::new(NetworkMode::None));
    let mut second = ItemEffect::none();
    second.env = vec![Variable::new("A", "2"), Variable::new("C", "2")];
    second.network = Some(NetworkEffect::new(NetworkMode::System));
    let applied = vec![
        Applied {
            id: "one".to_owned(),
            title: "One".to_owned(),
            origin: None,
            outcome: Outcome::Done(first),
        },
        Applied {
            id: "two".to_owned(),
            title: "Two".to_owned(),
            origin: None,
            outcome: Outcome::Done(second),
        },
    ];
    let seen: Vec<(&str, &str, &str)> = variables_of(&applied)
        .into_iter()
        .map(|(id, variable)| (id, variable.name.as_str(), variable.value.as_str()))
        .collect();
    assert_eq!(
        seen,
        [("one", "A", "1"), ("one", "B", "1"), ("two", "C", "2")]
    );
    assert_eq!(
        network_of(&applied).map(|each| each.mode.as_str()),
        Some("system")
    );
}

#[test]
fn the_bundled_identity_gives_git_its_name_and_address_and_the_account_label() {
    let applied = ItemCatalog::built_in().apply_all(
        &one(
            "identity",
            &json!({"name": "Ana Ruiz", "email": "ana@example.test", "account": "work", "variables": ["TEAM=core"]}),
        ),
        None,
    );
    let effect = done(&applied, "identity");
    let names: Vec<&str> = effect.env.iter().map(|each| each.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "GIT_AUTHOR_NAME",
            "GIT_COMMITTER_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_EMAIL",
            "AGENTS_ACCOUNT",
            "TEAM"
        ]
    );
    assert_eq!(effect.env[0].value, "Ana Ruiz");
    assert_eq!(effect.env[3].value, "ana@example.test");
    // A line that is not NAME=value is the provider's refusal, not a variable.
    let bad =
        ItemCatalog::built_in().apply_all(&one("identity", &json!({"variables": ["TEAM"]})), None);
    assert!(not(&bad, "identity").contains("NAME=value"));
    // Nothing set is nothing done.
    let empty = ItemCatalog::built_in().apply_all(&one("identity", &json!({})), None);
    assert!(done(&empty, "identity").env.is_empty());
    assert_eq!(empty[0].how(), "sets nothing for this session");
}

#[test]
fn the_bundled_proxy_asks_for_the_network_its_mode_names() {
    let catalog = ItemCatalog::built_in();
    let mode = |held: Value| {
        let applied = catalog.apply_all(&one("proxy", &held), None);
        done(&applied, "proxy").network.expect("a network")
    };
    assert_eq!(
        mode(json!({})).mode,
        NetworkMode::System,
        "system is the default"
    );
    assert_eq!(mode(json!({"mode": "none"})).mode, NetworkMode::None);
    let own = mode(
        json!({"mode": "proxy", "url": "http://p.example:3128", "bypass": ["a.example"], "user": "me", "password_variable": "PW"}),
    );
    assert_eq!(own.url.as_deref(), Some("http://p.example:3128"));
    assert_eq!(own.bypass, ["a.example"]);
    assert_eq!(own.password_variable.as_deref(), Some("PW"));
    let command = mode(json!({"mode": "command", "command": ["myproxy", "--listen", "{listen}"]}));
    assert_eq!(command.command, ["myproxy", "--listen", "{listen}"]);
    // An address is needed for a proxy and a program for a command.
    for held in [
        json!({"mode": "proxy"}),
        json!({"mode": "pac"}),
        json!({"mode": "command"}),
    ] {
        let applied = catalog.apply_all(&one("proxy", &held), None);
        assert!(not(&applied, "proxy").contains("needs"), "{held}");
    }
}
