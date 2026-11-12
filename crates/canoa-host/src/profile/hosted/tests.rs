use core::sync::atomic::{AtomicU64, Ordering};

use coqui_aos as aos;

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
    Arc::into_inner(offer(profiles, &config.profiles, None, "launcher".into()).expect("an offer"))
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
fn the_rust_translators_are_named_by_what_they_report() {
    assert_eq!(
        harness_of(Some("coqui-acp-claude"), "hosted"),
        Some("claude-code")
    );
    assert_eq!(harness_of(Some("coqui-acp-codex"), "hosted"), Some("codex"));
    assert_eq!(harness_of(Some("coqui-acp-pi"), "Pi"), None);
    assert_eq!(harness_of(Some("pi-acp"), "mine"), Some("pi"));
}

#[test]
fn an_initialize_answer_is_read_where_its_version_puts_what_it_says() {
    let v2 = json!({
        "protocolVersion": 2,
        "info": {"name": "coqui-acp-codex", "version": "0"},
        "capabilities": {"session": {"additionalDirectories": {}, "systemPrompt": {}, "mcp": {"stdio": true}}},
    });
    let (takes, name) = described(&v2);
    assert_eq!(name, Some("coqui-acp-codex"));
    assert!(takes.additional_directories() && takes.system_prompt() && takes.mcp("stdio"));
    let v1 = json!({
        "protocolVersion": 1,
        "agentInfo": {"name": "coqui-acp-claude", "version": "0"},
        "agentCapabilities": {"sessionCapabilities": {"systemPrompt": {}}},
    });
    let (takes, name) = described(&v1);
    assert_eq!(name, Some("coqui-acp-claude"));
    assert!(takes.system_prompt() && !takes.additional_directories());
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
    let (_, digest) = aos::read_tree(
        &aos::StdFs,
        &temp.0.join("overlays/team"),
        &aos::Settings::default(),
    )
    .expect("read");
    let digest = digest.digest.to_string();
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

const SKILL_SOURCE: &str = "https://skills.example.test/tools.git#skills/audit";

/// The tree digest of the directory `path` under `temp`.
fn digest_of(temp: &Temp, path: &str) -> String {
    aos::read_tree(&aos::StdFs, &temp.0.join(path), &aos::Settings::default())
        .expect("read")
        .1
        .digest
        .to_string()
}

/// An installed overlay `team` naming skill `audit` by `source` and `pin`, as the app installs one.
fn install_naming(temp: &Temp, source: &str, pin: Option<&str>) {
    let pin = pin.map_or_else(String::new, |pin| format!(", pin = \"{pin}\""));
    temp.write(
        "overlays/team/overlay.cfg",
        &format!(
            "name = \"team\"\nskills = [{{ name = \"audit\", source = \"{source}\"{pin} }}]\n"
        ),
    );
    let digest = digest_of(temp, "overlays/team");
    temp.write(
        "overlays/team.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
}

/// The files of a fetched `audit` and the record the app keeps of it, given by `record` but for its digest.
fn fetch_audit(temp: &Temp, front_name: &str, record: &Value) -> String {
    temp.write(
        "overlays/.skills/audit/SKILL.md",
        &format!("---\nname: {front_name}\n---\nAudit it.\n"),
    );
    temp.write("overlays/.skills/audit/notes/more.md", "More.\n");
    let digest = digest_of(temp, "overlays/.skills/audit");
    let mut record = record.clone();
    record["digest"] = json!(digest);
    temp.write("overlays/.skills/audit.json", &record.to_string());
    digest
}

fn record_for(overlays: &[&str]) -> Value {
    json!({"name": "audit", "source": SKILL_SOURCE, "overlays": overlays, "chosen": false})
}

/// The profile's parts for Claude Code; the connection stays alive with them, since the plugin goes with either.
fn audit_parts(temp: &Temp) -> (Connection, Parts) {
    audit_parts_in(temp, &config_with("overlays = [\"team\"]\n"))
}

fn audit_parts_in(temp: &Temp, config: &str) -> (Connection, Parts) {
    let offer = offer_of(temp, config);
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    (connection, parts)
}

fn plugin_of(parts: &Parts) -> PathBuf {
    PathBuf::from(
        parts.meta.as_ref().expect("a plugin")["claudeCode"]["options"]["plugins"][0]["path"]
            .as_str()
            .expect("a path"),
    )
}

#[test]
fn a_skill_an_overlay_names_by_source_is_delivered_from_its_fetched_copy_at_its_pin() {
    let temp = temp_with_skill();
    let digest = fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, Some(&digest));
    let (_connection, parts) = audit_parts(&temp);
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "applied", "{}", parts.report);
    assert_eq!(audit["layer"], "overlay:team", "{}", parts.report);
    let plugin = plugin_of(&parts);
    assert!(plugin.join("skills/audit/SKILL.md").is_file());
    assert!(plugin.join("skills/audit/notes/more.md").is_file());
    assert!(
        plugin.join("skills/review/SKILL.md").is_file(),
        "the profile's own"
    );
}

