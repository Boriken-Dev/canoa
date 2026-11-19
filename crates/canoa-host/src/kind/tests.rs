//! The account provider a harness is built with: what it offers is what accounts of the provider can be, and its settings are its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use alloc::sync::Arc;

use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{HARNESS_LOGIN, standard};
use serde_json::json;

use super::{Harness, registry};
use crate::config::Settings;
use crate::provider::ProviderError;

fn built_in() -> Arc<Registry> {
    Arc::new(registry(None).expect("the built-in providers"))
}

fn settings(value: serde_json::Value) -> Settings {
    match value {
        serde_json::Value::Object(map) => Settings::new(map),
        other => panic!("settings are a table, not {other}"),
    }
}

fn harness(id: &str, harness: &Settings, accounts: &Settings) -> Result<Harness, ProviderError> {
    Harness::new(
        &built_in(),
        id,
        (harness, accounts),
        false,
        Vec::new(),
        Arc::default(),
    )
}

fn ids(harness: &Harness) -> Vec<String> {
    harness
        .listing()
        .kinds
        .iter()
        .map(|kind| kind.id.clone())
        .collect()
}

#[test]
fn claude_lists_the_kinds_and_sign_in_methods_its_account_provider_offers() {
    let claude = harness("claude", &Settings::default(), &Settings::default()).expect("builds");
    let listing = claude.listing();
    let login = listing
        .kind("claude-login")
        .expect("the subscription login");
    assert!(login.renews, "the account provider renews it");
    assert!(
        login
            .provides
            .iter()
            .any(|each| each == standard::CLAUDE_CODE)
    );
    assert!(listing.kind(HARNESS_LOGIN).is_some());
    assert!(
        listing
            .logins
            .iter()
            .any(|method| method.id == "subscription")
    );
    // The harness itself signs no one in and renews nothing.
    let own = claude.catalog().expect("the harness's own catalog");
    assert!(own.logins.is_empty(), "{:?}", own.logins);
    assert!(own.kinds.iter().all(|kind| !kind.renews));
    assert!(claude.account_provider().is_ok());
}

#[test]
fn pi_offers_its_own_login_only() {
    let pi = harness("pi", &Settings::default(), &Settings::default()).expect("builds");
    let kinds = ids(&pi);
    assert!(kinds.iter().any(|id| id == "pi-oauth"), "{kinds:?}");
    assert!(
        !kinds.iter().any(|id| id == "pi-api-key"),
        "a key is not one it offers: {kinds:?}"
    );
    assert!(
        pi.listing()
            .logins
            .iter()
            .all(|method| method.produces != "pi-api-key")
    );
}

#[test]
fn the_renewal_settings_are_the_account_providers_and_not_the_harnesss() {
    let own = settings(json!({"renew_poll_ms": 100}));
    let refused = harness("claude", &own, &Settings::default()).expect_err("not a harness setting");
    assert!(refused.to_string().contains("renew_poll_ms"), "{refused}");
    let codex = settings(json!({"command_timeout_ms": 5000}));
    harness("codex", &codex, &Settings::default())
        .expect("Codex's harness keeps its own command timeout");
    harness("claude", &Settings::default(), &own).expect("the account provider takes it");
}

#[test]
fn an_account_setting_the_provider_does_not_declare_is_refused_naming_the_provider() {
    let unknown = settings(json!({"no_such_setting": 1}));
    let refused =
        harness("claude", &Settings::default(), &unknown).expect_err("an unknown account setting");
    let why = refused.to_string();
    assert!(why.contains("account provider"), "{why}");
    assert!(why.contains("no_such_setting"), "{why}");
}

#[test]
fn a_harness_with_a_login_of_its_own_offers_it_as_the_local_account_and_names_it_by_default() {
    let config = crate::config::Config::from_str_in(
        "[host.agents.Pi]
harness = \"pi\"
",
        std::path::Path::new(""),
    )
    .expect("parses");
    for id in ["claude", "codex", "pi"] {
        let built = harness(id, &Settings::default(), &Settings::default()).expect("builds");
        let (name, account, info) = built.local_account().expect("it offers its own login");
        assert_eq!(name, coqui_acp_api::LOCAL_ACCOUNT, "{id}");
        assert_eq!(account.kind, HARNESS_LOGIN);
        assert_eq!(account.harness, id);
        assert_eq!(info.id, HARNESS_LOGIN);
        // An entry that names no account offers it, and one that names others offers those alone.
        let none = crate::serve::entry_accounts(&config, id, &[], &built)
            .expect("listed")
            .expect("an account");
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].0, coqui_acp_api::LOCAL_ACCOUNT);
        let named = crate::serve::entry_accounts(&config, id, &["local".to_owned()], &built)
            .expect("listed")
            .expect("an account");
        assert_eq!(named[0].0, coqui_acp_api::LOCAL_ACCOUNT);
    }
}

#[test]
fn only_a_kind_the_account_provider_offers_is_accepted_for_an_account() {
    let pi = harness("pi", &Settings::default(), &Settings::default()).expect("builds");
    let config = |kind: &str| {
        crate::config::Config::from_str_in(
            &format!(
                "[host.accounts.p]\nharness = \"pi\"\nkind = \"{kind}\"\nid = \"id-p\"\n[host.agents.Pi]\nharness = \"pi\"\n"
            ),
            std::path::Path::new(""),
        )
        .expect("parses")
    };
    let names = ["p".to_owned()];
    crate::serve::offered_accounts(&config("pi-oauth"), "pi", &names, &pi).expect("offered");
    let why = crate::serve::offered_accounts(&config("pi-api-key"), "pi", &names, &pi)
        .expect_err("a key is not offered");
    assert!(why.contains("pi-api-key"), "{why}");
}
