use core::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::config::Config;

/// A directory of the test's own, removed afterwards.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-hosted-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its directory");
        std::fs::write(path, text).expect("written");
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SECRET: &str = "s3cret-value-never-shown";

std::thread_local! {
    /// The host's `PATH` as this thread's test sets it.
    static BIN: core::cell::RefCell<Option<String>> = const { core::cell::RefCell::new(None) };
}

fn lookup(name: &str) -> Option<String> {
    match name {
        "HOST_SECRET" => Some(SECRET.to_owned()),
        "PATH" => BIN.with(|bin| bin.borrow().clone()),
        "PATHEXT" => Some(".COM;.EXE".to_owned()),
        _ => None,
    }
}

/// A program's file name on this platform.
fn program(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// Puts programs `probe` and `node` in `bin/` and makes that directory the only one on `PATH`.
fn with_programs(temp: &Temp) {
    for name in ["probe", "node"] {
        let path = format!("bin/{}", program(name));
        temp.write(&path, "");
        #[cfg(unix)]
        std::fs::set_permissions(
            temp.0.join(&path),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("executable");
    }
    let bin = temp.0.join("bin").to_string_lossy().into_owned();
    BIN.with(|path| *path.borrow_mut() = Some(bin));
}

fn offer_of(temp: &Temp, text: &str) -> Offer {
    let config = Config::from_str_in(text, &temp.0).expect("parses");
    let crate::config::EntryKind::Agent { profiles, .. } = &config.entries[0].kind else {
        panic!("an agent");
    };
    Arc::into_inner(offer(profiles, &config.profiles, "launcher".into()).expect("an offer"))
        .expect("one owner")
}

fn config_with(extra: &str) -> String {
    format!(
        "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\", \"other\"]\n\
         [host.profiles.other]\n\
         [host.profiles.main]\nshare_env = [\"HOST_SECRET\"]\ncontext = {{ text = \"Host context.\" }}\n{extra}\n\
         [host.profiles.main.env.TOKEN]\nfrom_env = \"HOST_SECRET\"\n\
         [host.profiles.main.skills.review]\npath = \"skills/review\"\n\
         [host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = \"probe\"\nargs = [\"--x\"]\n\
         [host.profiles.main.mcp.probe.env.PROBE_TOKEN]\nfrom_env = \"HOST_SECRET\"\n"
    )
}

fn temp_with_skill() -> Temp {
    let temp = Temp::new();
    temp.write(
        "skills/review/SKILL.md",
        "---\nname: review\n---\nReview it.\n",
    );
    temp.write("skills/review/notes/more.md", "More.\n");
    with_programs(&temp);
    temp
}

fn item<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no {key} in {report}"))
}

fn claude() -> Takes {
    Takes {
        version: 1,
        capabilities: json!({"sessionCapabilities": {"systemPrompt": {}}, "mcpCapabilities": {"http": true}}),
    }
}

#[test]
fn a_client_chooses_only_among_the_profiles_its_entry_offers() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let initialize = |meta: Value| {
        serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"_meta": meta}}),
        )
        .expect("JSON")
    };
    assert_eq!(
        choose(&offer, &initialize(json!({}))).expect("the default"),
        0
    );
    assert_eq!(
        choose(&offer, &initialize(json!({PROFILE_META: "other"}))).expect("offered"),
        1
    );

    for meta in [
        json!({PROFILE_META: "elsewhere"}),
        json!({PROFILE_META: {"env": {"A": {"value": "b"}}}}),
        json!({PROFILE_META: null}),
    ] {
        let refused = choose(&offer, &initialize(meta.clone())).expect_err("refused");
        let answer: Value =
            serde_json::from_slice(&refused.answer.expect("answered")).expect("JSON");
        assert_eq!(answer["error"]["code"], INVALID_PARAMS, "{meta}: {answer}");
        assert_eq!(answer["id"], 0);
    }
    let elsewhere = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": 3, "method": "session/new", "params": {"_meta": {PROFILE_META: "main"}}}),
    )
    .expect("JSON");
    assert!(
        choose(&offer, &elsewhere).is_err(),
        "a choice outside initialize"
    );
    // Nothing is prepared or started for a first message that is not `initialize`, choice or none.
    for first in [
        json!({"jsonrpc": "2.0", "id": 4, "method": "session/new", "params": {"cwd": "/x", "mcpServers": []}}),
        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {}}),
        json!([1]),
    ] {
        let refused =
            choose(&offer, &serde_json::to_vec(&first).expect("JSON")).expect_err("not initialize");
        if first.get("id").is_some() {
            let answer: Value =
                serde_json::from_slice(&refused.answer.expect("answered")).expect("JSON");
            assert_eq!(answer["error"]["code"], INVALID_REQUEST, "{answer}");
        }
    }
}

