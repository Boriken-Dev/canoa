//! `secrets/*` against the control channel: who may ask, what the list holds (never a value), the value on request, forgetting, and a
//! secret typed into a form sealed to the key that asked.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::atomic::AtomicBool;

use coqui_acp::secret::{
    DEFAULT_LIFETIME, LinkKey, LinkRole, Method as WireMethod, Prover, RequestParams, RequestReply,
    Verifier,
};
use coqui_agentic_peer::auth::{Identity, Method};

use super::*;
use crate::delegate::BridgeCommand;
use crate::secrets::Hub;

/// A made-up value no real secret looks like, so a leak is found by its text.
const VALUE: &str = "zq9-not-a-real-secret-7f3a";

struct Rig {
    control: Control,
    hub: Arc<Hub>,
    dir: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn rig(name: &str) -> Rig {
    rig_in(name, false).0
}

/// A rig, and where its canoa.cfg is when it has one for the vault to read `[secrets]` from and `secrets/configure` to write.
fn rig_in(name: &str, with_config: bool) -> (Rig, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "canoa-secrets-control-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let store = Arc::new(TrustStore::open(&dir).expect("a trust store"));
    let host = Identity::generate(Method::WebCrypto).expect("a key");
    let hub = Hub::new(
        &host,
        &store,
        BridgeCommand::new(std::path::Path::new("canoa"), &[]),
    );
    let config = dir.join("canoa.cfg");
    let mut control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_sealer(Sealer::new(Arc::clone(&hub), Duration::from_secs(20)));
    if with_config {
        std::fs::write(&config, "").expect("a canoa.cfg");
        control = control.with_files(Files::new(
            None,
            Vec::new(),
            Some(config.clone()),
            None,
            UploadLimits::default(),
        ));
        hub.vault().configure(
            crate::secrets::catalog::Catalog::built_in_in(&dir),
            crate::secrets::settings::source_of(Some(config.clone())),
        );
    }
    (Rig { control, hub, dir }, config)
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    let answer = control.handle(caller, &line).expect("answered");
    serde_json::from_slice(&answer).expect("JSON")
}

fn peer(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

#[test]
fn a_key_with_no_secrets_grant_reaches_none_of_the_methods_and_the_owner_all_of_them() {
    let rig = rig("grant");
    let denied = peer(&[Grant::ConfigRead, Grant::Accounts, Grant::MailSetup]);
    for method in [
        "secrets/providers",
        "secrets/list",
        "secrets/reveal",
        "secrets/forget",
        "secrets/configure",
    ] {
        let answer = ask(
            &rig.control,
            &denied,
            method,
            json!({"provider": "form", "name": "x"}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        let allowed = ask(
            &rig.control,
            &Caller::Owner,
            method,
            json!({"provider": "form", "name": "x"}),
        );
        assert_ne!(allowed["error"]["code"], DENIED, "{method}: {allowed}");
    }
    // It is not among what a key is given unless canoa.cfg names it.
    assert!(!crate::config::NativeConfig::default_control().contains(&Grant::Secrets));
    assert_eq!(Grant::named("secrets"), Some(Grant::Secrets));
    let granted = ask(
        &rig.control,
        &peer(&[Grant::Secrets]),
        "host/grants",
        json!({}),
    );
    assert_eq!(granted["result"]["grants"], json!(["secrets"]));
}

#[test]
fn a_host_that_cannot_seal_a_typed_answer_keeps_no_secrets() {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let answer = ask(&control, &Caller::Owner, "secrets/list", json!({}));
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("keeps no secrets")),
        "{answer}"
    );
}

#[test]
fn the_list_names_where_each_secret_is_kept_and_what_it_is_called_and_never_what_it_is() {
    let rig = rig("list");
    let vault = rig.hub.vault();
    vault
        .remember(None, "api_key", VALUE.as_bytes(), "close")
        .expect("kept");
    vault
        .remember(Some("s1"), "mine", VALUE.as_bytes(), "15m")
        .expect("kept");
    let listed = ask(&rig.control, &Caller::Owner, "secrets/list", json!({}));
    let rows = listed["result"]["secrets"].as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{listed}");
    let api = rows
        .iter()
        .find(|row| row["name"] == "api_key")
        .expect("api_key");
    assert_eq!(
        (api["provider"].as_str(), api["kept"].as_str()),
        (Some("form"), Some("process"))
    );
    assert!(api["expires_in"].is_null());
    let mine = rows.iter().find(|row| row["name"] == "mine").expect("mine");
    assert_eq!(mine["expires_in"], 900);
    assert!(
        !listed.to_string().contains("zq9"),
        "a value is in the list: {listed}"
    );
    // The ways to keep what is typed come with it, for the app's own form.
    assert_eq!(listed["result"]["keeps"][0]["id"], "session");
    let providers = ask(&rig.control, &Caller::Owner, "secrets/providers", json!({}));
    let ids: Vec<&str> = providers["result"]["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .filter_map(|provider| provider["id"].as_str())
        .collect();
    assert_eq!(ids, ["form", "system"]);
    assert_eq!(providers["result"]["entries"][0]["name"], "form");
    assert!(!providers.to_string().contains("zq9"));
}

#[test]
fn a_secret_is_shown_when_asked_for_by_name_and_forgotten_when_told_to() {
    let rig = rig("reveal");
    rig.hub
        .vault()
        .remember(None, "api_key", VALUE.as_bytes(), "close")
        .expect("kept");
    let shown = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/reveal",
        json!({"provider": "form", "name": "api_key"}),
    );
    assert_eq!(shown["result"]["value"], VALUE);
    // A name another provider holds is not this provider's.
    let other = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/reveal",
        json!({"provider": "system", "name": "api_key"}),
    );
    assert_eq!(other["error"]["code"], NOT_FOUND, "{other}");
    let forgot = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/forget",
        json!({"provider": "form", "name": "api_key"}),
    );
    assert_eq!(forgot["result"], json!({"forgot": true}));
    let after = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/reveal",
        json!({"provider": "form", "name": "api_key"}),
    );
    assert_eq!(after["error"]["code"], NOT_FOUND, "{after}");
    assert!(
        ask(&rig.control, &Caller::Owner, "secrets/list", json!({}))["result"]["secrets"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    for params in [
        json!({}),
        json!({"provider": "form"}),
        json!({"provider": "form", "name": "a\nb"}),
    ] {
        let bad = ask(&rig.control, &Caller::Owner, "secrets/reveal", params);
        assert_eq!(bad["error"]["code"], BAD_PARAMS, "{bad}");
    }
}

/// The person's app answering the form the host put on `client`: sealed under the link's secret, with `value` typed and `keep` chosen.
fn app_answers(hub: &Hub, request: &Value, plain: &str, value: Option<&str>) -> Value {
    let key = LinkKey::derive(&*hub.link_secret().expect("a link secret")).expect("a key");
    let verifier = Verifier::builder(key.identity(LinkRole::App))
        .link(&key, LinkRole::App)
        .expect("the app's side")
        .build();
    let prover = Prover::linked(&key, LinkRole::App);
    let params = RequestParams::from_value(&request["params"]).expect("a request of the extension");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    let collector = verifier
        .collector(
            &key.identity(LinkRole::Host),
            WireMethod::Request,
            &params.context,
            &params.recipient,
            now,
        )
        .expect("a collector");
    let mut sealed = std::collections::BTreeMap::new();
    if let Some(value) = value {
        sealed.insert(
            "value".to_owned(),
            collector.seal("value", value).expect("sealed"),
        );
    }
    let auth = collector
        .authenticate(&prover, plain, &sealed, now, DEFAULT_LIFETIME)
        .expect("bound to the request");
    let reply = RequestReply::Accept {
        plain: plain.to_owned(),
        sealed,
        auth,
    };
    json!({"jsonrpc": "2.0", "id": request["id"], "result": reply.to_value()})
}

/// Runs `secrets/set` on a link of its own, and answers its form with `answer`; the line the host ends with, and every line it sent first.
fn set_through(
    rig: &Rig,
    params: &Value,
    answer: impl FnOnce(&Value) -> Option<Value>,
) -> (Value, Vec<String>) {
    let (client, host) = crate::local::pair(1 << 20, Arc::new(AtomicBool::new(false)));
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": "secrets/set", "params": params});
    let asker = rig.hub.asker_of(None);
    let vault = rig.hub.vault();
    let mut sent = Vec::new();
    let ended = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            secrets::set(
                vault,
                asker.as_ref(),
                Duration::from_secs(20),
                &request,
                &host,
                &|_| None,
            )
        });
        let line = client.receive(Some(Duration::from_secs(10)));
        if let Ok(Some(line)) = line {
            sent.push(String::from_utf8_lossy(&line).into_owned());
            let asked: Value = serde_json::from_slice(&line).expect("JSON");
            if let Some(reply) = answer(&asked) {
                client.send(reply.to_string().as_bytes()).expect("sent");
            } else {
                client.close();
            }
        }
        worker.join().expect("the call ended")
    });
    (serde_json::from_slice(&ended).expect("JSON"), sent)
}