#[test]
fn a_skill_with_no_pin_is_delivered_and_reported_as_unpinned() {
    let temp = temp_with_skill();
    fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, None);
    let (_connection, parts) = audit_parts(&temp);
    assert_eq!(item(&parts.report, "skills.audit")["state"], "applied");
    assert!(
        item(&parts.report, "skills.audit.unpinned")["how"]
            .as_str()
            .expect("how")
            .contains("no pin"),
        "{}",
        parts.report
    );
}

#[test]
fn a_fetched_copy_that_is_not_what_its_record_names_or_is_another_skill_is_not_delivered() {
    // Files changed after the record was written.
    let temp = temp_with_skill();
    fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, None);
    temp.write("overlays/.skills/audit/notes/more.md", "Changed.\n");
    let (_connection, parts) = audit_parts(&temp);
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "refused", "{}", parts.report);
    assert!(
        audit["how"].as_str().expect("how").contains("digest"),
        "{audit}"
    );
    assert!(!plugin_of(&parts).join("skills/audit").exists());
    assert!(
        plugin_of(&parts).join("skills/review").is_dir(),
        "the profile's own skill still goes"
    );

    // A skill whose front matter names another, at the digest its record names.
    let temp = temp_with_skill();
    fetch_audit(&temp, "other", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, None);
    let (_connection, parts) = audit_parts(&temp);
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "refused", "{}", parts.report);
    assert!(
        audit["how"]
            .as_str()
            .expect("how")
            .contains("skill-name-mismatch"),
        "{audit}"
    );
}

#[test]
fn a_skill_nothing_fetched_is_reported_and_the_overlays_other_parts_still_apply() {
    let temp = temp_with_skill();
    install_naming(&temp, SKILL_SOURCE, None);
    let (_connection, parts) = audit_parts(&temp);
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "not supported", "{}", parts.report);
    let how = audit["how"].as_str().expect("how");
    assert!(
        how.contains("skills.example.test/tools.git#skills/audit"),
        "{how}"
    );
    assert!(how.contains("not fetched"), "{how}");
    assert_eq!(item(&parts.report, "skills.review")["state"], "applied");
    assert_eq!(item(&parts.report, "context")["state"], "applied");
}