#[test]
fn a_bare_mcp_command_is_found_on_the_hosts_path_and_never_through_a_relative_entry() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let command = parts.servers[0]["command"].as_str().expect("a command");
    let expected = temp.0.join("bin").join(program("probe"));
    let expected = expected.to_string_lossy();
    // Windows finds `probe` as `probe` and a `PATHEXT` extension in whatever case `PATHEXT` gives it.
    assert!(
        command == expected || cfg!(windows) && command.eq_ignore_ascii_case(&expected),
        "{command}: {}",
        parts.report
    );

    // A relative PATH entry names whatever directory a process runs in: never searched, so nothing is found.
    BIN.with(|path| *path.borrow_mut() = Some(".".to_owned()));
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert_eq!(
        item(&parts.report, "mcp.probe")["state"],
        "refused",
        "{}",
        parts.report
    );
    assert!(parts.servers.is_empty());
}

#[test]
fn an_unknown_or_repeated_profile_and_a_bad_allow_digest_are_refused_in_the_configuration() {
    let temp = Temp::new();
    for (text, what) in [
        (
            "[host.agents.a]\nprogram = \"x\"\nprofiles = [\"none\"]\n",
            "does not hold",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nprofiles = [\"p\", \"p\"]\n[host.profiles.p]\n",
            "twice",
        ),
        (
            "[host.profiles.p]\nallow_mcp = { s = \"sha256:12\" }\n",
            "allow_mcp.s",
        ),
        ("[host.profiles.p]\noverlays = [\"../up\"]\n", "overlays"),
        ("[host.profiles.p]\nnope = 1\n", "profile p"),
    ] {
        let error = Config::from_str_in(text, &temp.0)
            .expect_err(text)
            .to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
}

#[test]
fn claude_takes_context_skills_as_a_plugin_and_mcp_servers_and_the_report_names_no_value() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    assert_eq!(connection.env.len(), 1);
    assert_eq!(connection.env[0].1.expose(), SECRET);
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert_eq!(parts.system_prompt.as_deref(), Some("Host context."));
    let plugin =
        parts.meta.as_ref().expect("a plugin")["claudeCode"]["options"]["plugins"][0]["path"]
            .as_str()
            .expect("a path")
            .to_owned();
    assert!(
        Path::new(&plugin)
            .join(".claude-plugin/plugin.json")
            .is_file(),
        "{plugin}"
    );
    assert!(
        Path::new(&plugin)
            .join("skills/review/notes/more.md")
            .is_file(),
        "{plugin}"
    );
    assert_eq!(parts.servers.len(), 1);
    let server = &parts.servers[0];
    assert!(
        server.get("type").is_none(),
        "v1's stdio form has no type: {server}"
    );
    assert_eq!(
        server["env"],
        json!([{"name": "PROBE_TOKEN", "value": SECRET}])
    );
    for key in ["context", "env.TOKEN", "skills.review", "mcp.probe"] {
        assert_eq!(
            item(&parts.report, key)["state"],
            "applied",
            "{key}: {}",
            parts.report
        );
    }
    assert!(
        !parts.report.to_string().contains(SECRET),
        "{}",
        parts.report
    );
    drop(parts);
    assert!(
        !Path::new(&plugin).exists(),
        "the plugin goes with the connection"
    );
}

#[test]
fn pi_and_an_agent_that_says_nothing_are_told_what_they_do_not_take() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with("model = \"m\"\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let pi = parts_for(&connection, Some("pi"), &Takes::default(), &lookup);
    assert!(pi.servers.is_empty() && pi.system_prompt.is_none() && pi.written.is_none());
    for key in ["context", "skills.review", "mcp.probe", "model"] {
        assert_eq!(
            item(&pi.report, key)["state"],
            "not supported",
            "{key}: {}",
            pi.report
        );
    }
    let bare = parts_for(
        &connection,
        None,
        &Takes {
            version: 2,
            capabilities: json!({}),
        },
        &lookup,
    );
    assert_eq!(
        item(&bare.report, "mcp.probe")["state"],
        "not supported",
        "{}",
        bare.report
    );
}

#[test]
fn codex_reads_skills_from_a_directory_given_as_additional_directories() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let takes = Takes {
        version: 2,
        capabilities: json!({"session": {"additionalDirectories": {}, "systemPrompt": {}, "mcp": {"stdio": true}}}),
    };
    let parts = parts_for(&connection, Some("codex"), &takes, &lookup);
    let directory = PathBuf::from(&parts.directories[0]);
    assert!(
        directory
            .join(CODEX_SKILLS)
            .join("review/SKILL.md")
            .is_file()
    );
    assert_eq!(parts.servers[0]["type"], "stdio", "v2 types it");
}

