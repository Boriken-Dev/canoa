use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use std::ffi::OsString;
use std::sync::Mutex;

use coqui_peer_webcrypto::TrustStore;

use super::accounts::HARNESS_MAX_REQUESTS;
use super::admission::{page_grants, reaches_control, read_admitted, write_admitted};
use super::bind::provider_args;
use super::{Agents, harness_settings, keeps_key_in_environment, private_starter};
use crate::config::{Config, EntryKind};
use crate::{control, delegate, kind};

#[test]
fn the_admitted_keys_that_are_kept_are_those_the_store_still_accepts_and_the_secret_asked_for_since_is_respected()
 {
    use coqui_peer_webcrypto::KeyMaterial;
    let dir = std::env::temp_dir().join(format!("canoa-admitted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("trust")).expect("a folder");
    let store = TrustStore::open(&dir.join("trust")).expect("a store");
    let key = |tag: u8| KeyMaterial::new("noise", "PUBLIC KEY", vec![tag; 32]).expect("a key");
    let (kept, taken_away) = (key(1), key(2));
    store.accept_key(&kept).expect("accepted");
    let file = dir.join("office").join("admitted.json");
    let keys = vec![
        kept.id().to_string(),
        taken_away.id().to_string(),
        "not an id".to_owned(),
    ];

    assert!(
        read_admitted(&file, false, &store).is_empty(),
        "no file holds none"
    );
    write_admitted(&file, false, &keys).expect("written");
    assert_eq!(
        read_admitted(&file, false, &store),
        BTreeSet::from([kept.id()]),
        "a key the trust store no longer accepts, and a line that is no key, are not held again"
    );
    assert!(
        read_admitted(&file, true, &store).is_empty(),
        "keys admitted when no secret was asked for are not held once one is"
    );
    write_admitted(&file, true, &keys).expect("written");
    assert_eq!(
        read_admitted(&file, true, &store),
        BTreeSet::from([kept.id()])
    );
    assert_eq!(
        read_admitted(&file, false, &store),
        BTreeSet::from([kept.id()]),
        "a secret asked for then is more than is asked for now"
    );
    std::fs::write(&file, "{ not json").expect("written");
    assert!(
        read_admitted(&file, false, &store).is_empty(),
        "a file that cannot be read holds none"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_admitted_page_is_given_the_services_only_when_one_has_something_to_give() {
    let grants = |text: &str| {
        let config = Config::from_str_in(text, std::path::Path::new("/canoa")).expect("parses");
        let services =
            crate::services::Services::new(crate::services::standin::TestHost::over(text, &[]));
        let given = page_grants(&config, &services);
        let offered = services.extras().iter().any(|extra| extra.offered(&given));
        (given, offered)
    };
    let account = "[host.services.mail.accounts.desktop]\nbackend = \"outlook\"\n";
    let (given, offered) = grants(account);
    assert!(given.contains(&control::Grant::Services) && offered);
    assert!(
        config_default().is_subset(&given),
        "what a browser key holds, it still holds"
    );
    let (given, offered) = grants("");
    assert!(!given.contains(&control::Grant::Services) && !offered);
    let (given, offered) = grants(&format!("[host.services.mail]\ntools = false\n{account}"));
    assert!(!given.contains(&control::Grant::Services) && !offered);
    let (given, _) = grants(&format!(
        "[host.native]\ncontrol = [\"ledger.read\"]\n{account}"
    ));
    assert!(given.contains(&control::Grant::Services));
}

fn config_default() -> BTreeSet<control::Grant> {
    crate::config::NativeConfig::default_control()
}

#[test]
fn a_tasks_connection_cannot_open_the_control_channel_and_this_process_can() {
    let task = delegate::ChildContext {
        principal: delegate::Principal::Owner,
        roots: Vec::new(),
        depth: 1,
        parent_session: None,
    };
    assert!(reaches_control(control::NAME, None));
    assert!(!reaches_control(control::NAME, Some(&task)));
    assert!(!reaches_control("Claude", None));
}

#[test]
fn a_built_in_harness_keeps_the_request_limit_the_host_always_had_unless_the_entry_sets_one() {
    let config = Config::from_str_in(
        "[host.agents.a]\nharness = \"claude\"\n[host.agents.b]\nharness = \"codex\"\nsettings = { max_requests = 8 }\n",
        std::path::Path::new("/canoa"),
    )
    .expect("parses");
    let limit = |name: &str| {
        let entry = config.entry(name).expect("there");
        let EntryKind::Harness {
            harness,
            executable,
            options,
            settings,
            ..
        } = &entry.kind
        else {
            panic!("a harness");
        };
        harness_settings(
            &crate::kind::registry(None).expect("bundled"),
            entry,
            harness,
            executable.as_ref(),
            options,
            settings.as_ref(),
        )
        .as_map()["max_requests"]
            .clone()
    };
    assert_eq!(limit("a"), serde_json::json!(HARNESS_MAX_REQUESTS));
    assert_eq!(limit("b"), serde_json::json!(8));
}

#[test]
fn opencodes_settings_are_its_program_and_its_limits_and_none_of_the_access_keys() {
    let config = Config::from_str_in(
        "[host.agents.o]
harness = \"opencode\"
executable = \"opencode\"
settings = { replay_max_bytes = 4096 }
",
        std::path::Path::new("/canoa"),
    )
    .expect("parses");
    let entry = config.entry("o").expect("there");
    let EntryKind::Harness {
        harness,
        executable,
        options,
        settings,
        ..
    } = &entry.kind
    else {
        panic!("a harness");
    };
    let map = harness_settings(
        &crate::kind::registry(None).expect("bundled"),
        entry,
        harness,
        executable.as_ref(),
        options,
        settings.as_ref(),
    );
    let map = map.as_map();
    assert_eq!(map["executable"], serde_json::json!("opencode"));
    assert_eq!(map["max_requests"], serde_json::json!(HARNESS_MAX_REQUESTS));
    assert_eq!(map["replay_max_bytes"], serde_json::json!(4096));
    for key in ["access", "allow_client_access", "allow_bypass", "modes"] {
        assert!(!map.contains_key(key), "{key} reached the stdio provider");
    }
}

#[test]
fn a_harness_whose_provider_declares_the_access_keys_is_given_them() {
    let config = Config::from_str_in(
        "[host.agents.c]\nharness = \"claude\"\n",
        std::path::Path::new("/canoa"),
    )
    .expect("parses");
    let entry = config.entry("c").expect("there");
    let EntryKind::Harness {
        harness,
        executable,
        options,
        settings,
        ..
    } = &entry.kind
    else {
        panic!("a harness");
    };
    let map = harness_settings(
        &crate::kind::registry(None).expect("bundled"),
        entry,
        harness,
        executable.as_ref(),
        options,
        settings.as_ref(),
    );
    let map = map.as_map();
    for key in ["access", "allow_client_access", "allow_bypass", "modes"] {
        assert!(map.contains_key(key), "{key} did not reach the provider");
    }
}

#[test]
fn a_setting_is_declared_by_the_provider_that_takes_it_and_by_no_other() {
    assert!(crate::vendors::declares(None, "claude", "setting_sources"));
    assert!(!crate::vendors::declares(None, "codex", "setting_sources"));
    assert!(!crate::vendors::declares(None, "opencode", "modes"));
    assert!(!crate::vendors::declares(None, "nobody", "modes"));
}

#[test]
fn stopping_the_host_ends_an_agent_started_for_a_reading_and_refuses_the_next() {
    let agents = Arc::new(Mutex::new(Agents::default()));
    let starter = private_starter(&agents, crate::config::DEFAULT_MAX_MESSAGE_BYTES);
    let (program, args): (OsString, Vec<OsString>) = if cfg!(windows) {
        (
            "ping".into(),
            vec!["-n".into(), "60".into(), "127.0.0.1".into()],
        )
    } else {
        ("sleep".into(), vec!["60".into()])
    };
    let (agent, _forget) = (starter.0)(&program, &args)
        .expect("it starts")
        .expect("the host is not stopping");
    assert_eq!(agents.lock().expect("lock").running.len(), 1);
    assert!(
        agent.try_status().expect("status").is_none(),
        "it is running"
    );
    super::end_all(&agents);
    let until = std::time::Instant::now() + core::time::Duration::from_secs(10);
    while agent.try_status().expect("status").is_none() && std::time::Instant::now() < until {
        std::thread::sleep(core::time::Duration::from_millis(50));
    }
    assert!(
        agent.try_status().expect("status").is_some(),
        "stopping the host ended it"
    );
    assert!(
        (starter.0)(&program, &args).expect("no error").is_none(),
        "none starts while the host stops"
    );
}

#[test]
fn a_provider_command_line_holds_no_setting() {
    let config = Config::from_str_in(
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n[host.providers.p.source_settings]\npaths = [\"SECRET-1\"]\n[host.providers.p.settings]\nkey = \"SECRET-2\"\n",
        std::path::Path::new("/canoa"),
    )
    .expect("parses");
    let entry = &config.entries[0];
    let EntryKind::Provider {
        origin,
        provider,
        directory,
        max_sessions,
        ..
    } = &entry.kind
    else {
        panic!("a provider");
    };
    let args = provider_args(&[], entry, origin, provider, directory, *max_sessions);
    assert!(
        args.iter().any(|arg| arg == "--settings-on-stdin"),
        "{args:?}"
    );
    assert!(
        !args
            .iter()
            .any(|arg| arg.to_string_lossy().contains("SECRET")),
        "{args:?}"
    );
}

/// An offered account of the kind `k`, which keeps nothing in a directory.
fn offered(provides: &[&str], endpoint: bool) -> kind::Offered {
    let mut info = coqui_acp_api::AccountKindInfo::new("k", "K").provides(provides.iter().copied());
    if endpoint {
        info.endpoints =
            vec![coqui_acp_api::EndpointSlot::new(coqui_acp_api::MODEL_API).key("base_url")];
    }
    let account = crate::config::AccountConfig {
        owner: None,
        kind: "k".to_owned(),
        id: "id".to_owned(),
        settings: None,
        route: None,
    };
    ("a".to_owned(), account, info)
}

#[test]
fn a_key_another_provider_made_is_a_key_in_the_environment_whatever_its_kind_says_of_an_address() {
    // Its own provider's key says it by its address; an API standard's account made elsewhere says it by what it provides.
    let mut lists_k = coqui_acp_api::AccountCatalog::new();
    lists_k
        .kinds
        .push(coqui_acp_api::AccountKindInfo::new("k", "K"));
    let lists_nothing = coqui_acp_api::AccountCatalog::new();
    assert!(keeps_key_in_environment(
        &crate::makers::Makers::default(),
        &[offered(&["anthropic-api"], true)],
        ("claude", Some(&lists_k))
    ));
    assert!(!keeps_key_in_environment(
        &crate::makers::Makers::default(),
        &[offered(&["claude-code"], false)],
        ("claude", Some(&lists_k))
    ));
    assert!(keeps_key_in_environment(
        &crate::makers::Makers::default(),
        &[offered(&["anthropic-api"], false)],
        ("pi", Some(&lists_nothing))
    ));
    assert!(!keeps_key_in_environment(
        &crate::makers::Makers::default(),
        &[offered(&["pi"], false)],
        ("claude", Some(&lists_k))
    ));
}
