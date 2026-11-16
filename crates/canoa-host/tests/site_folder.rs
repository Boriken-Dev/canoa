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

#[test]
fn a_site_names_no_directories_to_make_writable() {
    let (host, _documents) = host_with_documents();
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = rpc
            .request(
                "session/new",
                json!({"cwd": "/", "mcpServers": [], "additionalDirectories": ["/startup"],
                       "_meta": {SITE: {"site": "example.com"}}}),
            )
            .expect("a site session opens");
        let session = opened["sessionId"].as_str().expect("an id");
        let said = rpc.prompt(session, "Dirs").expect("the agent answers");
        assert_eq!(
            said, "dirs: null",
            "no directory of the site's reached the agent"
        );
    })
    .expect("a paired key is served");
}

#[test]
fn a_session_a_site_opened_is_set_only_to_modes_that_ask() {
    let scratch = Scratch::new();
    let documents = scratch.0.join("documents");
    let config = format!(
        "[host.agents.Agent]
program = {}
args = [\"--scripted\", \"--modes=default,bypassPermissions,auto\"]
",
        quoted(&example("stdio_agent", false).to_string_lossy())
    );
    let env = [("XDG_DOCUMENTS_DIR", documents.into_os_string())];
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        let set = |config: &str, value: Value| {
            rpc.request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": config, "value": value}),
            )
        };
        for value in [
            json!("bypassPermissions"),
            json!("auto"),
            json!(true),
            json!(5),
            json!(null),
        ] {
            let refused = set("mode", value.clone()).expect_err("a mode that does not ask");
            assert_eq!(refused["code"], -32602, "{value}: {refused}");
        }
        let refused = rpc
            .request(
                "session/set_mode",
                json!({"sessionId": session, "modeId": "auto"}),
            )
            .expect_err("set_mode too");
        assert_eq!(refused["code"], -32602, "{refused}");
        // An option the agent said is a model's changes nothing about asking.
        set("model", json!("fast")).expect("a model is the client's to choose");
        set("mode", json!("default")).expect("a mode that asks is the client's to choose");

        // A session no site opened is not held to it.
        let other = rpc
            .request("session/new", json!({"cwd": "/", "mcpServers": []}))
            .expect("a session with no site");
        let plain = other["sessionId"].as_str().expect("an id").to_owned();
        rpc.request(
            "session/set_config_option",
            json!({"sessionId": plain, "configId": "mode", "value": "bypassPermissions"}),
        )
        .expect("a session no site opened is not held to it");

        // Canoa's own chat in the extension marks itself, and a site's name cannot: its sessions are the person's own.
        let own = rpc
            .request(
                "session/new",
                json!({"cwd": "/", "mcpServers": [], "additionalDirectories": ["/mine"],
                       "_meta": {SITE: {"site": "canoa-extension", "own": true}}}),
            )
            .expect("the own chat's session opens");
        let chat = own["sessionId"].as_str().expect("an id").to_owned();
        assert!(
            own["_meta"][SITE].get("own").is_none(),
            "the marker is not echoed"
        );
        rpc.request(
            "session/set_config_option",
            json!({"sessionId": chat, "configId": "mode", "value": "bypassPermissions"}),
        )
        .expect("the own chat may choose bypass");
        assert_eq!(
            rpc.prompt(&chat, "Dirs").expect("it answers"),
            "dirs: [\"/mine\"]"
        );
        // Reopened as a site, the same name has no such say.
        let reopened = rpc.request(
            "session/load",
            json!({"sessionId": chat, "cwd": own["_meta"][SITE]["folder"], "mcpServers": [],
                   "_meta": {SITE: {"site": "canoa-extension"}}}),
        );
        assert!(reopened.is_ok(), "{reopened:?}");
        let refused =
            set_for(rpc, &chat, "mode", json!("bypassPermissions")).expect_err("now a site's");
        assert_eq!(refused["code"], -32602, "{refused}");
    })
    .expect("a paired key is served");
}

fn set_for(
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
    session: &str,
    config: &str,
    value: Value,
) -> Result<Value, Value> {
    rpc.request(
        "session/set_config_option",
        json!({"sessionId": session, "configId": config, "value": value}),
    )
}