#[test]
fn a_secret_set_is_typed_into_a_form_sealed_to_the_key_that_asked_and_kept_as_chosen() {
    let rig = rig("set");
    let (ended, sent) = set_through(
        &rig,
        &json!({"provider": "form", "name": "deploy_token"}),
        |asked| {
            assert_eq!(asked["method"], "_coqui_acp/secret_request");
            let schema = asked["params"]["schema"].as_str().expect("a schema");
            assert!(
                schema.contains("\"value\"") && schema.contains("Until Canoa closes"),
                "{schema}"
            );
            assert!(
                asked["params"]["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("deploy_token")),
                "{asked}"
            );
            Some(app_answers(
                &rig.hub,
                asked,
                r#"{"keep":"close"}"#,
                Some(VALUE),
            ))
        },
    );
    assert_eq!(ended["result"], json!({"set": true}), "{ended}");
    assert_eq!(ended["id"], 7);
    // The host sent nothing that holds the value, and the value is in the vault as chosen.
    assert!(!sent.join("\n").contains(VALUE));
    assert!(!ended.to_string().contains(VALUE));
    let shown = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/reveal",
        json!({"provider": "form", "name": "deploy_token"}),
    );
    assert_eq!(shown["result"]["value"], VALUE);
    let row = &ask(&rig.control, &Caller::Owner, "secrets/list", json!({}))["result"]["secrets"][0];
    assert_eq!(
        (row["kept"].as_str(), row["name"].as_str()),
        (Some("process"), Some("deploy_token"))
    );
}

