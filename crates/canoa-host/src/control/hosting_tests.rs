use std::path::PathBuf;

use super::files::{Files, UploadLimits};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-hosting-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn editing(text: &str) -> (PathBuf, Control) {
    let tmp = dir("config");
    let path = tmp.join("canoa.cfg");
    fs::write(&path, text).expect("canoa.cfg");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        Some(tmp),
        Vec::new(),
        Some(path.clone()),
        None,
        UploadLimits::default(),
    ));
    (path, control)
}

#[test]
fn only_the_hosting_grant_reaches_keys_pairing_and_the_host_table() {
    let (_, control) = editing("[host]\n");
    // Every other grant together does not open it.
    let others: Vec<Grant> = Grant::all()
        .into_iter()
        .filter(|grant| *grant != Grant::Hosting)
        .collect();
    let without = key(&others);
    for (method, params) in [
        ("host/keys", json!({})),
        ("host/accept", json!({"id": "x"})),
        ("host/revoke", json!({"id": "x"})),
        ("config/get", json!({"section": "host"})),
        ("config/set", json!({"section": "host", "value": {}})),
        ("config/check", json!({"section": "host", "value": {}})),
    ] {
        assert_eq!(
            ask(&control, &without, method, params)["error"]["code"],
            DENIED,
            "{method}"
        );
    }
    assert_eq!(Grant::named("hosting"), Some(Grant::Hosting));
    // The preference sections stay with config.read and config.write.
    let hosting = key(&[Grant::Hosting]);
    assert_eq!(
        ask(
            &control,
            &hosting,
            "config/get",
            json!({"section": "sessions"})
        )["error"]["code"],
        DENIED
    );
    // A host with no trust store to manage says so, not that the key is wrong.
    let answer = ask(&control, &hosting, "host/keys", json!({}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn the_host_table_is_read_checked_and_written_in_the_hosts_own_words() {
    let (path, control) = editing(
        "[desktop]\nstart_hidden = true\n\n[host.agents.echo]\nprogram = \"echo\"\nlisten = \"127.0.0.1:0\"\n",
    );
    let who = key(&[Grant::Hosting]);
    let read = ask(&control, &who, "config/get", json!({"section": "host"}));
    assert_eq!(read["result"]["value"]["agents"]["echo"]["program"], "echo");
    assert_eq!(read["result"]["restart"], true);
    let before = fs::read_to_string(&path).expect("canoa.cfg");

    // The host's own refusal comes back, and the file is as it was.
    let wrong = json!({"agents": {"echo": {"listen": "127.0.0.1:0"}}});
    for method in ["config/check", "config/set"] {
        let refused = ask(
            &control,
            &who,
            method,
            json!({"section": "host", "value": wrong}),
        );
        assert_eq!(refused["error"]["code"], REFUSED, "{method}: {refused}");
        assert!(
            refused["error"]["message"]
                .as_str()
                .is_some_and(|words| words.contains("canoa.cfg")),
            "{refused}"
        );
        assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);
    }
    let unknown = ask(
        &control,
        &who,
        "config/check",
        json!({"section": "host", "value": {"surprise": 1}}),
    );
    assert_eq!(unknown["error"]["code"], REFUSED, "{unknown}");

    // A good table is checked without being written, then written, and the other tables stay.
    let good = json!({"origins": ["https://canoa.example"], "agents": {"echo": {"program": "echo", "listen": "127.0.0.1:0"}}});
    let checked = ask(
        &control,
        &who,
        "config/check",
        json!({"section": "host", "value": good}),
    );
    assert_eq!(checked["result"]["valid"], true, "{checked}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);
    let set = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "host", "value": good}),
    );
    assert_eq!(set["result"]["restart"], true, "{set}");
    let text = fs::read_to_string(&path).expect("canoa.cfg");
    assert!(text.contains("https://canoa.example"), "{text}");
    assert!(text.contains("start_hidden = true"), "{text}");
}

#[test]
fn config_check_answers_for_a_section_a_key_may_write_and_writes_nothing() {
    let (path, control) = editing("[desktop]\nstart_hidden = true\n");
    let before = fs::read_to_string(&path).expect("canoa.cfg");
    let writer = key(&[Grant::ConfigWrite]);
    let checked = ask(
        &control,
        &writer,
        "config/check",
        json!({"section": "sessions", "value": {"keep": 3}}),
    );
    assert_eq!(checked["result"]["valid"], true, "{checked}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);
    // A check is no way round the grant a write needs.
    let guarded = ask(
        &control,
        &writer,
        "config/check",
        json!({"section": "permissions", "value": {}}),
    );
    assert_eq!(guarded["error"]["code"], DENIED, "{guarded}");
    let reader = key(&[Grant::ConfigRead]);
    let denied = ask(
        &control,
        &reader,
        "config/check",
        json!({"section": "sessions", "value": {}}),
    );
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
}
