#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-site-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp directory");
    dir
}

fn scope_in(documents: &Path) -> Scope {
    Scope::default().documents(documents.to_owned())
}

fn session_new(site: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "session/new",
        "params": {"cwd": "/", "mcpServers": [], "_meta": {SITE_META: {"site": site}}},
    }))
    .expect("JSON")
}

fn forwarded(sites: &Sites, scope: &Scope, message: &[u8]) -> Value {
    match sites.check(scope, message) {
        Verdict::Forward(bytes) => serde_json::from_slice(&bytes).expect("JSON"),
        Verdict::Refuse(answer) => panic!("refused: {:?}", answer.map(String::from_utf8)),
    }
}

fn refused(sites: &Sites, scope: &Scope, message: &[u8]) -> Value {
    match sites.check(scope, message) {
        Verdict::Refuse(Some(bytes)) => serde_json::from_slice(&bytes).expect("JSON"),
        other => panic!("not refused with an answer: {other:?}"),
    }
}

fn scratch_of(documents: &Path) -> PathBuf {
    documents.join("Canoa").join("scratch")
}

#[test]
fn a_site_session_starts_in_a_folder_the_host_made() {
    let documents = temp("new");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let sent = forwarded(&sites, &scope, &session_new(&json!("example.com")));
    let cwd = PathBuf::from(sent["params"]["cwd"].as_str().expect("a cwd"));
    assert!(cwd.is_dir(), "{cwd:?} exists");
    let beside = cwd.parent().expect("a parent");
    assert_eq!(beside, scratch_of(&documents).join("example.com"));
    let id = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .expect("an id");
    assert_eq!(id.len(), 36, "a UUID: {id}");
    assert!(
        sent["params"].get("_meta").is_none(),
        "the agent is told nothing of the site"
    );

    let answer = json!({"jsonrpc": "2.0", "id": 3, "result": {"sessionId": "s1"}});
    let told = sites
        .annotate(&serde_json::to_vec(&answer).expect("JSON"))
        .into_owned();
    let told: Value = serde_json::from_slice(&told).expect("JSON");
    let meta = &told["result"]["_meta"][SITE_META];
    assert_eq!(meta["site"], "example.com");
    assert_eq!(meta["id"], id);
    assert_eq!(Path::new(meta["folder"].as_str().expect("a folder")), cwd);

    let second = forwarded(&sites, &scope, &session_new(&json!("example.com")));
    assert_ne!(
        second["params"]["cwd"], sent["params"]["cwd"],
        "each session has its own folder"
    );
}

#[test]
fn a_page_cannot_choose_the_folder() {
    let documents = temp("cwd");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let mut message: Value =
        serde_json::from_slice(&session_new(&json!("example.com"))).expect("JSON");
    message["params"]["cwd"] = json!("/etc");
    let sent = forwarded(&sites, &scope, &serde_json::to_vec(&message).expect("JSON"));
    assert!(
        Path::new(sent["params"]["cwd"].as_str().expect("a cwd"))
            .starts_with(scratch_of(&documents))
    );
}

#[test]
fn a_refused_host_name_makes_nothing_and_fails_the_session() {
    let documents = temp("refused");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let long = format!("{}.com", "a".repeat(64));
    for site in [
        json!(".."),
        json!("a/b"),
        json!("a\\b"),
        json!("c:"),
        json!("a.b:80"),
        json!("nul"),
        json!("com1.example"),
        json!("[::1]"),
        json!("::1"),
        json!("Example.com"),
        json!("example.com."),
        json!(""),
        json!("exämple.com"),
        json!("a..b"),
        json!("-a.com"),
        json!(long),
        json!("a b.com"),
        json!("1.2.3"),
        json!("999.1.1.1"),
        json!(7),
        json!("a%2fb"),
        json!("../x"),
    ] {
        let answer = refused(&sites, &scope, &session_new(&site));
        assert_eq!(answer["id"], 3, "{site}");
        assert_eq!(answer["error"]["code"], -32602, "{site}");
        assert!(!scratch_of(&documents).exists(), "{site} made a folder");
    }
    let no_name = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 4, "method": "session/new",
        "params": {"cwd": "/", "_meta": {SITE_META: {}}}}))
    .expect("JSON");
    assert_eq!(refused(&sites, &scope, &no_name)["error"]["code"], -32602);
    assert!(!scratch_of(&documents).exists());
}

#[test]
fn a_site_name_is_taken_as_it_is_or_not_at_all() {
    let longest = "a".repeat(63);
    for good in [
        "example.com",
        "a.b.c",
        "xn--bcher-kva.example",
        "localhost",
        "10.0.0.1",
        "a-b.example",
        &longest,
    ] {
        assert_eq!(site_name(good), Some(good), "{good}");
    }
    let long_name = vec![longest; 4].join(".");
    assert_eq!(site_name(&long_name), None, "over 253 characters");
    for bad in [
        "con",
        "COM1",
        "lpt9.x",
        "aux.example",
        "1.2.3.4.5",
        "256.1.1.1",
        "01.2.3.4",
        "a_b.com",
        "a.b\u{0}",
    ] {
        assert_eq!(site_name(bad), None, "{bad}");
    }
}