/// A host whose scripted agent offers a mode that does not ask.
fn host_with_modes() -> Host {
    let scratch = Scratch::new();
    let documents = scratch.0.join("documents");
    let config = format!(
        "[host.agents.Agent]\nprogram = {}\nargs = [\"--scripted\", \"--modes=default,bypassPermissions\"]\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    );
    let env = [("XDG_DOCUMENTS_DIR", documents.into_os_string())];
    Host::start_in(scratch, &config, 1, &env)
}

const APPROVAL: &str = "dev.boriken.canoa/approval";

#[test]
fn a_mode_change_the_person_approved_is_accepted_only_with_this_connections_token() {
    let host = host_with_modes();
    let client = support::key();
    host.pair(&client);
    let token = with_link(host.url("Agent"), &client, host.peer, |rpc| {
        let init = rpc.initialize().expect("initialize is answered");
        let token = init["_meta"][APPROVAL]
            .as_str()
            .expect("the sealed answer carries the connection's token")
            .to_owned();
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        let change = |marker: Value| {
            let mut params = json!({"sessionId": session, "configId": "mode", "value": "bypassPermissions"});
            if !marker.is_null() {
                params["_meta"] = json!({APPROVAL: marker});
            }
            rpc.request("session/set_config_option", params)
        };
        for forged in [json!(null), json!("forged"), json!(""), json!(true), json!(token.to_uppercase())] {
            let refused = change(forged.clone()).expect_err("no token of this connection's");
            assert_eq!(refused["code"], -32602, "{forged}: {refused}");
        }
        assert_eq!(rpc.prompt(&session, "Mode").expect("it answers"), "mode: default");
        change(json!(token)).expect("the extension's marked change is accepted");
        assert_eq!(rpc.prompt(&session, "Mode").expect("it answers"), "mode: bypassPermissions");
        // The same on set_mode.
        let again = rpc.request(
            "session/set_mode",
            json!({"sessionId": session, "modeId": "bypassPermissions", "_meta": {APPROVAL: token}}),
        );
        assert!(again.is_ok(), "{again:?}");
        token
    })
    .expect("a paired key is served");

    // Another connection has another token, so one that was seen on the first is worth nothing on it.
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        let init = rpc.initialize().expect("initialize is answered");
        assert_ne!(init["_meta"][APPROVAL], json!(token));
        let opened = site_session(rpc, &json!("example.com")).expect("a site session opens");
        let session = opened["sessionId"].as_str().expect("an id");
        let refused = rpc
            .request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "mode", "value": "bypassPermissions", "_meta": {APPROVAL: token}}),
            )
            .expect_err("another connection's token");
        assert_eq!(refused["code"], -32602, "{refused}");
    })
    .expect("a paired key is served");
}

#[test]
fn the_desktop_app_the_web_app_and_the_own_chat_still_choose_bypass() {
    let host = host_with_modes();
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let choose = |session: &str| {
            rpc.request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "mode", "value": "bypassPermissions"}),
            )
        };
        // The desktop app names no site.
        let desktop = rpc
            .request("session/new", json!({"cwd": "/", "mcpServers": []}))
            .expect("a session with no site");
        choose(desktop["sessionId"].as_str().expect("an id")).expect("the desktop app chooses bypass");
        // The web app the host serves and the extension's own chat name a site and mark themselves; a web page never reaches the host
        // with this mark, since the extension removes everything a page sends in `_meta` and sets the mark only for its own pages.
        for site in ["example.com", "canoa-extension"] {
            let own = rpc
                .request(
                    "session/new",
                    json!({"cwd": "/", "mcpServers": [], "_meta": {SITE: {"site": site, "own": true}}}),
                )
                .expect("an own session opens");
            choose(own["sessionId"].as_str().expect("an id")).unwrap_or_else(|why| panic!("{site}: {why}"));
        }
        // Only a session placed for a web page is limited.
        let page = site_session(rpc, &json!("example.com")).expect("a page's session opens");
        let refused = choose(page["sessionId"].as_str().expect("an id")).expect_err("a page's");
        assert_eq!(refused["code"], -32602, "{refused}");
    })
    .expect("a paired key is served");
}