#[test]
fn a_session_request_gets_the_parts_and_a_client_server_may_not_take_a_profile_name() {
    let parts = Parts {
        system_prompt: Some("Host.".to_owned()),
        servers: vec![json!({"name": "probe"})],
        names: vec!["probe".to_owned(), "off".to_owned()],
        ..Parts::default()
    };
    let mut params = json!({"systemPrompt": {"mode": "append", "content": [{"type": "text", "text": "Client."}]}});
    add_parts(params.as_object_mut().expect("an object"), &parts).expect("added");
    assert_eq!(params["systemPrompt"]["content"][0]["text"], "Host.");
    assert_eq!(params["systemPrompt"]["content"][1]["text"], "Client.");
    assert_eq!(params["mcpServers"], json!([{"name": "probe"}]));
    for name in ["probe", "off"] {
        let mut taken =
            json!({"mcpServers": [{"type": "http", "name": name, "url": "https://x.example"}]});
        assert!(
            add_parts(taken.as_object_mut().expect("an object"), &parts).is_err(),
            "{name}"
        );
    }
}

/// An installed overlay with one stdio MCP server, as the app installs one: its files and a record holding their digest.
fn install_overlay(temp: &Temp) -> String {
    temp.write(
        "overlays/team/overlay.cfg",
        "[env]\nTEAM = \"crew\"\n\n[[mcp.servers]]\nname = \"team\"\ntransport = \"stdio\"\ncommand = \"node\"\n\
         args = [\"serve.mjs\"]\nenv = { WHO = \"${TEAM}-bot\", WHERE = \"${AOS_OVERLAY_ROOT}\" }\n\
         env_passthrough = [\"NOT_SHARED\"]\n",
    );
    let tree = aos::read_tree(&temp.0.join("overlays/team"), 64, 1 << 20).expect("read");
    let digest = aos::digest_entries(&tree).expect("a digest").digest;
    temp.write(
        "overlays/team.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    digest
}

#[test]
fn an_overlays_mcp_server_stays_off_until_allow_mcp_names_it_at_its_digest() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    let offer = offer_of(&temp, &config_with("overlays = [\"team\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let team = item(&parts.report, "mcp.team");
    assert_eq!(team["state"], "off", "{}", parts.report);
    assert!(!parts.servers.iter().any(|server| server["name"] == "team"));
    let digest = team["digest"].as_str().expect("a digest").to_owned();

    for (allowed, on) in [
        (format!("sha256:{}", "0".repeat(64)), false),
        (digest, true),
    ] {
        let offer = offer_of(
            &temp,
            &config_with(&format!(
                "overlays = [\"team\"]\nallow_mcp = {{ team = \"{allowed}\" }}\n"
            )),
        );
        let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
        let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
        let server = parts.servers.iter().find(|server| server["name"] == "team");
        assert_eq!(server.is_some(), on, "{allowed}: {}", parts.report);
        if let Some(server) = server {
            let env: BTreeMap<&str, &str> = server["env"]
                .as_array()
                .expect("env")
                .iter()
                .map(|pair| {
                    (
                        pair["name"].as_str().expect("a name"),
                        pair["value"].as_str().expect("a value"),
                    )
                })
                .collect();
            assert_eq!(env["WHO"], "crew-bot");
            assert!(env["WHERE"].ends_with("team"), "{}", env["WHERE"]);
            assert!(
                !env.contains_key("NOT_SHARED"),
                "a variable the host's profile does not share is withheld"
            );
        }
    }
}

#[test]
fn an_overlay_whose_files_changed_since_it_was_installed_is_left_out() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    temp.write("overlays/team/extra.md", "Added after the install.\n");
    let offer = offer_of(
        &temp,
        &config_with(
            "overlays = [\"team\"]\nallow_mcp = { team = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\" }\n",
        ),
    );
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert_eq!(
        item(&parts.report, "overlay")["state"],
        "refused",
        "{}",
        parts.report
    );
    assert!(
        parts.report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["key"] != "mcp.team")
    );
}

#[test]
fn a_profile_whose_environment_cannot_be_built_starts_no_agent() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let error = Connection::prepare(&offer, 0, "a", &|_| None).expect_err("HOST_SECRET is not set");
    assert!(error.contains("env.TOKEN"), "{error}");
}
