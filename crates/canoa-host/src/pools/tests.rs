//! Pools of accounts: which accounts a pool may hold, the host's own `any-<standard>` pools and what an entry offers of them. The
//! kinds are the bundled providers' own: no credential is read.

use super::*;

fn config(text: &str) -> Config {
    Config::from_str_in(text, std::path::Path::new("")).expect("parses")
}

/// Two Anthropic-API accounts, a Claude Code login, and an OpenAI-API key made through Codex's provider.
const ACCOUNTS: &str = "\
[host.accounts.api-one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
[host.accounts.api-two]\nkind = \"anthropic-api-key\"\nid = \"id-two\"\n\
[host.accounts.login]\nkind = \"claude-login\"\nid = \"id-login\"\n\
[host.accounts.openai]\nkind = \"codex-api-key\"\nid = \"id-openai\"\n";

fn with(extra: &str) -> Config {
    config(&format!("{ACCOUNTS}{extra}"))
}

fn names(pool: &AccountPool) -> Vec<&str> {
    pool.accounts.iter().map(String::as_str).collect()
}

fn pool_named<'a>(pools: &'a [AccountPool], name: &str) -> &'a AccountPool {
    pools
        .iter()
        .find(|pool| pool.name == name)
        .unwrap_or_else(|| panic!("no pool {name} in {pools:?}"))
}

#[test]
fn a_pool_holds_accounts_that_provide_its_standard_and_refuses_one_that_does_not() {
    let kept = with(
        "[host.pools.keys]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\", \"api-two\"]\n",
    );
    let catalogs = Catalogs::new(&kept);
    check(&kept, &catalogs).expect("both provide it");
    // A login only one harness takes does not provide the API standard: the refusal names the account, its kind's standards and
    // the pool's.
    let bad = with(
        "[host.pools.mixed]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\", \"login\"]\n",
    );
    let why = check(&bad, &Catalogs::new(&bad)).expect_err("a login is not an API key");
    assert!(
        why.contains("login")
            && why.contains("claude-login")
            && why.contains("claude-code")
            && why.contains("anthropic-api"),
        "{why}"
    );
    // An account of another provider that provides another standard is refused the same way.
    let other =
        with("[host.pools.crossed]\nstandard = \"anthropic-api\"\naccounts = [\"openai\"]\n");
    let why = check(&other, &Catalogs::new(&other)).expect_err("an OpenAI key");
    assert!(
        why.contains("openai") && why.contains("openai-api"),
        "{why}"
    );
}

#[test]
fn the_automatic_local_account_is_in_a_pool_of_the_standard_its_harness_login_provides() {
    let found =
        with("[host.pools.mine]\nstandard = \"claude-code\"\naccounts = [\"local\", \"login\"]\n");
    check(&found, &Catalogs::new(&found)).expect("the login of Claude Code is claude-code");
    let wrong = with("[host.pools.mine]\nstandard = \"anthropic-api\"\naccounts = [\"local\"]\n");
    let why = check(&wrong, &Catalogs::new(&wrong)).expect_err("no login is an API key");
    assert!(
        why.contains("local") && why.contains("harness-login"),
        "{why}"
    );
}

#[test]
fn the_host_makes_an_any_pool_of_each_standard_and_it_follows_the_accounts() {
    let two = with("");
    let pools = listing(&two, &Catalogs::new(&two));
    let any = pool_named(&pools, "any-anthropic-api");
    assert!(any.automatic);
    assert_eq!(any.standard, "anthropic-api");
    assert_eq!(names(any), ["api-one", "api-two"]);
    assert_eq!(
        names(pool_named(&pools, "any-claude-code")),
        ["local", "login"]
    );
    assert_eq!(names(pool_named(&pools, "any-openai-api")), ["openai"]);
    assert_eq!(names(pool_named(&pools, "any-codex")), ["local"]);
    // An account added is in it; one removed is not; nothing is written for either.
    let three = config(&format!(
        "{ACCOUNTS}[host.accounts.api-three]\nkind = \"anthropic-api-key\"\nid = \"id-three\"\n"
    ));
    let pools = listing(&three, &Catalogs::new(&three));
    assert_eq!(
        names(pool_named(&pools, "any-anthropic-api")),
        ["api-one", "api-two", "api-three"]
    );
    let one = config("[host.accounts.api-one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n");
    let pools = listing(&one, &Catalogs::new(&one));
    assert_eq!(names(pool_named(&pools, "any-anthropic-api")), ["api-one"]);
    assert!(
        pools.iter().all(|pool| pool.name != "any-openai-api"),
        "no account provides it now: {pools:?}"
    );
}