#[test]
fn a_form_declined_a_link_that_ends_or_a_name_that_is_none_sets_nothing() {
    let rig = rig("set_refused");
    let params = json!({"provider": "form", "name": "deploy_token"});
    let (declined, _) = set_through(&rig, &params, |asked| {
        Some(json!({"jsonrpc": "2.0", "id": asked["id"], "result": {"action": "decline"}}))
    });
    assert_eq!(declined["error"]["code"], REFUSED, "{declined}");
    let (silent, _) = set_through(&rig, &params, |_| None);
    assert_eq!(silent["error"]["code"], REFUSED, "{silent}");
    let (unknown, sent) = set_through(&rig, &json!({"provider": "ghost", "name": "x"}), |_| None);
    assert_eq!(unknown["error"]["code"], NOT_FOUND, "{unknown}");
    assert!(
        sent.is_empty(),
        "nothing was asked for a provider this host has not set up"
    );
    let (nameless, _) = set_through(&rig, &json!({"provider": "form"}), |_| None);
    assert_eq!(nameless["error"]["code"], BAD_PARAMS, "{nameless}");
    assert!(rig.hub.vault().rows().is_empty());
}

#[test]
fn a_set_with_nothing_to_seal_the_answer_with_is_refused_before_anything_is_asked() {
    let rig = rig("no_asker");
    let (client, host) = crate::local::pair(1 << 20, Arc::new(AtomicBool::new(false)));
    let request = json!({"jsonrpc": "2.0", "id": 3, "method": "secrets/set", "params": {"provider": "form", "name": "x"}});
    let ended = secrets::set(
        rig.hub.vault(),
        None,
        Duration::from_secs(1),
        &request,
        &host,
        &|_| None,
    );
    let ended: Value = serde_json::from_slice(&ended).expect("JSON");
    assert_eq!(ended["error"]["code"], REFUSED, "{ended}");
    assert!(matches!(
        client.receive(Some(Duration::from_millis(50))),
        Err(_) | Ok(None)
    ));
}

