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

#[test]
fn an_extension_is_added_changed_and_removed_through_the_host_table() {
    let (path, control) =
        editing("[desktop]\nstart_hidden = true\n\n[host.agents.echo]\nprogram = \"echo\"\n");
    let who = key(&[Grant::Hosting]);
    let table = |extensions: Value| json!({"agents": {"echo": {"program": "echo"}}, "extensions": extensions});
    let set = |value: Value| {
        ask(
            &control,
            &who,
            "config/set",
            json!({"section": "host", "value": value}),
        )
    };

    let added = set(table(
        json!({"sample": {"library": "sample.dll", "provider": "sample_extension"}}),
    ));
    assert_eq!(added["result"]["restart"], true, "{added}");
    let read = ask(&control, &who, "config/get", json!({"section": "host"}));
    assert_eq!(
        read["result"]["value"]["extensions"]["sample"]["provider"],
        "sample_extension"
    );

    // An extension that names neither a library nor a source, or both, is the host's own refusal and writes nothing.
    let before = fs::read_to_string(&path).expect("canoa.cfg");
    let neither = set(table(json!({"sample": {"provider": "sample_extension"}})));
    assert_eq!(neither["error"]["code"], REFUSED, "{neither}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);

    // Another name and library replace the first, and the rest of canoa.cfg stays.
    let changed = set(table(
        json!({"renamed": {"library": "other.dll", "provider": "sample_extension"}}),
    ));
    assert_eq!(changed["result"]["restart"], true, "{changed}");
    let text = fs::read_to_string(&path).expect("canoa.cfg");
    assert!(
        text.contains("renamed") && !text.contains("sample.dll"),
        "{text}"
    );
    assert!(text.contains("start_hidden = true"), "{text}");

    let removed = set(json!({"agents": {"echo": {"program": "echo"}}}));
    assert_eq!(removed["result"]["restart"], true, "{removed}");
    assert!(
        !fs::read_to_string(&path)
            .expect("canoa.cfg")
            .contains("renamed")
    );

    // Without the grant that guards `[host]`, the table is not reachable at all.
    let reader = key(&[Grant::ConfigRead, Grant::ConfigWrite]);
    let denied = ask(
        &control,
        &reader,
        "config/set",
        json!({"section": "host", "value": {}}),
    );
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
}

#[test]
fn a_host_write_made_from_a_table_the_file_no_longer_holds_is_refused() {
    let (path, control) = editing("[host.agents.echo]\nprogram = \"echo\"\nlisten = \"127.0.0.1:0\"\n");
    let who = key(&[Grant::Hosting]);
    let read = ask(&control, &who, "config/get", json!({"section": "host"}));
    let digest = read["result"]["digest"].as_str().expect("a digest").to_owned();
    assert!(!digest.is_empty(), "{read}");

    // Another writer adds an agent after the page read the table.
    let meanwhile = fs::read_to_string(&path).expect("canoa.cfg")
        + "\n[host.agents.later]\nprogram = \"echo\"\nlisten = \"127.0.0.1:0\"\n";
    fs::write(&path, &meanwhile).expect("canoa.cfg");
    let mine = json!({"agents": {"echo": {"program": "echo", "listen": "127.0.0.1:0"}}, "origins": ["https://canoa.example"]});
    let refused = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "host", "value": mine, "digest": digest}),
    );
    assert_eq!(refused["error"]["code"], files::CHANGED, "{refused}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), meanwhile);

    // A check changes nothing and is not refused; read again, the same write goes through and names the new table.
    let reread = ask(&control, &who, "config/get", json!({"section": "host"}));
    let fresh = reread["result"]["digest"].as_str().expect("a digest").to_owned();
    assert_ne!(fresh, digest);
    let mut both = reread["result"]["value"].clone();
    both["origins"] = json!(["https://canoa.example"]);
    let set = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "host", "value": both, "digest": fresh}),
    );
    assert_eq!(set["result"]["restart"], true, "{set}");
    assert_ne!(set["result"]["digest"], json!(fresh), "{set}");
    let text = fs::read_to_string(&path).expect("canoa.cfg");
    assert!(text.contains("later") && text.contains("canoa.example"), "{text}");

    // Without a digest the write is the caller's whole word, as before.
    let plain = ask(
        &control,
        &who,
        "config/set",
        json!({"section": "host", "value": mine}),
    );
    assert_eq!(plain["result"]["restart"], true, "{plain}");
}

#[test]
fn the_app_the_host_runs_in_writes_the_tables_no_key_edits() {
    let (path, control) = editing("[desktop]\nstart_hidden = true\n\n[sessions]\nqueue_delivery = \"now\"\n");
    let owner = Caller::Owner;
    for section in ["agents", "desktop", "profile", "skill_pins"] {
        let key = key(&[Grant::ConfigRead, Grant::ConfigWrite, Grant::ConfigWriteSessions, Grant::Hosting]);
        for method in ["config/get", "config/set", "config/check"] {
            let answer = ask(&control, &key, method, json!({"section": section, "value": {}}));
            assert!(answer["error"].is_object(), "{method} {section}: {answer}");
        }
    }
    let read = ask(&control, &owner, "config/get", json!({"section": "desktop"}));
    assert_eq!(read["result"]["value"]["start_hidden"], true, "{read}");
    let before = fs::read_to_string(&path).expect("canoa.cfg");
    let checked = ask(&control, &owner, "config/check", json!({"section": "desktop", "value": {"start_hidden": false}}));
    assert_eq!(checked["result"]["valid"], true, "{checked}");
    assert_eq!(fs::read_to_string(&path).expect("canoa.cfg"), before);
    let set = ask(&control, &owner, "config/set", json!({"section": "desktop", "value": {"start_hidden": false}}));
    assert!(set["result"].is_object(), "{set}");
    let text = fs::read_to_string(&path).expect("canoa.cfg");
    assert!(text.contains("start_hidden = false") && text.contains("queue_delivery"), "{text}");
}