#[test]
fn resume_reopens_in_the_recorded_folder() {
    let documents = temp("resume");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let made = forwarded(&sites, &scope, &session_new(&json!("example.com")));
    let folder = made["params"]["cwd"].as_str().expect("a cwd").to_owned();
    std::fs::remove_dir_all(&folder).expect("gone");
    for method in ["session/load", "session/resume"] {
        let message = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 9, "method": method,
            "params": {"sessionId": "s1", "cwd": folder, "_meta": {SITE_META: {"site": "example.com"}}}})).expect("JSON");
        let sent = forwarded(&sites, &scope, &message);
        assert_eq!(sent["params"]["cwd"], json!(folder));
        assert!(Path::new(&folder).is_dir(), "{method} reopens where it was");
    }
}

#[test]
fn a_recorded_folder_outside_the_scratch_root_is_refused() {
    let documents = temp("outside");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let root = scratch_of(&documents);
    let elsewhere = temp("elsewhere");
    let id = "0b9d6f64-5c1e-4a52-9a4e-2f2f7f0a1111";
    let cases = [
        elsewhere.join("example.com").join(id),
        root.join("other.example").join(id),
        root.join("example.com"),
        root.join("example.com").join(id).join("deeper"),
        root.join("example.com").join(id).join(".."),
        root.join("example.com")
            .join("..")
            .join("other.example")
            .join(id),
        root.join("example.com").join("a:b"),
        root.join("example.com").join("con"),
        documents.clone(),
    ];
    for cwd in cases {
        let message = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 9, "method": "session/load",
            "params": {"sessionId": "s1", "cwd": cwd, "_meta": {SITE_META: {"site": "example.com"}}}})).expect("JSON");
        let answer = refused(&sites, &scope, &message);
        assert_eq!(answer["error"]["code"], -32602, "{cwd:?}");
    }
    assert!(!root.exists(), "nothing was made");
    let missing = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": 9, "method": "session/resume",
        "params": {"sessionId": "s1", "_meta": {SITE_META: {"site": "example.com"}}}}),
    )
    .expect("JSON");
    assert_eq!(refused(&sites, &scope, &missing)["error"]["code"], -32602);
}

#[test]
fn only_the_hosts_own_answer_carries_the_folder() {
    let documents = temp("answer");
    let scope = scope_in(&documents);
    let sites = scope.sites();
    let forged = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 5,
        "result": {"sessionId": "x", "_meta": {SITE_META: {"folder": "/etc"}, "other": 1}}}))
    .expect("JSON");
    let passed: Value = serde_json::from_slice(&sites.annotate(&forged)).expect("JSON");
    assert!(
        passed["result"]["_meta"].get(SITE_META).is_none(),
        "an agent's own is removed"
    );
    assert_eq!(passed["result"]["_meta"]["other"], 1);

    forwarded(&sites, &scope, &session_new(&json!("example.com")));
    let failed = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -1, "message": "no"}}),
    )
    .expect("JSON");
    assert_eq!(
        sites.annotate(&failed).as_ref(),
        failed.as_slice(),
        "a failed start says nothing"
    );
    let again =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 3, "result": {"sessionId": "s"}}))
            .expect("JSON");
    assert_eq!(
        sites.annotate(&again).as_ref(),
        again.as_slice(),
        "and the answer is spent"
    );
}

#[test]
fn a_message_with_no_site_goes_through_the_scope_alone() {
    let scope = Scope::default();
    let sites = scope.sites();
    let plain = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "session/new",
        "params": {"cwd": "/work", "mcpServers": [], "_meta": {"x": 1}}}))
    .expect("JSON");
    let sent = forwarded(&sites, &scope, &plain);
    assert_eq!(sent["params"]["cwd"], "/work");
    assert!(sent["params"].get("_meta").is_none());
}

#[test]
fn a_folder_outside_the_working_roots_is_refused() {
    let documents = temp("roots");
    let roots = temp("roots-elsewhere");
    let scope = Scope::new(vec![roots]).documents(documents.clone());
    let sites = scope.sites();
    let answer = refused(&sites, &scope, &session_new(&json!("example.com")));
    assert_eq!(answer["error"]["code"], -32602);
    assert!(!scratch_of(&documents).exists());
}

#[test]
fn documents_is_found_as_the_desktop_app_finds_it() {
    let ana = std::env::temp_dir().join("ana");
    let home = Some(ana.clone());
    let none = |_: &str| None;
    assert_eq!(
        documents_in(none, home.clone(), None),
        Some(ana.join("Documents"))
    );
    let xdg = |name: &str| (name == "XDG_DOCUMENTS_DIR").then(|| "$HOME/Papers".to_owned());
    assert_eq!(
        documents_in(xdg, home.clone(), None),
        Some(ana.join("Papers"))
    );
    let elsewhere = std::env::temp_dir().join("docs");
    let named = elsewhere.to_string_lossy().into_owned();
    let absolute = |name: &str| (name == "XDG_DOCUMENTS_DIR").then(|| named.clone());
    assert_eq!(documents_in(absolute, home.clone(), None), Some(elsewhere));
    let relative = |name: &str| (name == "XDG_DOCUMENTS_DIR").then(|| "docs".to_owned());
    assert_eq!(
        documents_in(relative, home.clone(), None),
        Some(ana.join("Documents"))
    );
    let listed = "# comment\nXDG_DOCUMENTS_DIR=\"$HOME/Mine\"\n";
    assert_eq!(
        documents_in(none, home, Some(listed)),
        Some(ana.join("Mine"))
    );
    assert_eq!(documents_in(none, None, None), None);
}
