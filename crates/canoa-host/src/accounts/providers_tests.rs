//! Account providers listed once each, an account found by its kind, and the harnesses an account fits by its types. The providers
//! are built and asked what they offer; nothing is started and no login is read.

use std::path::Path;

use super::matching::{Catalogs, fitting};
use super::providers;
use crate::config::{AccountConfig, Config, Settings};

fn config(text: &str) -> Config {
    Config::from_str_in(text, Path::new("")).expect("parses")
}

fn account(kind: &str, settings: Option<serde_json::Value>) -> AccountConfig {
    AccountConfig {
        owner: None,
        kind: kind.to_owned(),
        id: "id".to_owned(),
        settings: settings.map(|value| Settings::new(value.as_object().cloned().expect("a table"))),
    }
}

#[test]
fn every_account_provider_is_listed_once_an_agents_own_with_its_agent_and_a_maker_with_none() {
    let listed = providers::all(&config(""));
    let ids: Vec<&str> = listed.iter().map(|each| each.id.as_str()).collect();
    let mut once = ids.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(
        once.len(),
        ids.len(),
        "each provider is listed once: {ids:?}"
    );
    for (id, harness) in [
        ("claude_accounts", Some("claude")),
        ("codex_accounts", Some("codex")),
    ] {
        let row = listed.iter().find(|each| each.id == id).expect(id);
        assert_eq!(row.harness.as_deref(), harness, "{id}");
    }
    for maker in [
        "litellm_accounts",
        "foundry_accounts",
        "api_gateway_accounts",
    ] {
        let row = listed.iter().find(|each| each.id == maker).expect(maker);
        assert_eq!(row.harness, None, "{maker} belongs to no agent");
    }
}

#[test]
fn a_makers_sign_in_ways_are_its_own_and_no_agents() {
    let listed = providers::all(&config(""));
    let litellm = listed
        .iter()
        .find(|each| each.id == "litellm_accounts")
        .expect("litellm");
    assert!(
        litellm
            .offer
            .logins
            .iter()
            .any(|login| login.produces == "litellm"),
        "LiteLLM signs in to its own kind"
    );
    for row in listed.iter().filter(|each| each.harness.is_some()) {
        assert!(
            row.offer.kinds.iter().all(|kind| kind.id != "litellm"),
            "{} does not offer LiteLLM's kind",
            row.id
        );
        assert!(
            row.offer
                .logins
                .iter()
                .all(|login| login.produces != "litellm"),
            "{} does not sign in to LiteLLM",
            row.id
        );
    }
}

#[test]
fn an_account_is_found_by_its_kind_in_the_provider_that_makes_it() {
    let config = config("");
    let catalogs = Catalogs::new(&config);
    let (made, _) = catalogs.made("claude-login").expect("a Claude kind");
    assert_eq!(made.provider, "claude_accounts");
    assert_eq!(made.harness.as_deref(), Some("claude"));
    let (made, _) = catalogs.made("litellm").expect("a maker's kind");
    assert_eq!(made.provider, "litellm_accounts");
    assert_eq!(made.harness, None);
    assert!(catalogs.made("no-such-kind").is_none());
}

#[test]
fn a_claude_login_fits_claude_only() {
    let config = config("");
    let catalogs = Catalogs::new(&config);
    let account = account("claude-login", None);
    let info = catalogs.kind(&account).expect("its kind");
    assert_eq!(fitting(&catalogs, &account, &info), ["claude"]);
}

#[test]
fn a_litellm_account_has_no_agent_and_fits_the_agents_that_can_move_the_endpoint() {
    let config = config("");
    let catalogs = Catalogs::new(&config);
    let account = account(
        "litellm",
        Some(serde_json::json!({"base_url": "https://llm.example.com"})),
    );
    let info = catalogs.kind(&account).expect("its kind");
    assert!(
        info.provides
            .iter()
            .any(|standard| standard == "anthropic-api")
            && info
                .provides
                .iter()
                .any(|standard| standard == "openai-api"),
        "{:?}",
        info.provides
    );
    let fits = fitting(&catalogs, &account, &info);
    assert!(
        fits.iter().any(|each| each == "claude") && fits.iter().any(|each| each == "codex"),
        "{fits:?}"
    );
    assert!(
        fits.iter().all(|each| each != "grok"),
        "Grok takes the Anthropic and OpenAI APIs only at its vendor's own address: {fits:?}"
    );
}

#[test]
fn an_account_that_names_a_harness_is_refused_with_the_key_named() {
    let why = Config::from_str_in(
        "[host.accounts.work]\nharness = \"claude\"\nkind = \"claude-login\"\nid = \"id-work\"\n",
        Path::new(""),
    )
    .expect_err("an account names no harness")
    .to_string();
    assert!(why.contains("work") && why.contains("harness"), "{why}");
    config("[host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n");
}