/// The pools by name that a person is shown.
fn shown(config: &Config, catalogs: &Catalogs<'_>) -> Vec<String> {
    listed(config, catalogs)
        .into_iter()
        .map(|pool| pool.name)
        .collect()
}

#[test]
fn the_host_lists_its_any_pool_only_when_two_accounts_hold_it_and_still_resolves_one_that_holds_one()
 {
    // No account but each harness's own login: every pool the host makes holds one account, so none is listed.
    let none = config("");
    let catalogs = Catalogs::new(&none);
    assert!(
        listing(&none, &catalogs)
            .iter()
            .any(|pool| pool.name == "any-pi" && names(pool) == ["local"]),
        "Pi's own login makes a pool of one"
    );
    assert!(
        shown(&none, &catalogs).is_empty(),
        "{:?}",
        shown(&none, &catalogs)
    );
    // A session or canoa.cfg that names it still gets it, and the entry still offers it for a session to choose.
    let any_pi = ["any-pi".to_owned()];
    assert_eq!(
        expand(&none, &catalogs, "pi", &any_pi).expect("resolves"),
        ["local"]
    );
    let offered = for_entry(&none, &catalogs, "pi", &any_pi).expect("pools");
    assert!(
        offered
            .iter()
            .any(|pool| pool.name == "any-pi" && pool.members.len() == 1),
        "{offered:?}"
    );
    // A Pi sign-in account makes it a pool of two, and it is listed with both.
    let two = config("[host.accounts.pi-one]\nkind = \"pi-oauth\"\nid = \"id-pi\"\n");
    let catalogs = Catalogs::new(&two);
    assert_eq!(shown(&two, &catalogs), ["any-pi"]);
    assert_eq!(
        names(pool_named(&listed(&two, &catalogs), "any-pi")),
        ["local", "pi-one"]
    );
    // The person's own pool is listed however many accounts it holds.
    let own = config(
        "[host.accounts.api-one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
         [host.pools.mine]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\"]\n",
    );
    assert_eq!(shown(&own, &Catalogs::new(&own)), ["mine"]);
}

#[test]
fn a_pi_provider_key_is_in_the_pool_of_the_api_its_variable_names_beside_an_api_key_account() {
    let both = config(
        "[host.accounts.api-one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
         [host.accounts.pi-key]\nkind = \"pi-api-key\"\nid = \"id-key\"\nsettings = { variable = \"ANTHROPIC_API_KEY\" }\n\
         [host.accounts.pi-other]\nkind = \"pi-api-key\"\nid = \"id-other\"\nsettings = { variable = \"MISTRAL_API_KEY\" }\n\
         [host.pools.mixed]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\", \"pi-key\"]\n",
    );
    // Pi lists the kind as its harness applies it; the kind, not Canoa, says the variable picks the standard.
    let pi = canoa_pi::acp::accounts::catalog();
    let catalogs = Catalogs::with_known(&both, "pi", (&pi, &pi), (Some("pi_accounts"), true));
    check(&both, &catalogs).expect("the key of the Anthropic variable provides the Anthropic API");
    let pools = listed(&both, &catalogs);
    assert_eq!(
        names(pool_named(&pools, "any-anthropic-api")),
        ["api-one", "pi-key"]
    );
    assert!(
        pools
            .iter()
            .all(|pool| !pool.accounts.contains(&"pi-other".to_owned())),
        "a key of a variable no API is named for provides none: {pools:?}"
    );
    // Claude Code runs it as an Anthropic key and so is offered both.
    assert_eq!(
        expand(
            &both,
            &catalogs,
            "claude",
            &["any-anthropic-api".to_owned()]
        )
        .expect("resolves"),
        ["api-one", "pi-key"]
    );
    let other = config(
        "[host.accounts.pi-other]\nkind = \"pi-api-key\"\nid = \"id-other\"\nsettings = { variable = \"MISTRAL_API_KEY\" }\n\
         [host.pools.mixed]\nstandard = \"anthropic-api\"\naccounts = [\"pi-other\"]\n",
    );
    let why = check(
        &other,
        &Catalogs::with_known(&other, "pi", (&pi, &pi), (Some("pi_accounts"), true)),
    )
    .expect_err("provides none");
    assert!(why.contains("pi-other") && why.contains("none"), "{why}");
}