#[test]
fn a_provider_entry_is_set_up_with_plain_settings_and_a_secret_taken_from_another_provider() {
    let (rig, config) = rig_in("configure", true);
    let set = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "provider": "bitwarden", "keep": "close",
            "settings": {"command": "bw", "part": "password"},
            "from": {"session": {"entry": "form", "name": "bw_session"}}}),
    );
    assert_eq!(set["result"]["saved"], true, "{set}");
    let text = std::fs::read_to_string(&config).expect("canoa.cfg");
    assert!(text.contains("[secrets.providers.work]"), "{text}");
    assert!(text.contains("provider = \"bitwarden\""), "{text}");
    assert!(text.contains("name = \"bw_session\""), "{text}");
    let listed = ask(&rig.control, &Caller::Owner, "secrets/providers", json!({}));
    let entries = listed["result"]["entries"].as_array().expect("entries");
    let work = entries
        .iter()
        .find(|entry| entry["name"] == "work")
        .expect("the entry is listed");
    assert_eq!(work["provider"], "bitwarden");
    assert_eq!(work["settings"]["command"], "bw");
    assert_eq!(
        work["from"]["session"],
        json!({"entry": "form", "name": "bw_session"})
    );
}

#[test]
fn a_secret_is_never_written_to_canoa_cfg_and_only_a_setting_that_may_be_taken_from_a_secret_is() {
    let (rig, config) = rig_in("configure_refused", true);
    let typed = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "provider": "bitwarden",
            "settings": {"session": "typed-key-zq9-made-up"}}),
    );
    let why = typed["error"]["message"].as_str().expect("refused");
    assert!(why.contains("is a secret"), "{why}");
    assert!(
        !why.contains("typed-key-zq9"),
        "the refusal repeats the value"
    );
    let plain = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "provider": "bitwarden",
            "from": {"part": {"entry": "form", "name": "which"}}}),
    );
    assert!(
        plain["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("does not take part")),
        "{plain}"
    );
    let ghost = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "provider": "ghost"}),
    );
    assert!(ghost["error"].is_object(), "{ghost}");
    let unknown = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "provider": "bitwarden", "settings": {"surprise": 1}}),
    );
    assert!(unknown["error"].is_object(), "{unknown}");
    let after = std::fs::read_to_string(&config).expect("canoa.cfg");
    assert!(
        !after.contains("secrets") && !after.contains("typed-key"),
        "nothing was written: {after}"
    );
}

#[test]
fn an_entry_is_removed_unless_a_name_or_the_default_still_points_at_it() {
    let (rig, config) = rig_in("configure_remove", true);
    for (name, entry) in [("work", "work"), ("other", "other")] {
        let set = ask(
            &rig.control,
            &Caller::Owner,
            "secrets/configure",
            json!({"name": entry, "provider": "bitwarden"}),
        );
        assert_eq!(set["result"]["saved"], true, "{name}: {set}");
    }
    std::fs::write(
        &config,
        format!(
            "{}\n[secrets.names]\ngithub = \"work\"\n",
            std::fs::read_to_string(&config).expect("canoa.cfg")
        ),
    )
    .expect("a name points at work");
    let kept = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "work", "remove": true}),
    );
    assert!(kept["error"].is_object(), "{kept}");
    let removed = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "other", "remove": true}),
    );
    assert_eq!(removed["result"]["removed"], true, "{removed}");
    let missing = ask(
        &rig.control,
        &Caller::Owner,
        "secrets/configure",
        json!({"name": "other", "remove": true}),
    );
    assert!(missing["error"].is_object(), "{missing}");
}