#[test]
fn a_named_skill_with_no_record_is_reported_as_resolved_by_no_catalog() {
    let temp = temp_with_skill();
    temp.write(
        "overlays/team/overlay.cfg",
        "name = \"team\"\nskills = [\"audit\"]\n",
    );
    let digest = digest_of(&temp, "overlays/team");
    temp.write(
        "overlays/team.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    let (_connection, parts) = audit_parts(&temp);
    let how = item(&parts.report, "skills.audit")["how"]
        .as_str()
        .expect("how")
        .to_owned();
    assert!(how.contains("no catalog resolved it"), "{how}");
}

#[test]
fn a_record_that_does_not_list_the_overlay_or_names_another_source_or_pin_is_not_used() {
    let other_pin = format!("sha256:{}", "ab".repeat(32));
    for (what, record, pin, state) in [
        (
            "another overlay's",
            record_for(&["elsewhere"]),
            None,
            "not supported",
        ),
        (
            "another source",
            json!({"name": "audit", "source": "https://skills.example.test/other.git", "overlays": ["team"]}),
            None,
            "refused",
        ),
        (
            "another pin",
            record_for(&["team"]),
            Some(other_pin.as_str()),
            "refused",
        ),
        (
            "another skill's",
            json!({"name": "other", "source": SKILL_SOURCE, "overlays": ["team"]}),
            None,
            "refused",
        ),
    ] {
        let temp = temp_with_skill();
        fetch_audit(&temp, "audit", &record);
        install_naming(&temp, SKILL_SOURCE, pin);
        let (_connection, parts) = audit_parts(&temp);
        assert_eq!(
            item(&parts.report, "skills.audit")["state"],
            state,
            "{what}: {}",
            parts.report
        );
        assert!(!plugin_of(&parts).join("skills/audit").exists(), "{what}");
    }
}

#[test]
fn a_skill_name_that_is_not_one_path_component_is_never_read_from_the_store() {
    for name in [
        "con", "NUL.txt", "..", ".hidden", "a/b", "a\\b", "pdf.", "PROGRA~1", "", "nul",
    ] {
        assert!(!fetched::confined(name), "{name:?}");
    }
    for name in ["audit", "Review", "pdf-2", "a_b.c"] {
        assert!(fetched::confined(name), "{name:?}");
    }
}

/// Every item of the report with `key` that belongs to `layer`.
fn items_of<'a>(report: &'a Value, key: &str, layer: &str) -> Vec<&'a Value> {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["key"] == key && item["layer"] == layer)
        .collect()
}

/// An installed overlay `name` with `files`, and the record the app wrote for it.
fn install_files(temp: &Temp, name: &str, files: &[(&str, &str)]) {
    for (path, text) in files {
        temp.write(&format!("overlays/{name}/{path}"), text);
    }
    let digest = digest_of(temp, &format!("overlays/{name}"));
    temp.write(
        &format!("overlays/{name}.json"),
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
}

#[test]
fn a_later_overlays_fetched_skill_does_not_take_the_name_an_earlier_overlays_bundled_one_holds() {
    let temp = temp_with_skill();
    install_files(
        &temp,
        "base",
        &[
            ("overlay.cfg", "name = \"base\"\n"),
            (
                "skills/audit/SKILL.md",
                "---\nname: audit\n---\nBase audit.\n",
            ),
        ],
    );
    fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_files(
        &temp,
        "team",
        &[(
            "overlay.cfg",
            &format!(
                "name = \"team\"\nrequires = [\"base\"]\nskills = [{{ name = \"audit\", source = \"{SKILL_SOURCE}\" }}]\n"
            ),
        )],
    );
    let (_connection, parts) =
        audit_parts_in(&temp, &config_with("overlays = [\"base\", \"team\"]\n"));
    let text = std::fs::read_to_string(plugin_of(&parts).join("skills/audit/SKILL.md"))
        .expect("the skill is written");
    assert!(text.contains("Base audit."), "{text}");
    assert!(
        !plugin_of(&parts)
            .join("skills/audit/notes/more.md")
            .exists(),
        "none of the fetched copy"
    );
    assert_eq!(
        items_of(&parts.report, "skills.audit", "overlay:base")[0]["state"],
        "applied"
    );
    let shadowed = items_of(&parts.report, "skills.audit", "overlay:team");
    assert_eq!(shadowed.len(), 1, "{}", parts.report);
    assert_eq!(shadowed[0]["state"], "refused", "{}", parts.report);
}

#[test]
fn a_fetched_skill_over_plain_http_is_reported_with_its_setting() {
    let temp = temp_with_skill();
    let source = "http://skills.example.test/tools.git#skills/audit";
    let mut record = record_for(&["team"]);
    record["source"] = json!(source);
    fetch_audit(&temp, "audit", &record);
    install_naming(&temp, source, None);
    let (_connection, parts) = audit_parts(&temp);
    assert_eq!(item(&parts.report, "skills.audit")["state"], "applied");
    let note = item(&parts.report, "skills.audit.insecure-transport");
    assert_eq!(note["state"], "applied", "{}", parts.report);
}

#[test]
fn a_pin_in_canoa_cfg_must_match_too_and_anchors_a_skill_that_names_none() {
    let temp = temp_with_skill();
    let digest = fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, None);
    let configured = |digest: &str| {
        format!(
            "{}\n[skill_pins]\nAudit = \"{digest}\"\n",
            config_with("overlays = [\"team\"]\n")
        )
    };

    let (_connection, parts) = audit_parts_in(&temp, &configured(&digest));
    assert_eq!(item(&parts.report, "skills.audit")["state"], "applied");
    assert!(
        !parts.report.to_string().contains("skills.audit.unpinned"),
        "a configured pin anchors it: {}",
        parts.report
    );

    let other = format!("sha256:{}", "ab".repeat(32));
    let (_connection, parts) = audit_parts_in(&temp, &configured(&other));
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "refused", "{}", parts.report);
    assert!(
        audit["how"].as_str().expect("how").contains("skill_pins"),
        "{audit}"
    );
    assert!(!plugin_of(&parts).join("skills/audit").exists());

    // The entry's own pin must match as well: the configured one does not stand in for it.
    install_naming(&temp, SKILL_SOURCE, Some(&other));
    let (_connection, parts) = audit_parts_in(&temp, &configured(&digest));
    assert_eq!(
        item(&parts.report, "skills.audit")["state"],
        "refused",
        "{}",
        parts.report
    );
}