#[test]
fn an_account_in_the_pools_of_two_standards_is_one_account() {
    // Foundry Local's kind provides both wire formats, so the one account is in both of the host's pools.
    let both = config(
        "[host.accounts.chat]\nkind = \"foundry-local\"\nid = \"id-chat\"\n\
         [host.pools.mine]\nstandard = \"openai-api\"\naccounts = [\"chat\"]\n\
         [host.pools.yours]\nstandard = \"anthropic-api\"\naccounts = [\"chat\"]\n",
    );
    let catalogs = Catalogs::new(&both);
    check(&both, &catalogs).expect("it provides both");
    let pools = listing(&both, &catalogs);
    assert_eq!(names(pool_named(&pools, "any-openai-api")), ["chat"]);
    assert_eq!(names(pool_named(&pools, "any-anthropic-api")), ["chat"]);
    assert_eq!(
        both.accounts.len(),
        1,
        "a pool holds names, never a copy of the account"
    );
}

#[test]
fn an_entry_is_offered_the_pools_its_harness_accepts_and_names_the_ones_it_lists() {
    let config = with(
        "[host.pools.keys]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\", \"api-two\"]\n",
    );
    let catalogs = Catalogs::new(&config);
    // An entry that lists no account offers the harness's own login; the host's pools hold the rest for a key trusted with
    // accounts.
    let plain = for_entry(&config, &catalogs, "claude", &[]).expect("pools");
    let by_name =
        |pools: &[EntryPool], name: &str| pools.iter().find(|pool| pool.name == name).cloned();
    // A key that reaches the agent in its environment has to be listed by the entry, so a pool of them offers none until the
    // entry names it, whoever asks.
    let keys = by_name(&plain, "keys").expect("a pool its harness accepts");
    assert!(!keys.listed && keys.members.is_empty(), "{keys:?}");
    let any = by_name(&plain, "any-claude-code").expect("the login's pool");
    assert_eq!(
        any.usable(false)
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>(),
        ["local"]
    );
    assert_eq!(
        any.members
            .iter()
            .map(|member| (member.name.as_str(), member.listed))
            .collect::<Vec<_>>(),
        [("local", true), ("login", false)]
    );
    // Claude Code takes no OpenAI key, so the pool of those is no choice there.
    assert!(by_name(&plain, "any-openai-api").is_none(), "{plain:?}");
    // The entry that names the pool offers its accounts to everyone.
    let named = for_entry(&config, &catalogs, "claude", &["keys".to_owned()]).expect("pools");
    let keys = by_name(&named, "keys").expect("named");
    assert!(keys.listed && keys.usable(false).count() == 2, "{keys:?}");
    assert!(
        by_name(&named, "any-anthropic-api")
            .expect("the host's")
            .members
            .iter()
            .all(|member| member.listed)
    );
    // Codex takes the OpenAI key's pool and not the other.
    let codex = for_entry(&config, &catalogs, "codex", &[]).expect("pools");
    assert!(
        by_name(&codex, "any-openai-api").is_some() && by_name(&codex, "keys").is_none(),
        "{codex:?}"
    );
}

#[test]
fn naming_a_pool_a_harness_cannot_take_is_refused_naming_both_standards() {
    let config =
        with("[host.pools.keys]\nstandard = \"anthropic-api\"\naccounts = [\"api-one\"]\n");
    let catalogs = Catalogs::new(&config);
    let why = expand(&config, &catalogs, "codex", &["keys".to_owned()])
        .expect_err("Codex takes no Anthropic key");
    assert!(
        why.contains("anthropic-api") && why.contains("codex"),
        "{why}"
    );
    let why = for_entry(
        &config,
        &catalogs,
        "codex",
        &["any-anthropic-api".to_owned()],
    )
    .expect_err("the host's own pool");
    assert!(why.contains("anthropic-api"), "{why}");
    // The accounts of a pool an entry names take the pool's place in what the entry offers, once each, in order.
    let offered = expand(
        &config,
        &catalogs,
        "claude",
        &["login".to_owned(), "keys".to_owned(), "api-one".to_owned()],
    )
    .expect("a pool its harness accepts");
    assert_eq!(offered, ["login", "api-one"]);
}
