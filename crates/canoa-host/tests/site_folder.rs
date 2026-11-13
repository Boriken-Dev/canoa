//! A webpage's session works in a scratch folder the host makes from the site its `_meta` names, in the Documents folder of
//! the machine the agent runs on. The page names no path.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{Host, Scratch, example, quoted, with_link};

const SITE: &str = "dev.boriken.canoa/site";

/// The host, with a Documents folder of the test's own in place of the person's.
fn host_with_documents() -> (Host, PathBuf) {
    let scratch = Scratch::new();
    let documents = scratch.0.join("documents");
    let config = format!(
        "[host.agents.Agent]\nprogram = {}\nargs = [\"--scripted\"]\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    );
    let env = [("XDG_DOCUMENTS_DIR", documents.clone().into_os_string())];
    (Host::start_in(scratch, &config, 1, &env), documents)
}

fn site_session(
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
    site: &Value,
) -> Result<Value, Value> {
    rpc.request(
        "session/new",
        json!({"cwd": "/", "mcpServers": [], "_meta": {SITE: {"site": site}}}),
    )
}

#[test]
fn a_session_from_a_page_starts_in_a_folder_the_host_made() {
    let (host, documents) = host_with_documents();
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let told = &opened["_meta"][SITE];
        let folder = PathBuf::from(
            told["folder"]
                .as_str()
                .expect("the answer says which folder"),
        );
        let id = told["id"].as_str().expect("and its id");
        assert_eq!(told["site"], "example.com");
        assert_eq!(
            folder,
            documents
                .join("Canoa")
                .join("scratch")
                .join("example.com")
                .join(id)
        );
        assert!(folder.is_dir(), "{folder:?} exists");
        let session = opened["sessionId"].as_str().expect("an id");
        let said = rpc.prompt(session, "Cwd").expect("the agent answers");
        assert_eq!(
            said,
            format!("cwd: {}", folder.display()),
            "the agent works there"
        );

        let second = site_session(rpc, &json!("example.com")).expect("another session opens");
        assert_ne!(
            second["_meta"][SITE]["folder"], told["folder"],
            "each has its own"
        );
    })
    .expect("a paired key is served");
}

#[test]
fn a_refused_host_name_makes_nothing_and_fails_the_session() {
    let (host, documents) = host_with_documents();
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        for site in [
            json!(".."),
            json!("a/b"),
            json!("c:"),
            json!("nul"),
            json!("[::1]"),
            json!("Example.com"),
        ] {
            let refused = site_session(rpc, &site).expect_err("the session fails");
            assert_eq!(refused["code"], -32602, "{site}: {refused}");
        }
        assert!(!documents.exists(), "nothing was made");
    })
    .expect("a paired key is served");
}

#[test]
fn resume_reopens_in_the_recorded_folder_and_nowhere_else() {
    let (host, documents) = host_with_documents();
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let folder = opened["_meta"][SITE]["folder"].as_str().expect("a folder").to_owned();
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        let reopen = |cwd: &Path, site: &str| {
            rpc.request(
                "session/load",
                json!({"sessionId": session, "cwd": cwd, "mcpServers": [], "_meta": {SITE: {"site": site}}}),
            )
        };
        let again = reopen(Path::new(&folder), "example.com").expect("it reopens where it was");
        assert!(again.is_object());
        let outside = documents.join("elsewhere");
        let refused = reopen(&outside, "example.com").expect_err("outside the scratch root");
        assert_eq!(refused["code"], -32602, "{refused}");
        assert!(!outside.exists());
        let other = reopen(Path::new(&folder), "other.example").expect_err("another site's folder");
        assert_eq!(other["code"], -32602, "{other}");
    })
    .expect("a paired key is served");
}

#[test]
fn an_entry_with_a_profile_makes_the_folder_too() {
    let scratch = Scratch::new();
    let documents = scratch.0.join("documents");
    let config = format!(
        "[host.agents.Agent]\nprogram = {}\nprofiles = [\"plain\"]\n[host.profiles.plain]\n",
        quoted(&example("profile_agent", false).to_string_lossy())
    );
    let env = [("XDG_DOCUMENTS_DIR", documents.clone().into_os_string())];
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let folder = opened["_meta"][SITE]["folder"]
            .as_str()
            .expect("the answer says which folder");
        assert!(
            Path::new(folder)
                .starts_with(documents.join("Canoa").join("scratch").join("example.com"))
        );
        assert!(Path::new(folder).is_dir(), "{folder} exists");
        assert!(
            opened["_meta"]["dev.boriken.canoa/profile"].is_object(),
            "the profile report is still there: {opened}"
        );
        site_session(rpc, &json!("..")).expect_err("a refused name fails here too");
    })
    .expect("a paired key is served");
}