#[test]
fn skill_pins_in_canoa_cfg_are_names_and_digests_or_the_file_is_refused() {
    let base = |pins: &str| {
        Config::from_str_in(
            &format!("{}\n[skill_pins]\n{pins}\n", config_with("")),
            Path::new("."),
        )
    };
    let digest = format!("sha256:{}", "ab".repeat(32));
    assert!(base(&format!("audit = \"{digest}\"")).is_ok());
    for bad in [
        "audit = \"sha256:abc\"".to_owned(),
        "audit = 3".to_owned(),
        format!("\"../x\" = \"{digest}\""),
        format!("con = \"{digest}\""),
        format!("audit = \"{digest}\"\nAudit = \"{digest}\""),
    ] {
        assert!(base(&bad).is_err(), "{bad}");
    }
}

#[test]
fn a_malformed_record_is_reported_and_never_stops_the_session_or_reads_outside_the_store() {
    let long = "x".repeat(200_000);
    let digest_less = json!({"name": "audit", "source": SKILL_SOURCE, "overlays": ["team"]});
    for (what, text) in [
        ("an array", "[1, 2]".to_owned()),
        ("a number", "3".to_owned()),
        ("not JSON", "{".to_owned()),
        ("wrong types", json!({"name": 3, "overlays": "team", "digest": 4, "source": []}).to_string()),
        ("overlays of numbers", json!({"name": "audit", "overlays": [1, null, {"a": 1}], "digest": null}).to_string()),
        ("no digest", digest_less.to_string()),
        ("a path as the digest", json!({"name": "audit", "source": SKILL_SOURCE, "overlays": ["team"], "digest": "../../team"}).to_string()),
        ("a huge record", json!({"name": "audit", "overlays": ["team"], "pad": long}).to_string()),
    ] {
        let temp = temp_with_skill();
        fetch_audit(&temp, "audit", &record_for(&["team"]));
        temp.write("overlays/.skills/audit.json", &text);
        install_naming(&temp, SKILL_SOURCE, None);
        let (_connection, parts) = audit_parts(&temp);
        let audit = item(&parts.report, "skills.audit");
        assert!(
            audit["state"] == "refused" || audit["state"] == "not supported",
            "{what}: {}",
            parts.report
        );
        assert!(!plugin_of(&parts).join("skills/audit").exists(), "{what}");
    }
}

#[test]
fn a_skill_directory_that_is_a_link_is_never_read() {
    let temp = temp_with_skill();
    fetch_audit(&temp, "audit", &record_for(&["team"]));
    install_naming(&temp, SKILL_SOURCE, None);
    // `.skills/audit` becomes a link to a directory with the same files: the record's digest still matches them.
    let real = temp.0.join("overlays/.skills/audit");
    let moved = temp.0.join("elsewhere");
    std::fs::rename(&real, &moved).expect("moved");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&moved, &real).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&moved, &real).is_ok();
    if !linked {
        eprintln!("SKIPPED: this account cannot make a symbolic link");
        return;
    }
    let (_connection, parts) = audit_parts(&temp);
    assert_ne!(
        item(&parts.report, "skills.audit")["state"],
        "applied",
        "{}",
        parts.report
    );
}
