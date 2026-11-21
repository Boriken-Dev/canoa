use core::sync::atomic::{AtomicU64, Ordering};

use coqui_aos as aos;

use super::*;
use crate::config::Config;
use crate::scope::Scope;

/// Where Codex looks for the skills of a directory it is given, which its description says.
const AGENT_SKILLS: &str = ".agents/skills";

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
    Arc::into_inner(
        offer(
            profiles,
            &config.profiles,
            None,
            "launcher".into(),
            Vec::new(),
        )
        .expect("an offer"),
    )
    .expect("one owner")
}

fn config_with(extra: &str) -> String {
    format!(
        "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\", \"other\"]\n\
         [host.profiles.other]\n\
         [host.profiles.main]\ncontext = {{ text = \"Host context.\" }}\n{extra}\n\
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
    let token = connection
        .env
        .iter()
        .find(|(name, _)| name == "TOKEN")
        .expect("the profile's variable");
    assert_eq!(token.1.expose(), SECRET);
    // Beside it only what names the session, never a value of the person's.
    assert!(
        connection
            .env
            .iter()
            .all(|(name, _)| name == "TOKEN" || name.starts_with("AGENTS_")),
        "{:?}",
        connection
            .env
            .iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    );
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
            .join(AGENT_SKILLS)
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

/// Whether two paths name one file as Windows reads them: the same text with either separator, in any case.
fn same_file_name(a: &Path, b: &Path) -> bool {
    let text = |path: &Path| {
        let text = path.to_string_lossy().replace('/', "\\");
        if cfg!(windows) {
            text.to_ascii_lowercase()
        } else {
            text
        }
    };
    text(a) == text(b)
}

/// A small real program and the arguments that make it end with status 0, to put in an overlay's `bin/` as a server's command.
fn small_program() -> (PathBuf, Vec<&'static str>) {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        (
            PathBuf::from(root).join("System32").join("cmd.exe"),
            vec!["/d", "/c", "exit 0"],
        )
    } else {
        (PathBuf::from("/bin/sh"), vec!["-c", "exit 0"])
    }
}

#[test]
fn an_mcp_servers_bare_command_is_found_in_its_overlays_bin_as_well_as_on_the_hosts_path_and_starts()
 {
    let temp = temp_with_skill();
    let (original, args) = small_program();
    let tool = program("canoa-test-overlay-tool");
    temp.write("overlays/tools/overlay.cfg", "[[mcp.servers]]\nname = \"tool\"\ntransport = \"stdio\"\ncommand = \"canoa-test-overlay-tool\"\n");
    std::fs::create_dir_all(temp.0.join("overlays/tools/bin")).expect("bin");
    std::fs::copy(&original, temp.0.join("overlays/tools/bin").join(&tool)).expect("a program");
    // The same name on the host's own path is not the one an overlay's server runs: the overlay's own folder comes first.
    temp.write(&format!("bin/{tool}"), "");
    let digest = digest_of(&temp, "overlays/tools");
    temp.write(
        "overlays/tools.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"tools\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let server = item(&parts.report, "mcp.tool");
    assert_eq!(server["state"], "off", "{}", parts.report);
    let allowed = server["digest"].as_str().expect("a digest").to_owned();

    let offer = offer_of(
        &temp,
        &config_with(&format!(
            "overlays = [\"tools\"]\nallow_mcp = {{ tool = \"{allowed}\" }}\n"
        )),
    );
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let started = parts
        .servers
        .iter()
        .find(|server| server["name"] == "tool")
        .unwrap_or_else(|| panic!("the server is given to the agent: {}", parts.report));
    let command = PathBuf::from(started["command"].as_str().expect("a command"));
    let inside = temp.0.join("overlays/tools/bin").join(&tool);
    assert!(
        same_file_name(&command, &inside),
        "{command:?} is not {inside:?}"
    );
    // What the agent is given starts.
    let status = std::process::Command::new(&command)
        .args(&args)
        .status()
        .expect("starts");
    assert!(status.success(), "{status}");

    // A command no folder holds is still not found, and the host's own path still finds what only it holds.
    temp.write(
        "overlays/tools/overlay.cfg",
        "[[mcp.servers]]\nname = \"tool\"\ntransport = \"stdio\"\ncommand = \"canoa-test-absent-program\"\n",
    );
    let digest = digest_of(&temp, "overlays/tools");
    temp.write(
        "overlays/tools.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"tools\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert_eq!(
        item(&parts.report, "mcp.tool")["state"],
        "refused",
        "{}",
        parts.report
    );
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
#[cfg_attr(
    windows,
    ignore = "needs permission to make a symbolic link: Developer Mode or an administrator"
)]
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
    assert!(linked, "this account can make a symbolic link");
    let (_connection, parts) = audit_parts(&temp);
    assert_ne!(
        item(&parts.report, "skills.audit")["state"],
        "applied",
        "{}",
        parts.report
    );
}

#[test]
fn a_client_narrows_the_overlays_a_connection_starts_with_and_never_adds_one() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    let offer = offer_of(&temp, &config_with("overlays = [\"team\"]\n"));
    let report_of = |only: Option<Vec<String>>| {
        let connection = Connection::prepare_with(
            &offer,
            0,
            "a",
            &lookup,
            Choices {
                only: only.as_deref(),
                ..Choices::default()
            },
            Starting::default(),
        )
        .expect("prepared");
        parts_for(&connection, Some("claude-code"), &claude(), &lookup).report
    };

    // Every listed overlay when the client chooses none.
    let all = report_of(None);
    assert_eq!(item(&all, "mcp.team")["state"], "off", "{all}");

    // Left out: nothing of it applies, and the report says it was not chosen.
    let none = report_of(Some(Vec::new()));
    assert_eq!(item(&none, "overlay")["state"], "off", "{none}");
    assert_eq!(item(&none, "overlay")["layer"], "overlay:team", "{none}");
    assert!(
        none["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["key"] != "mcp.team"),
        "{none}"
    );

    // A name the profile does not list adds nothing, and the report says so.
    let ghost = report_of(Some(vec!["team".into(), "ghost".into()]));
    assert_eq!(item(&ghost, "mcp.team")["state"], "off", "{ghost}");
    assert_eq!(item(&ghost, "overlay")["state"], "not supported", "{ghost}");
    assert_eq!(item(&ghost, "overlay")["layer"], "overlay:ghost", "{ghost}");
}

#[test]
fn the_overlays_are_chosen_as_a_list_of_names_or_the_first_message_is_refused() {
    let first = |value: Value| {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"_meta": {OVERLAYS_META: value}}}))
        .expect("json")
    };
    assert_eq!(
        choose_overlays(&first(json!(["a", "b"]))).expect("names"),
        Some(vec!["a".into(), "b".into()])
    );
    assert_eq!(
        choose_overlays(b"{\"method\":\"initialize\"}").expect("none"),
        None
    );
    for bad in [json!("a"), json!([1]), json!({"a": true})] {
        let refusal = choose_overlays(&first(bad)).expect_err("refused");
        let answer: Value =
            serde_json::from_slice(&refusal.answer.expect("an answer")).expect("json");
        assert_eq!(answer["error"]["code"], INVALID_PARAMS, "{answer}");
    }
}

const OPTIONS_MANIFEST: &str = "name = \"docs\"\n\
    [options.depth]\ntype = \"string\"\ndefault = \"normal\"\n\
    [options.rounds]\ntype = \"integer\"\ndefault = 2\n\
    [options.token]\ntype = \"string\"\nsensitive = true\n";

/// The profile's parts for Claude Code with overlay `docs` installed from `files` and `options` as canoa.cfg gives them.
fn docs_parts(files: &[(&str, &str)], options: &str) -> (Temp, Connection, Parts) {
    let temp = temp_with_skill();
    install_files(&temp, "docs", files);
    let config = format!(
        "[overlays.docs]\n{options}\n{}",
        config_with("overlays = [\"docs\"]\n")
    );
    let (connection, parts) = audit_parts_in(&temp, &config);
    (temp, connection, parts)
}

#[test]
fn a_bundled_skills_body_is_served_with_the_overlays_private_copy_as_its_root() {
    let skill = "---\nname: audit\ndescription: For ${AOS_OVERLAY_NAME}\n---\n\
        Read ${AOS_OVERLAY_ROOT}/kb.md, ${AOS_OVERLAY_NAME}, ${HOME} and $${AOS_OVERLAY_ROOT}.\n";
    let (temp, _connection, parts) = docs_parts(
        &[
            ("overlay.cfg", OPTIONS_MANIFEST),
            ("kb.md", "Knowledge.\n"),
            ("skills/audit/SKILL.md", skill),
        ],
        "",
    );
    let text = std::fs::read_to_string(plugin_of(&parts).join("skills/audit/SKILL.md"))
        .expect("the skill is written");
    let (head, body) = text.split_once("\n---\n").expect("front matter");
    assert_eq!(
        head,
        "---\nname: audit\ndescription: For ${AOS_OVERLAY_NAME}"
    );
    let line = body.trim_end().strip_prefix("Read ").expect("the body");
    let (root, rest) = line.split_once("/kb.md").expect("a root and a path");
    assert_eq!(rest, ", docs, ${HOME} and ${AOS_OVERLAY_ROOT}.");
    let root = Path::new(root);
    assert_eq!(
        std::fs::read_to_string(root.join("kb.md")).expect("a copy of the overlay"),
        "Knowledge.\n"
    );
    assert!(
        !root.starts_with(temp.0.join("overlays")),
        "the root is the private copy, not the installed files: {}",
        root.display()
    );
    // The installed file keeps its names and its digest.
    let installed = std::fs::read_to_string(temp.0.join("overlays/docs/skills/audit/SKILL.md"))
        .expect("installed");
    assert_eq!(installed, skill);
    let record: Value = serde_json::from_slice(
        &std::fs::read(temp.0.join("overlays/docs.json")).expect("its record"),
    )
    .expect("JSON");
    assert_eq!(record["digest"], digest_of(&temp, "overlays/docs").as_str());
}

fn prompt_of(parts: &Parts) -> String {
    parts.system_prompt.clone().expect("a system prompt")
}

#[test]
fn a_configured_option_is_used_in_context_and_the_manifests_default_otherwise() {
    let files = [
        ("overlay.cfg", OPTIONS_MANIFEST),
        (
            "CONTEXT.md",
            "Depth ${depth}, rounds ${rounds}, ${nothing}.\n",
        ),
    ];
    let (_temp, _connection, parts) =
        docs_parts(&files, "options = { depth = \"deep\", rounds = 5 }");
    let prompt = prompt_of(&parts);
    assert!(
        prompt.contains("Depth deep, rounds 5, ${nothing}."),
        "{prompt}"
    );

    let (_temp, _connection, parts) = docs_parts(&files, "");
    let prompt = prompt_of(&parts);
    assert!(
        prompt.contains("Depth normal, rounds 2, ${nothing}."),
        "{prompt}"
    );
}

#[test]
fn a_sensitive_option_never_reaches_the_context_even_when_configured() {
    let (_temp, _connection, parts) = docs_parts(
        &[
            ("overlay.cfg", OPTIONS_MANIFEST),
            (
                "CONTEXT.md",
                "Token is ${token}, depth ${depth}.
",
            ),
        ],
        "options = { token = \"tok-never-shown\" }",
    );
    let prompt = prompt_of(&parts);
    assert!(
        prompt.contains("Token is ${token}, depth normal."),
        "{prompt}"
    );
    assert!(!prompt.contains("tok-never-shown"), "{prompt}");
    assert!(!parts.report.to_string().contains("tok-never-shown"));
}

#[test]
fn an_option_of_the_wrong_type_refuses_the_text_that_reads_it_naming_the_key() {
    let (_temp, _connection, parts) = docs_parts(
        &[
            ("overlay.cfg", OPTIONS_MANIFEST),
            ("CONTEXT.md", "Rounds ${rounds}.\n"),
            (
                "skills/audit/SKILL.md",
                "---\nname: audit\n---\nBody ${rounds}.\n",
            ),
        ],
        "options = { rounds = \"many\" }",
    );
    // Only the host's own context is left: the overlay's is refused whole.
    assert_eq!(parts.system_prompt.as_deref(), Some("Host context."));
    for key in ["context", "skills.audit"] {
        let refused = items_of(&parts.report, key, "overlay:docs");
        assert_eq!(refused[0]["state"], "refused", "{}", parts.report);
        let how = refused[0]["how"].as_str().expect("words");
        assert!(how.contains("rounds"), "{how}");
        assert!(!how.contains("many"), "a value is never quoted: {how}");
    }
}

#[test]
fn rules_are_not_templated_and_a_context_naming_the_project_gets_each_sessions_folder() {
    let (_temp, _connection, parts) = docs_parts(
        &[
            ("overlay.cfg", OPTIONS_MANIFEST),
            ("rules/style.md", "Rule ${depth}.\n"),
            (
                "CONTEXT.md",
                "Work in ${AOS_PROJECT_ROOT}, depth ${depth}.\n",
            ),
        ],
        "options = { depth = \"deep\" }",
    );
    let prompt = prompt_of(&parts);
    assert!(prompt.contains("Rule ${depth}."), "{prompt}");
    assert!(
        prompt.contains("Work in ${AOS_PROJECT_ROOT}, depth deep."),
        "no folder yet: {prompt}"
    );

    for folder in ["/work/one", "/work/two"] {
        let mut params = json!({"cwd": folder, "mcpServers": []});
        add_parts(params.as_object_mut().expect("an object"), &parts).expect("added");
        let text = params["systemPrompt"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(
            text.contains(&format!("Work in {folder}, depth deep.")),
            "{text}"
        );
        assert!(text.contains("Rule ${depth}."), "{text}");
    }
    // A session with no folder gets the text as it stands.
    let mut params = json!({"mcpServers": []});
    add_parts(params.as_object_mut().expect("an object"), &parts).expect("added");
    assert_eq!(
        params["systemPrompt"]["content"][0]["text"],
        prompt.as_str()
    );
}

#[test]
fn the_body_of_overlay_md_is_templated_and_its_front_matter_is_not_context() {
    let (_temp, _connection, parts) = docs_parts(
        &[(
            "OVERLAY.md",
            "---\nname: docs\noptions:\n  depth:\n    type: string\n    default: normal\n---\nBody at ${depth}, in ${AOS_PROJECT_ROOT}.\n",
        )],
        "options = { depth = \"deep\" }",
    );
    let prompt = prompt_of(&parts);
    assert!(
        prompt.contains("Body at deep, in ${AOS_PROJECT_ROOT}."),
        "{prompt}"
    );
    assert!(!prompt.contains("options:"), "{prompt}");
    let mut params = json!({"cwd": "/work/one", "mcpServers": []});
    add_parts(params.as_object_mut().expect("an object"), &parts).expect("added");
    let text = params["systemPrompt"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text.contains("Body at deep, in /work/one."), "{text}");
}

/// The profile's parts for Claude Code of the person's own top-level `text` (`[loadouts]`, `[skill_pins]`), for an agent `a`.
fn chosen_parts(temp: &Temp, text: &str) -> (Connection, Parts) {
    let config = Config::from_str_in(&format!("[agents.a]\nprogram = \"agent\"\n{text}"), &temp.0)
        .expect("parses");
    let local = config.local_profiles.for_agent("a").expect("a profile");
    let offer = Arc::into_inner(
        offer(
            &[],
            &IndexMap::new(),
            Some(&local),
            "launcher".into(),
            Vec::new(),
        )
        .expect("an offer"),
    )
    .expect("one owner");
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    (connection, parts)
}

/// A skill the person chose, as the app keeps it, and its digest.
fn choose_audit(temp: &Temp, chosen: bool) -> String {
    let mut record = record_for(&[]);
    record["chosen"] = json!(chosen);
    fetch_audit(temp, "audit", &record)
}

#[test]
fn a_skill_the_person_chose_is_delivered_from_its_record_when_the_files_agree() {
    let temp = Temp::new();
    choose_audit(&temp, true);
    let (_connection, parts) = chosen_parts(
        &temp,
        "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n",
    );
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "applied", "{}", parts.report);
    assert_eq!(audit["layer"], "user", "{}", parts.report);
    let plugin = plugin_of(&parts);
    assert!(plugin.join("skills/audit/SKILL.md").is_file());
    assert!(plugin.join("skills/audit/notes/more.md").is_file());
}

#[test]
fn a_chosen_skill_whose_files_changed_after_the_record_was_written_is_not_delivered_and_is_reported()
 {
    let temp = Temp::new();
    choose_audit(&temp, true);
    temp.write("overlays/.skills/audit/notes/more.md", "Changed.\n");
    let (_connection, parts) = chosen_parts(
        &temp,
        "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n",
    );
    let audit = item(&parts.report, "skills.audit");
    assert_eq!(audit["state"], "refused", "{}", parts.report);
    assert!(
        audit["how"].as_str().expect("how").contains("digest"),
        "{audit}"
    );
    assert!(parts.meta.is_none() || !plugin_of(&parts).join("skills/audit").exists());

    // A file added, and one that is another skill's, are not what the record names either.
    let temp = Temp::new();
    choose_audit(&temp, true);
    temp.write("overlays/.skills/audit/extra.md", "Added.\n");
    let (_connection, parts) = chosen_parts(
        &temp,
        "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n",
    );
    assert_eq!(item(&parts.report, "skills.audit")["state"], "refused");

    // A pin in canoa.cfg that the record's digest is not.
    let temp = Temp::new();
    choose_audit(&temp, true);
    let pin = format!("sha256:{}", "0".repeat(64));
    let (_connection, parts) = chosen_parts(
        &temp,
        &format!(
            "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n[skill_pins]\naudit = \"{pin}\"\n"
        ),
    );
    assert_eq!(item(&parts.report, "skills.audit")["state"], "refused");
}

#[test]
fn a_skill_a_record_does_not_say_the_person_chose_is_not_delivered_whoever_names_it() {
    let temp = Temp::new();
    choose_audit(&temp, false);
    let (_connection, parts) = chosen_parts(
        &temp,
        "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n",
    );
    assert_eq!(
        item(&parts.report, "skills.audit")["state"],
        "not supported"
    );
    assert!(parts.meta.is_none() || !plugin_of(&parts).join("skills/audit").exists());
}

#[test]
fn a_chosen_skill_with_no_record_is_reported_and_one_not_listed_is_never_read() {
    let temp = Temp::new();
    let (_connection, parts) = chosen_parts(
        &temp,
        "[loadouts.main]\nskills = [\"audit\"]\n[defaults]\nloadout = \"main\"\n",
    );
    assert_eq!(
        item(&parts.report, "skills.audit")["state"],
        "not supported"
    );
    // Fetched and chosen, but not named by the loadout.
    let temp = Temp::new();
    choose_audit(&temp, true);
    let config = Config::from_str_in(
        "[agents.a]\nprogram = \"agent\"\n[loadouts.main]\noverlays = []\n[defaults]\nloadout = \"main\"\n",
        &temp.0,
    )
    .expect("parses");
    let local = config.local_profiles.for_agent("a").expect("a profile");
    let resolved = local
        .resolve_loadout(None)
        .expect("known")
        .expect("a default");
    assert!(resolved.skills.is_empty());
}

/// The offer of the person's own layer for agent `a` from `text`, with overlay `team` installed.
fn loadout_offer(temp: &Temp, text: &str) -> Offer {
    let config = Config::from_str_in(&format!("[agents.a]\nprogram = \"agent\"\n{text}"), &temp.0)
        .expect("parses");
    let local = config.local_profiles.for_agent("a").expect("a profile");
    Arc::into_inner(
        offer(
            &[],
            &IndexMap::new(),
            Some(&local),
            "launcher".into(),
            Vec::new(),
        )
        .expect("an offer"),
    )
    .expect("one owner")
}

/// A session of `offer` that names `loadout`, and the report it starts with.
fn loadout_report(offer: &Offer, loadout: Option<&str>) -> (Connection, Value) {
    let connection = Connection::prepare_with(
        offer,
        0,
        "a",
        &lookup,
        Choices {
            loadout,
            ..Choices::default()
        },
        Starting::default(),
    )
    .expect("prepared");
    let report = parts_for(&connection, Some("claude-code"), &claude(), &lookup).report;
    (connection, report)
}

const LOADOUTS: &str = "[overlays]\nteam = \"https://example.invalid/team.git\"\n\
    [profile.mcp.own]\ntype = \"stdio\"\ncommand = \"probe\"\n\
    [profile.mcp.spare]\ntype = \"stdio\"\ncommand = \"node\"\n\
    [loadouts.full]\noverlays = [\"team\"]\nmcp = [\"own\"]\n\
    [loadouts.lean]\nmcp = [\"spare\", \"ghost\"]\n\
    [defaults]\nloadout = \"full\"\n";

#[test]
fn a_session_starts_with_exactly_the_overlays_and_servers_of_its_loadout() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    let offer = loadout_offer(&temp, LOADOUTS);

    // The default loadout, when the session names none: its overlay, and the person's servers it does not name are off.
    let (connection, full) = loadout_report(&offer, None);
    // The loadout is the role the knowledge tools tell a template the agent is in.
    assert_eq!(connection.kb_caller().role.as_deref(), Some("full"));
    assert_eq!(item(&full, "mcp.team")["state"], "off", "{full}");
    assert_ne!(item(&full, "mcp.own")["state"], "off", "{full}");
    assert_eq!(item(&full, "mcp.spare")["state"], "off", "{full}");

    // Another loadout by name: no overlay, so nothing of `team` applies; its own server, and one the profile lacks.
    let (named, lean) = loadout_report(&offer, Some("lean"));
    assert_eq!(named.kb_caller().role.as_deref(), Some("lean"));
    assert_eq!(item(&lean, "overlay")["state"], "off", "{lean}");
    assert_eq!(item(&lean, "overlay")["layer"], "overlay:team", "{lean}");
    assert!(
        lean["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["key"] != "mcp.team"),
        "{lean}"
    );
    assert_eq!(item(&lean, "mcp.own")["state"], "off", "{lean}");
    assert_ne!(item(&lean, "mcp.spare")["state"], "off", "{lean}");
    assert_eq!(item(&lean, "mcp.ghost")["state"], "not supported", "{lean}");
}

#[test]
fn an_overlay_the_client_names_wins_over_the_loadouts_and_skills_come_from_the_loadout() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    choose_audit(&temp, true);
    let offer = loadout_offer(
        &temp,
        &format!("{LOADOUTS}[loadouts.lean2]\nskills = [\"audit\"]\n"),
    );
    let connection = Connection::prepare_with(
        &offer,
        0,
        "a",
        &lookup,
        Choices {
            only: Some(&["team".to_owned()]),
            loadout: Some("lean2"),
            ..Choices::default()
        },
        Starting::default(),
    )
    .expect("prepared");
    let report = parts_for(&connection, Some("claude-code"), &claude(), &lookup).report;
    assert_eq!(item(&report, "mcp.team")["state"], "off", "{report}");
    assert_eq!(
        item(&report, "skills.audit")["state"],
        "applied",
        "{report}"
    );
}

#[test]
fn a_loadout_a_session_names_that_the_profile_does_not_hold_is_refused_with_its_name() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, LOADOUTS);
    let first = |value: Value| {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"_meta": {LOADOUT_META: value}}}))
        .expect("json")
    };
    assert_eq!(
        choose_loadout(&offer, 0, &first(json!("lean"))).expect("held"),
        Some("lean".to_owned())
    );
    assert_eq!(
        choose_loadout(&offer, 0, b"{\"method\":\"initialize\"}").expect("none"),
        None
    );
    for bad in [json!("ghost"), json!(3), json!(["lean"])] {
        let refusal = choose_loadout(&offer, 0, &first(bad.clone())).expect_err("refused");
        let answer: Value =
            serde_json::from_slice(&refusal.answer.expect("an answer")).expect("json");
        assert_eq!(answer["error"]["code"], INVALID_PARAMS, "{answer}");
        if bad == json!("ghost") {
            assert!(
                answer["error"]["message"]
                    .as_str()
                    .expect("text")
                    .contains("ghost"),
                "{answer}"
            );
        }
    }
}

const PRESETS: &str = "[profile]\nmodel = \"mine\"\neffort = \"low\"\n\
    [agents.a.profile]\nmode = \"plan\"\n\
    [loadouts.lean]\nmcp = []\n\
    [profiles.deep]\nmodel = \"big\"\neffort = \"high\"\ncontext = \"Read the tests first.\"\nloadout = \"lean\"\n\
    [profiles.orphan]\nloadout = \"gone\"\n";

/// A session of `offer` that names the preset `preset`, and the report it starts with.
fn preset_report(offer: &Offer, preset: Option<&str>) -> (Connection, Value) {
    let connection = Connection::prepare_with(
        offer,
        0,
        "a",
        &lookup,
        Choices {
            preset,
            ..Choices::default()
        },
        Starting::default(),
    )
    .expect("prepared");
    let report = parts_for(&connection, Some("claude-code"), &claude(), &lookup).report;
    (connection, report)
}

#[test]
fn a_preset_is_the_sessions_layer_above_the_person_and_the_agent_and_gives_its_loadout() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, PRESETS);
    let (connection, report) = preset_report(&offer, Some("deep"));
    assert_eq!(item(&report, "model")["layer"], "session", "{report}");
    assert_eq!(item(&report, "model")["value"], "big", "{report}");
    assert_eq!(item(&report, "effort")["layer"], "session", "{report}");
    // What the preset leaves alone stays the lower layer's.
    assert_eq!(item(&report, "mode")["layer"], "agent:a", "{report}");
    // The starting instructions are added to the initial context, as the session's own part of it.
    let context = items_of(&report, "context", "session");
    assert_eq!(context.len(), 1, "{report}");
    assert_eq!(context[0]["state"], "applied", "{report}");
    assert_eq!(connection.role.as_deref(), Some("lean"));
    // With no preset the person's own values stand and nothing is the session's.
    let (_, bare) = preset_report(&offer, None);
    assert_eq!(item(&bare, "model")["value"], "mine", "{bare}");
    assert!(items_of(&bare, "context", "session").is_empty(), "{bare}");
}

#[test]
fn a_preset_whose_loadout_is_gone_says_so_and_the_default_applies() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, &format!("{PRESETS}[defaults]\nloadout = \"lean\"\n"));
    let (connection, report) = preset_report(&offer, Some("orphan"));
    assert_eq!(connection.role.as_deref(), Some("lean"));
    assert_eq!(
        item(&report, "loadout")["state"],
        "not supported",
        "{report}"
    );
}

#[test]
fn a_preset_a_client_names_that_the_profile_does_not_hold_is_refused_with_its_name() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, PRESETS);
    let first = |value: Value| {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"_meta": {PRESET_META: value}}}))
        .expect("json")
    };
    assert_eq!(
        choose_preset(&offer, 0, &first(json!("deep"))).expect("held"),
        Some("deep".to_owned())
    );
    assert_eq!(
        choose_preset(&offer, 0, b"{\"method\":\"initialize\"}").expect("none"),
        None
    );
    for bad in [json!("ghost"), json!(3), json!(["deep"])] {
        let refusal = choose_preset(&offer, 0, &first(bad.clone())).expect_err("refused");
        let answer: Value =
            serde_json::from_slice(&refusal.answer.expect("an answer")).expect("json");
        assert_eq!(answer["error"]["code"], INVALID_PARAMS, "{answer}");
        if bad == json!("ghost") {
            assert!(
                answer["error"]["message"]
                    .as_str()
                    .expect("text")
                    .contains("ghost"),
                "{answer}"
            );
        }
    }
}
/// What a session of `offer` starts with when it names `loadout`, `preset` and the kind of client it is: the loadout it got, and the
/// report it started with.
fn client_start(
    offer: &Offer,
    loadout: Option<&str>,
    preset: Option<&str>,
    client: Option<&str>,
) -> (Option<String>, Value) {
    let connection = Connection::prepare_with(
        offer,
        0,
        "a",
        &lookup,
        Choices {
            only: None,
            loadout,
            preset,
            client,
        },
        Starting::default(),
    )
    .expect("prepared");
    let report = parts_for(&connection, Some("claude-code"), &claude(), &lookup).report;
    (connection.role.clone(), report)
}

#[test]
fn the_sessions_loadout_variable_is_the_loadout_the_rule_chose_the_kinds_default_included() {
    let temp = temp_with_skill();
    let offer = loadout_offer(
        &temp,
        "[loadouts.full]\nmcp = []\n[loadouts.mail]\nmcp = []\n[loadouts.lean]\nmcp = []\n\
         [defaults]\nloadout = \"full\"\n[defaults.clients]\noutlook = \"mail\"\nword = \"gone\"\n",
    );
    let loadout_of = |loadout, client| {
        let connection = Connection::prepare_with(
            &offer,
            0,
            "a",
            &lookup,
            Choices {
                loadout,
                client,
                ..Choices::default()
            },
            Starting::default(),
        )
        .expect("prepared");
        env_value(&connection, "AGENTS_LOADOUT").map(str::to_owned)
    };
    assert_eq!(loadout_of(None, Some("outlook")).as_deref(), Some("mail"));
    assert_eq!(
        loadout_of(Some("lean"), Some("outlook")).as_deref(),
        Some("lean")
    );
    assert_eq!(loadout_of(None, Some("excel")).as_deref(), Some("full"));
    assert_eq!(loadout_of(None, None).as_deref(), Some("full"));
    // A kind's loadout the file lost leaves the default.
    assert_eq!(loadout_of(None, Some("word")).as_deref(), Some("full"));
}

#[test]
fn a_client_kind_has_a_default_loadout_below_what_the_session_or_its_preset_names() {
    let temp = temp_with_skill();
    let offer = loadout_offer(
        &temp,
        &format!(
            "{PRESETS}[profiles.plain]\nmodel = \"big\"\n\
             [loadouts.mail]\nmcp = []\n[loadouts.full]\nmcp = []\n\
             [defaults]\nloadout = \"full\"\n[defaults.clients]\noutlook = \"mail\"\n"
        ),
    );
    let role = |loadout, preset, client| client_start(&offer, loadout, preset, client).0;
    // The kind's loadout, where it has one; the default for a kind with none, and for a client that says none.
    assert_eq!(role(None, None, Some("outlook")).as_deref(), Some("mail"));
    assert_eq!(role(None, None, Some("word")).as_deref(), Some("full"));
    assert_eq!(
        role(None, None, Some("unheard-of")).as_deref(),
        Some("full")
    );
    assert_eq!(role(None, None, None).as_deref(), Some("full"));
    // A loadout the session names, and the loadout of the preset it names, win over the kind's.
    assert_eq!(
        role(Some("lean"), None, Some("outlook")).as_deref(),
        Some("lean")
    );
    assert_eq!(
        role(None, Some("deep"), Some("outlook")).as_deref(),
        Some("lean")
    );
    // A preset with no loadout of its own leaves the kind's.
    assert_eq!(
        role(None, Some("plain"), Some("outlook")).as_deref(),
        Some("mail")
    );
    // The report says which loadout the session got, and null where none applies.
    let (_, report) = client_start(&offer, None, None, Some("outlook"));
    assert_eq!(report["loadout"], "mail", "{report}");
}

#[test]
fn a_client_default_naming_a_loadout_the_file_lost_says_so_and_the_default_applies() {
    let temp = temp_with_skill();
    let offer = loadout_offer(
        &temp,
        "[loadouts.full]\nmcp = []\n[defaults]\nloadout = \"full\"\n\
         [defaults.clients]\noutlook = \"gone\"\n",
    );
    let (role, report) = client_start(&offer, None, None, Some("outlook"));
    assert_eq!(role.as_deref(), Some("full"));
    let line = item(&report, "loadout");
    assert_eq!(line["state"], "not supported", "{report}");
    assert!(
        line["how"].as_str().expect("text").contains("gone"),
        "{report}"
    );
    // A kind with no entry is not told anything.
    let (_, other) = client_start(&offer, None, None, Some("word"));
    assert!(
        other["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["key"] != "loadout"),
        "{other}"
    );
}

#[test]
fn a_client_that_names_its_kind_in_text_is_heard_and_anything_else_is_refused() {
    let first = |value: Value| {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"_meta": {CLIENT_META: value}}}))
        .expect("json")
    };
    assert_eq!(
        choose_client(&first(json!("outlook"))).expect("text"),
        Some("outlook".to_owned())
    );
    assert_eq!(
        choose_client(b"{\"method\":\"initialize\"}").expect("none"),
        None
    );
    for bad in [json!(3), json!(["outlook"]), json!(null)] {
        let refusal = choose_client(&first(bad)).expect_err("refused");
        let answer: Value =
            serde_json::from_slice(&refusal.answer.expect("an answer")).expect("json");
        assert_eq!(answer["error"]["code"], INVALID_PARAMS, "{answer}");
    }
}

#[test]
fn a_change_to_canoa_cfg_after_a_session_started_leaves_that_sessions_report_as_it_was() {
    let temp = temp_with_skill();
    install_overlay(&temp);
    let offer = loadout_offer(&temp, LOADOUTS);
    let (started, before) = loadout_report(&offer, Some("full"));

    // The person empties the loadout; a session started now sees it, the one running does not.
    let edited = loadout_offer(
        &temp,
        &LOADOUTS.replace(
            "[loadouts.full]\noverlays = [\"team\"]\nmcp = [\"own\"]\n",
            "[loadouts.full]\n",
        ),
    );
    let (_, after) = loadout_report(&edited, Some("full"));
    assert_ne!(before, after);
    let again = parts_for(&started, Some("claude-code"), &claude(), &lookup).report;
    assert_eq!(before, again);
}

/// What `agy acp` 1.5.9 answers to `initialize`, trimmed to the parts that decide how it is served.
fn antigravity_initialize() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "sessionCapabilities": {"additionalDirectories": {}, "resume": {}, "close": {}, "load": {}, "list": {}},
            "_meta": {"antigravity": {"tier": true, "systemPrompt": {"append": true}}},
        },
        "agentInfo": {"name": "Antigravity ACP Server", "version": "1.5.9"},
    })
}

#[test]
fn antigravity_is_named_by_what_it_reports() {
    assert_eq!(
        harness_of(Some("Antigravity ACP Server"), "mine"),
        Some("antigravity")
    );
    assert_eq!(harness_of(None, "Antigravity"), Some("antigravity"));
}

#[test]
fn antigravity_reads_skills_from_an_additional_directory_the_connection_made() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let initialize = antigravity_initialize();
    let (takes, reported) = described(&initialize);
    let harness = harness_of(reported, "mine");
    let parts = parts_for(&connection, harness, &takes, &lookup);
    let directory = PathBuf::from(&parts.directories[0]);
    assert!(
        directory
            .join(AGENT_SKILLS)
            .join("review/SKILL.md")
            .is_file(),
        "{}",
        parts.report
    );
    assert_eq!(
        item(&parts.report, "skills.review")["state"],
        "applied",
        "{}",
        parts.report
    );
}

#[test]
fn an_antigravity_that_takes_no_additional_directories_is_told_why() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let takes = Takes {
        version: 1,
        capabilities: json!({"sessionCapabilities": {}}),
    };
    let parts = parts_for(&connection, Some("antigravity"), &takes, &lookup);
    assert!(parts.directories.is_empty() && parts.written.is_none());
    let skill = item(&parts.report, "skills.review");
    assert_eq!(skill["state"], "not supported", "{}", parts.report);
    assert!(
        skill["how"]
            .as_str()
            .unwrap_or("")
            .contains("additionalDirectories"),
        "{}",
        parts.report
    );
}

#[test]
fn a_skill_the_host_adds_reaches_claude_as_a_plugin_skill_and_codex_as_a_skill_directory() {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let skill = || crate::store::mcp::skill_files();
    let connection = Connection::prepare(&offer, 0, "a", &lookup)
        .expect("prepared")
        .with_skill(crate::store::mcp::SKILL_NAME, skill());
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let plugin =
        parts.meta.as_ref().expect("a plugin")["claudeCode"]["options"]["plugins"][0]["path"]
            .as_str()
            .expect("a path")
            .to_owned();
    let text = std::fs::read_to_string(Path::new(&plugin).join("skills/canoa-store/SKILL.md"))
        .expect("the skill is in the plugin");
    assert!(text.starts_with("---\nname: canoa-store\n"), "{text}");
    assert!(
        Path::new(&plugin).join("skills/review/SKILL.md").is_file(),
        "the profile's own skills are still there"
    );
    drop(parts);

    let takes = Takes {
        version: 2,
        capabilities: json!({"session": {"additionalDirectories": {}, "systemPrompt": {}, "mcp": {"stdio": true}}}),
    };
    let parts = parts_for(&connection, Some("codex"), &takes, &lookup);
    let directory = PathBuf::from(&parts.directories[0]);
    assert!(
        directory
            .join(AGENT_SKILLS)
            .join("canoa-store/SKILL.md")
            .is_file()
    );
    assert_eq!(
        item(&parts.report, "skills.canoa-store")["state"],
        "applied",
        "{}",
        parts.report
    );
}

#[test]
fn a_connection_without_the_added_skill_has_none_and_a_skill_of_the_same_name_the_person_chose_stays_theirs()
 {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(""));
    let plain = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&plain, Some("claude-code"), &claude(), &lookup);
    let plugin =
        parts.meta.as_ref().expect("a plugin")["claudeCode"]["options"]["plugins"][0]["path"]
            .as_str()
            .expect("a path")
            .to_owned();
    assert!(!Path::new(&plugin).join("skills/canoa-store").exists());
    drop(parts);
    let mine: BTreeMap<String, Vec<u8>> = [(
        "SKILL.md".to_owned(),
        b"---\nname: canoa-store\n---\nMine.\n".to_vec(),
    )]
    .into();
    let mut chosen = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    chosen.chosen.insert("canoa-store".to_owned(), mine);
    let chosen = chosen.with_skill(
        crate::store::mcp::SKILL_NAME,
        crate::store::mcp::skill_files(),
    );
    let parts = parts_for(&chosen, Some("claude-code"), &claude(), &lookup);
    let plugin =
        parts.meta.as_ref().expect("a plugin")["claudeCode"]["options"]["plugins"][0]["path"]
            .as_str()
            .expect("a path")
            .to_owned();
    let text = std::fs::read_to_string(Path::new(&plugin).join("skills/canoa-store/SKILL.md"))
        .expect("a skill");
    assert!(text.contains("Mine."), "{text}");
}

#[test]
fn an_overlay_served_as_an_index_leaves_its_files_out_of_the_context_only_when_the_tools_are_given()
{
    let files = [
        ("overlay.cfg", OPTIONS_MANIFEST),
        ("CONTEXT.md", "Depth ${depth}.\n"),
        ("rules/style.md", "Prefer short functions.\n"),
    ];
    let (temp, connection, parts) = docs_parts(&files, "kb = \"index\"");
    // The client did not ask for the tools: nothing changes, so no knowledge is lost.
    let prompt = prompt_of(&parts);
    assert!(
        prompt.contains("Prefer short functions.") && prompt.contains("Depth normal."),
        "{prompt}"
    );
    assert!(!prompt.contains("kb_search"), "{prompt}");

    let connection = connection.with_kb_tools();
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    let prompt = prompt_of(&parts);
    assert!(
        !prompt.contains("Prefer short functions.") && !prompt.contains("Depth normal."),
        "{prompt}"
    );
    assert!(
        prompt.contains("Host context."),
        "the person's own context stays: {prompt}"
    );
    assert_eq!(
        prompt.matches("kb_search (overlay: docs)").count(),
        1,
        "{prompt}"
    );

    let sources = connection.kb_sources();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].served);
    assert_eq!(sources[0].identity, "docs");
    assert_eq!(
        sources[0].digest,
        digest_of(&temp, "overlays/docs").as_str()
    );
    assert!(sources[0].tree.paths().any(|path| path == "rules/style.md"));

    // `files`, and no key, are the default.
    for option in ["kb = \"files\"", ""] {
        let (_temp, connection, _parts) = docs_parts(&files, option);
        let connection = connection.with_kb_tools();
        let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
        assert!(
            prompt_of(&parts).contains("Prefer short functions."),
            "{option}"
        );
        assert!(!connection.kb_sources()[0].served);
    }
}

#[test]
fn an_overlays_own_hint_serves_its_knowledge_until_the_person_says_files() {
    let hinted = format!("{OPTIONS_MANIFEST}[knowledge]\nserve = true\n");
    let files = [
        ("overlay.cfg", hinted.as_str()),
        ("rules/style.md", "Prefer short functions.\n"),
    ];
    // No `kb` key: the overlay's hint decides, and the tools being given is what leaves the files out.
    let (_temp, connection, _parts) = docs_parts(&files, "");
    assert!(connection.kb_sources()[0].served);
    let connection = connection.with_kb_tools();
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert!(!prompt_of(&parts).contains("Prefer short functions."));
    // The person's `files` wins over the hint, and `index` over its absence.
    let (_temp, connection, _parts) = docs_parts(&files, "kb = \"files\"");
    assert!(!connection.kb_sources()[0].served);
    let connection = connection.with_kb_tools();
    let parts = parts_for(&connection, Some("claude-code"), &claude(), &lookup);
    assert!(prompt_of(&parts).contains("Prefer short functions."));
    let plain = [
        ("overlay.cfg", OPTIONS_MANIFEST),
        ("rules/style.md", "Prefer short functions.\n"),
    ];
    let (_temp, connection, _parts) = docs_parts(&plain, "");
    assert!(!connection.kb_sources()[0].served);
}

#[test]
fn the_knowledge_tools_are_told_the_profile_the_loadout_and_the_sessions_variables() {
    let files = [("overlay.cfg", OPTIONS_MANIFEST)];
    let (_temp, connection, _parts) = docs_parts(&files, "");
    let caller = connection.kb_caller();
    assert_eq!(caller.profile.as_deref(), Some("main"));
    assert_eq!(caller.role, None, "no loadout was named");
    // The session's own variables, never the host's others.
    assert!(
        caller
            .env
            .iter()
            .any(|(name, value)| name == "AGENTS_PROFILE" && value == "main"),
        "{:?}",
        caller.env.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    assert!(
        caller
            .env
            .iter()
            .all(|(name, _)| name.starts_with("AGENTS_"))
    );
}

#[test]
fn an_overlays_bin_and_lib_reach_the_environment_every_harness_is_started_with() {
    use crate::profile::overlay::resolve_path;
    let temp = temp_with_skill();
    install_files(
        &temp,
        "team",
        &[
            ("CONTEXT.md", "Team.\n"),
            ("bin/tool", "x"),
            ("lib/python/t.py", "x"),
            ("lib/node/t.js", "x"),
        ],
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"team\"]\n"));
    let root = temp.0.join("overlays").join("team");
    let root = root.to_string_lossy();
    let separator = if cfg!(windows) { ';' } else { ':' };
    // The host's own entries on the three list variables, which the overlay's go in front of.
    let host = |name: &str| match name {
        "PYTHONPATH" => Some("host-py".to_owned()),
        "NODE_PATH" => Some("host-node".to_owned()),
        _ => lookup(name),
    };
    let expected = [
        ("PATH", resolve_path(&root, "bin", cfg!(windows))),
        (
            "PYTHONPATH",
            resolve_path(&root, "lib/python", cfg!(windows)),
        ),
        ("NODE_PATH", resolve_path(&root, "lib/node", cfg!(windows))),
    ]
    .map(|(name, directory)| {
        (
            name,
            format!("{directory}{separator}{}", host(name).expect("held")),
        )
    });
    // Claude, Codex and Pi run in the host's process and are given the environment as a request's; Grok, Antigravity and any
    // other program get it on `canoa launch`'s first line.
    for entry in ["Claude", "Codex", "Pi", "Grok", "Antigravity"] {
        let connection = Connection::prepare(&offer, 0, entry, &host).expect("prepared");
        let given: BTreeMap<&str, &str> = connection
            .env()
            .iter()
            .map(|(name, value)| (name.as_str(), value.expose()))
            .collect();
        let line: Value = serde_json::from_slice(&connection.launch_line()).expect("a line");
        let launched: BTreeMap<&str, &str> = line["env"]
            .as_array()
            .expect("env")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_str().expect("name"),
                    pair[1].as_str().expect("value"),
                )
            })
            .collect();
        for (name, value) in &expected {
            assert_eq!(given.get(name), Some(&value.as_str()), "{entry}: {name}");
            assert_eq!(launched.get(name), Some(&value.as_str()), "{entry}: {name}");
        }
        let parts = parts_for(
            &connection,
            harness_of(None, entry),
            &Takes::default(),
            &host,
        );
        for (name, _) in &expected {
            let each = item(&parts.report, &format!("env.{name}"));
            assert_eq!(each["state"], "applied", "{entry}: {}", parts.report);
            assert_eq!(each["layer"], "overlay:team");
        }
        let summary = parts.report["summary"].as_str().expect("summary");
        assert!(!summary.contains("PATH"), "{entry}: {summary}");
    }
}

#[test]
fn each_line_not_applied_names_its_overlay_and_is_said_once() {
    let temp = temp_with_skill();
    for name in ["alpha", "beta"] {
        install_files(
            &temp,
            name,
            &[
                ("overlay.cfg", "[harness.codex]\nmodel = \"gpt-5\"\n"),
                ("CONTEXT.md", "x\n"),
            ],
        );
    }
    let config = config_with("overlays = [\"alpha\", \"beta\"]\n");
    let offer = offer_of(&temp, &config);
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, None, &Takes::default(), &lookup);
    let summary = parts.report["summary"].as_str().expect("summary");
    for name in ["alpha", "beta"] {
        let line = format!("{name}: harness.codex: hints for another harness (not supported)");
        assert_eq!(summary.matches(&line).count(), 1, "{summary}");
    }
    // An overlay the person's list names twice says each thing once.
    let twice = config_with("overlays = [\"alpha\", \"alpha\"]\n");
    let offer = offer_of(&temp, &twice);
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, None, &Takes::default(), &lookup);
    let summary = parts.report["summary"].as_str().expect("summary");
    assert_eq!(
        summary
            .matches("alpha: harness.codex: hints for another harness (not supported)")
            .count(),
        1,
        "{summary}"
    );
    let items = parts.report["items"].as_array().expect("items");
    let setups = items
        .iter()
        .filter(|item| {
            item["key"] == "harness.codex: hints for another harness"
                && item["layer"] == "overlay:alpha"
        })
        .count();
    assert_eq!(setups, 1, "{}", parts.report);
}

#[test]
fn what_an_overlay_needs_is_applied_when_this_computer_has_it_and_offered_from_the_store_when_it_does_not()
 {
    let temp = temp_with_skill();
    install_files(
        &temp,
        "needy",
        &[(
            "overlay.cfg",
            "[runtime]\ncommands = [\"probe\", \"canoa-test-absent-tool\"]\n\
             [[runtime.python]]\nversion = \"999.0\"\n\
             [[runtime.node]]\n",
        )],
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"needy\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, None, &Takes::default(), &lookup);
    let report = &parts.report;
    let found = item(report, "runtime.commands: probe");
    assert_eq!(found["state"], "applied", "{report}");
    assert_eq!(found["layer"], "overlay:needy");
    assert!(found.get("offer").is_none(), "{found}");
    // The stand-in node has no version to ask for, so a Node of no minimum is met by finding it.
    assert_eq!(item(report, "runtime.node")["state"], "applied", "{report}");

    let absent = item(report, "runtime.commands: canoa-test-absent-tool");
    assert_eq!(absent["state"], "off");
    assert!(
        absent.get("offer").is_none(),
        "no runtime provides it: {absent}"
    );
    assert!(
        absent["how"]
            .as_str()
            .expect("how")
            .contains("not on the PATH")
    );

    let python = item(report, "runtime.python");
    assert_eq!(python["state"], "off", "{report}");
    assert_eq!(python["offer"], "python_store");
    assert!(
        python["how"]
            .as_str()
            .expect("how")
            .contains("999.0 or later"),
        "{python}"
    );

    let summary = report["summary"].as_str().expect("summary");
    assert!(summary.contains("needy: runtime.python (off)"), "{summary}");
    assert!(
        summary.contains("needy: runtime.commands: canoa-test-absent-tool (off)"),
        "{summary}"
    );
    assert!(!summary.contains("runtime.node"), "{summary}");
    assert!(!summary.contains("runtime.commands: probe"), "{summary}");
    // The two found ones count as applied, and none is reported as a thing Canoa does not apply.
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["state"] != "not supported"
                || !item["key"].as_str().unwrap_or("").starts_with("runtime")),
        "{report}"
    );
}

#[test]
fn an_overlay_that_needs_nothing_reports_no_runtime_line() {
    let temp = temp_with_skill();
    install_files(&temp, "plain", &[("overlay.cfg", "name = \"plain\"\n")]);
    let offer = offer_of(&temp, &config_with("overlays = [\"plain\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let parts = parts_for(&connection, None, &Takes::default(), &lookup);
    assert!(
        parts.report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| !item["key"].as_str().unwrap_or("").starts_with("runtime")),
        "{}",
        parts.report
    );
}

/// Marks the overlay `name` as prepared at `digest` by hand: its record, its state directory and a packages folder for Python.
fn prepared_by_hand(temp: &Temp, name: &str, digest: &str) -> PathBuf {
    let packages = temp
        .0
        .join("overlays/.environments")
        .join(name)
        .join("python")
        .join(if cfg!(windows) {
            "Lib/site-packages"
        } else {
            "lib/python3.12/site-packages"
        });
    std::fs::create_dir_all(&packages).expect("packages");
    std::fs::create_dir_all(temp.0.join("overlays/.state").join(name)).expect("state");
    prepare::Home::new(temp.0.join("overlays"))
        .write_record(name, &aos::SetupRecord::new(digest, aos::Phase::Install))
        .expect("recorded");
    packages
}

#[test]
fn what_preparing_made_reaches_the_session_only_for_the_version_it_was_made_for() {
    let temp = temp_with_skill();
    install_files(
        &temp,
        "needy",
        &[
            ("overlay.cfg", "name = \"needy\"\n"),
            ("lib/python/requirements.txt", "canoa-test-pkg==1.0\n"),
            ("setup.py", "pass\n"),
        ],
    );
    let digest = digest_of(&temp, "overlays/needy");
    let offer = offer_of(&temp, &config_with("overlays = [\"needy\"]\n"));
    let report =
        |connection: &Connection| parts_for(connection, None, &Takes::default(), &lookup).report;

    // Not prepared: both are reported off with what to do, and only the overlay's own `lib/python` is on the path.
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let before = report(&connection);
    for key in [
        "dependencies: lib/python/requirements.txt",
        "setup: setup.py",
    ] {
        let each = item(&before, key);
        assert_eq!(each["state"], "off", "{key}: {before}");
        assert_eq!(each["layer"], "overlay:needy");
        assert!(
            each["how"]
                .as_str()
                .expect("how")
                .contains("install or sync the overlay")
        );
    }
    let own = env_value(&connection, "PYTHONPATH")
        .expect("PYTHONPATH")
        .to_owned();
    assert!(
        own.ends_with("lib/python") || own.ends_with("lib\\python"),
        "{own}"
    );
    assert!(env_value(&connection, &aos::state_variable("needy")).is_none());

    // Prepared at this digest: applied, the packages go after `lib/python`, and the state directory is told.
    let packages = prepared_by_hand(&temp, "needy", &digest);
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let after = report(&connection);
    for key in [
        "dependencies: lib/python/requirements.txt",
        "setup: setup.py",
    ] {
        assert_eq!(item(&after, key)["state"], "applied", "{key}: {after}");
    }
    assert!(
        item(&after, "setup: setup.py")["how"]
            .as_str()
            .expect("how")
            .contains("ran as install"),
        "{after}"
    );
    let path = env_value(&connection, "PYTHONPATH")
        .expect("PYTHONPATH")
        .to_owned();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let (head, tail) = path.split_once(sep).expect("two folders");
    assert_eq!(head, own);
    assert!(
        same_file_name(Path::new(tail), &packages),
        "{tail} is not {packages:?}"
    );
    assert!(
        env_value(&connection, &aos::state_variable("needy"))
            .is_some_and(|state| state.ends_with("needy")),
        "the state directory is `NAME_OVERLAY_STATE`"
    );

    // A preparation of another version counts for nothing when dependency digests also differ.
    prepared_by_hand(&temp, "needy", &format!("sha256:{}", "0".repeat(64)));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let stale = report(&connection);
    assert_eq!(item(&stale, "setup: setup.py")["state"], "off", "{stale}");
    assert_eq!(env_value(&connection, "PYTHONPATH"), Some(own.as_str()));

    // When dependency manifest digest is recorded and unchanged across a non-dependency edit, dependencies stay applied.
    let dep_digest = prepare::dependency_digest_files(
        &temp.0.join("overlays/needy"),
        "lib/python/requirements.txt",
        None,
    )
    .expect("dep digest");
    let mut deps = BTreeMap::new();
    deps.insert("python".to_owned(), dep_digest);
    prepare::Home::new(temp.0.join("overlays"))
        .write_record_with_deps(
            "needy",
            &aos::SetupRecord::new(format!("sha256:{}", "1".repeat(64)), aos::Phase::Install),
            &deps,
        )
        .expect("recorded with deps");
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let dep_kept = report(&connection);
    assert_eq!(
        item(&dep_kept, "dependencies: lib/python/requirements.txt")["state"],
        "applied",
        "{dep_kept}"
    );
    assert!(
        env_value(&connection, "PYTHONPATH")
            .expect("PYTHONPATH")
            .contains("site-packages")
    );
}

#[test]
fn the_commands_an_overlay_exposes_are_launchers_on_the_agents_path_and_two_of_one_name_are_not_exposed()
 {
    let temp = temp_with_skill();
    temp.write(&format!("bin/{}", program("python")), "");
    #[cfg(unix)]
    std::fs::set_permissions(
        temp.0.join("bin").join(program("python")),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("executable");
    install_files(
        &temp,
        "tools",
        &[
            (
                "overlay.cfg",
                "[scripts]\nhello = \"bin/hello.py\"\nlint = { path = \"bin/lint.sh\", interpreter = \"bash\" }\nold = false\n",
            ),
            ("bin/hello.py", "print('hi')\n"),
            ("bin/lint.sh", "true\n"),
        ],
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"tools\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let report = parts_for(&connection, None, &Takes::default(), &lookup).report;
    let hello = item(&report, "scripts.hello");
    assert_eq!(hello["state"], "applied", "{report}");
    assert!(
        hello["how"]
            .as_str()
            .expect("how")
            .contains("bin/hello.py under python"),
        "{hello}"
    );
    // `bash` is on neither the test's path nor, unless this computer's Store has Git Bash, the Store's runtimes.
    let store_has_bash = crate::sources::runtime_dirs()
        .iter()
        .any(|dir| dir.join("bash.exe").exists() || dir.join("bash").exists());
    if !store_has_bash {
        let lint = item(&report, "scripts.lint");
        assert_eq!(lint["state"], "off", "{report}");
        assert!(
            lint["how"]
                .as_str()
                .expect("how")
                .contains("is not on the PATH"),
            "{lint}"
        );
    }
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|each| each["key"] != "scripts.old")
    );
    // The launcher is in the folder that is first on the agent's `PATH`, ahead of the host's own.
    let commands = temp.0.join("overlays/.environments/tools/commands");
    let launcher = commands.join(if cfg!(windows) { "hello.cmd" } else { "hello" });
    let text = std::fs::read_to_string(&launcher).expect("a launcher");
    assert!(
        text.contains("hello.py") && text.contains("python"),
        "{text}"
    );
    assert!(
        store_has_bash || !commands.join("lint").exists() && !commands.join("lint.cmd").exists()
    );
    let path = env_value(&connection, "PATH").expect("PATH").to_owned();
    let first = std::env::split_paths(&path).next().expect("a folder");
    assert_eq!(first, commands);
    assert!(path.contains("bin"), "the host's own folders stay: {path}");

    // Two overlays exposing a name that differs only by case: neither is exposed.
    install_files(
        &temp,
        "others",
        &[
            ("overlay.cfg", "[scripts]\nHello = \"bin/hello.py\"\n"),
            ("bin/hello.py", "print('hello')\n"),
        ],
    );
    let offer = offer_of(&temp, &config_with("overlays = [\"tools\", \"others\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let report = parts_for(&connection, None, &Takes::default(), &lookup).report;
    for (key, layer) in [
        ("scripts.hello", "overlay:tools"),
        ("scripts.Hello", "overlay:others"),
    ] {
        let each = item(&report, key);
        assert_eq!(each["state"], "refused", "{report}");
        assert_eq!(each["layer"], layer);
        assert!(
            each["how"]
                .as_str()
                .expect("how")
                .contains("more than one overlay"),
            "{each}"
        );
    }
    // Launchers made for the last session stay on disk; only a folder `PATH` names counts, and none is named.
    let path = env_value(&connection, "PATH")
        .unwrap_or_default()
        .to_owned();
    assert!(
        std::env::split_paths(&path).all(|dir| dir != commands),
        "{path}"
    );
}

/// The value of `name` in a connection's environment.
fn env_value<'a>(connection: &'a Connection, name: &str) -> Option<&'a str> {
    connection
        .env()
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.expose())
}

/// A connection of the profile `main` over the overlay `team` made of `cfg`, started with `starting`.
fn session_connection(cfg: &str, starting: Starting<'_>) -> Connection {
    let temp = temp_with_skill();
    install_files(&temp, "team", &[("overlay.cfg", cfg)]);
    let offer = offer_of(&temp, &config_with("overlays = [\"team\"]\n"));
    Connection::prepare_with(&offer, 0, "a", &lookup, Choices::default(), starting)
        .expect("prepared")
}

#[test]
fn an_overlays_variables_that_change_how_programs_run_apply_for_every_harness() {
    let connection = session_connection(
        "[env]\nNODE_OPTIONS = \"--require x\"\nLD_PRELOAD = \"x.so\"\nHTTPS_PROXY = \"http://proxy.example\"\n\
         ANTHROPIC_BASE_URL = \"https://gateway.example\"\n",
        Starting::default(),
    );
    for (name, value) in [
        ("NODE_OPTIONS", "--require x"),
        ("LD_PRELOAD", "x.so"),
        ("HTTPS_PROXY", "http://proxy.example"),
        ("ANTHROPIC_BASE_URL", "https://gateway.example"),
    ] {
        assert_eq!(env_value(&connection, name), Some(value), "{name}");
    }
    // The same environment goes to every harness's session: the report names each variable as applied.
    for harness in [
        "claude-code",
        "codex",
        "pi",
        "antigravity",
        "grok",
        "opencode",
    ] {
        let parts = parts_for(&connection, Some(harness), &claude(), &lookup);
        for name in ["NODE_OPTIONS", "LD_PRELOAD", "HTTPS_PROXY"] {
            assert_eq!(
                item(&parts.report, &format!("env.{name}"))["state"],
                "applied",
                "{harness} {name}"
            );
        }
    }
}

#[test]
fn an_overlay_reads_the_identity_variables_of_every_harness_and_the_agent_is_given_them() {
    for (harness, vendor) in [
        ("claude-code", "anthropic"),
        ("codex", "openai"),
        ("antigravity", "google"),
        ("grok", "xai"),
    ] {
        let connection = session_connection(
            "[env]\nWHO = \"${AGENTS_HARNESS}/${AGENTS_VENDOR}/${AGENT}\"\n",
            Starting {
                who: Who {
                    harness: Some(harness),
                    ..Who::default()
                },
                host: Vec::new(),
                account: Vec::new(),
            },
        );
        assert_eq!(
            env_value(&connection, "WHO"),
            Some(format!("{harness}/{vendor}/{harness}").as_str())
        );
        assert_eq!(env_value(&connection, "AGENTS_HARNESS"), Some(harness));
        assert_eq!(env_value(&connection, "AGENTS_VENDOR"), Some(vendor));
        assert_eq!(env_value(&connection, "AGENT"), Some(harness));
        assert_eq!(env_value(&connection, "AGENTS_HOST"), Some("canoa"));
        assert_eq!(env_value(&connection, "AGENTS_PROFILE"), Some("main"));
    }
    // Pi and OpenCode have no vendor of their own, so none is set.
    for harness in ["pi", "opencode"] {
        let connection = session_connection(
            "[env]\nWHO = \"${AGENTS_HARNESS}\"\n",
            Starting {
                who: Who {
                    harness: Some(harness),
                    ..Who::default()
                },
                host: Vec::new(),
                account: Vec::new(),
            },
        );
        assert_eq!(env_value(&connection, "WHO"), Some(harness));
        assert_eq!(env_value(&connection, "AGENTS_VENDOR"), None);
        assert_eq!(env_value(&connection, "AGENT"), Some(harness));
    }
}

#[test]
fn a_source_named_opencode_is_the_opencode_harness() {
    assert_eq!(harness_identity(None, "OpenCode"), Some("opencode"));
    assert_eq!(harness_identity(None, "Grok"), Some("grok"));
    assert_eq!(harness_identity(None, "Elsewhere"), None);
}

#[test]
fn the_sessions_access_is_in_its_environment_and_an_overlay_reads_it() {
    let starting = || Starting {
        who: Who {
            harness: Some("codex"),
            ..Who::default()
        },
        host: Vec::new(),
        account: Vec::new(),
    };
    // Nothing in a layer above the built-in one: the standard access.
    let connection = session_connection(
        "[env]\nWHO = \"${AGENTS_SANDBOX}/${AGENTS_NETWORK}/${AGENTS_APPROVALS}\"\n",
        starting(),
    );
    assert_eq!(env_value(&connection, "WHO"), Some("none/true/ask"));
    assert_eq!(env_value(&connection, "AGENTS_SANDBOX"), Some("none"));
    // The person's layer sets some parts: those, and the standard for the rest. An overlay's own cannot (its file is refused).
    let temp = temp_with_skill();
    install_files(
        &temp,
        "team",
        &[(
            "overlay.cfg",
            "[env]
WHO = \"${AGENTS_SANDBOX}/${AGENTS_NETWORK}/${AGENTS_APPROVALS}\"
",
        )],
    );
    let offer = offer_of(
        &temp,
        &config_with(
            "overlays = [\"team\"]
access = { sandbox = \"workspace\", network = false }
",
        ),
    );
    let connection =
        Connection::prepare_with(&offer, 0, "a", &lookup, Choices::default(), starting())
            .expect("prepared");
    assert_eq!(env_value(&connection, "WHO"), Some("workspace/false/ask"));
    assert_eq!(env_value(&connection, "AGENTS_NETWORK"), Some("false"));
    assert_eq!(env_value(&connection, "AGENTS_APPROVALS"), Some("ask"));
}

#[test]
fn the_account_model_and_run_variables_are_set_only_when_they_have_a_value() {
    let connection = session_connection(
        "[env]\nSAME = \"${AGENTS_ACCOUNT}:${AGENTS_ACCOUNT_KIND}:${AGENTS_ACCOUNT_PROVIDER}:${AGENTS_MODEL}\"\n",
        Starting {
            who: Who {
                harness: Some("claude-code"),
                model: Some("sonnet"),
                effort: Some("high"),
                version: Some("2.1.0"),
                account: Some("work"),
                account_kind: Some("subscription"),
                account_provider: Some("anthropic"),
                role: Some("delegate"),
                ..Who::default()
            },
            host: Vec::new(),
            account: Vec::new(),
        },
    );
    assert_eq!(
        env_value(&connection, "SAME"),
        Some("work:subscription:anthropic:sonnet")
    );
    for (name, value) in [
        ("AGENTS_EFFORT", "high"),
        ("AGENTS_HARNESS_VERSION", "2.1.0"),
        ("AGENTS_ROLE", "delegate"),
        ("AGENTS_MODEL", "sonnet"),
    ] {
        assert_eq!(env_value(&connection, name), Some(value), "{name}");
    }
    // No account, no model, nothing else known: none of them is set, and none is ever empty.
    let bare = session_connection(
        "[env]\nWHO = \"${AGENTS_HARNESS}\"\n",
        Starting {
            who: Who {
                harness: Some("codex"),
                ..Who::default()
            },
            host: Vec::new(),
            account: Vec::new(),
        },
    );
    for name in [
        "AGENTS_ACCOUNT",
        "AGENTS_ACCOUNT_KIND",
        "AGENTS_ACCOUNT_PROVIDER",
        "AGENTS_MODEL",
        "AGENTS_EFFORT",
        "AGENTS_HARNESS_VERSION",
        "AGENTS_ROLE",
        "AGENTS_LOADOUT",
        "AGENTS_SESSION_ID",
        "AGENTS_PROJECT_ROOT",
    ] {
        assert_eq!(env_value(&bare, name), None, "{name}");
    }
    assert!(
        bare.env()
            .iter()
            .all(|(_, value)| !value.expose().is_empty())
    );
}

#[test]
fn an_overlays_own_value_replaces_an_identity_variable_and_the_hosts_variables_are_readable() {
    let connection = session_connection(
        "[env]\nAGENTS_MODEL = \"mine\"\nSTATE = \"${CODEX_HOME}/thing\"\nCODEX_HOME = \"${CODEX_HOME}/own\"\n",
        Starting {
            who: Who {
                harness: Some("codex"),
                model: Some("gpt"),
                ..Who::default()
            },
            host: vec![("CODEX_HOME".to_owned(), "/state".to_owned())],
            account: Vec::new(),
        },
    );
    assert_eq!(env_value(&connection, "AGENTS_MODEL"), Some("mine"));
    // Mapping order carries no meaning: each value reads the variable as it was before the overlay.
    assert_eq!(env_value(&connection, "STATE"), Some("/state/thing"));
    assert_eq!(env_value(&connection, "CODEX_HOME"), Some("/state/own"));
}

#[test]
fn an_overlay_naming_a_variable_the_specification_reserves_is_left_out() {
    let temp = temp_with_skill();
    install_files(&temp, "team", &[("env", "FOO_OVERLAY_ROOT=/x\n")]);
    let offer = offer_of(&temp, &config_with("overlays = [\"team\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    assert!(
        connection
            .notes
            .iter()
            .any(|item| item.key == "overlay" && item.state == State::Refused),
        "{:?}",
        connection.notes
    );
    assert_eq!(env_value(&connection, "FOO_OVERLAY_ROOT"), None);
}

#[test]
fn an_overlay_reads_any_variable_of_this_computer_by_placeholder_or_reference() {
    let connection = session_connection(
        "[env]\nCOPY = \"${HOST_SECRET}\"\nREF = { provider = \"env\", key = \"HOST_SECRET\" }\n",
        Starting::default(),
    );
    assert_eq!(env_value(&connection, "COPY"), Some(SECRET));
    assert_eq!(env_value(&connection, "REF"), Some(SECRET));
}

#[test]
fn a_profile_naming_share_env_is_refused() {
    let temp = temp_with_skill();
    let text = config_with("share_env = [\"HOST_SECRET\"]\n");
    assert!(Config::from_str_in(&text, &temp.0).is_err());
}

/// The environment and arguments `canoa launch` is given for `connection`.
fn launched_with(connection: &Connection) -> (BTreeMap<String, String>, Vec<String>) {
    let line: Value = serde_json::from_slice(&connection.launch_line()).expect("a line");
    let env = line["env"]
        .as_array()
        .expect("env")
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().expect("name").to_owned(),
                pair[1].as_str().expect("value").to_owned(),
            )
        })
        .collect();
    let args = line["args"]
        .as_array()
        .map(|args| {
            args.iter()
                .map(|arg| arg.as_str().expect("text").to_owned())
                .collect()
        })
        .unwrap_or_default();
    (env, args)
}

fn access_connection(harness: Option<&'static str>, extra: &str) -> Connection {
    let temp = temp_with_skill();
    let offer = offer_of(&temp, &config_with(extra));
    Connection::prepare_with(
        &offer,
        0,
        "a",
        &lookup,
        Choices::default(),
        Starting {
            who: Who {
                harness,
                ..Who::default()
            },
            host: Vec::new(),
            account: Vec::new(),
        },
    )
    .expect("prepared")
}

fn access_item(parts: &Parts, key: &str) -> Option<Value> {
    parts.report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .cloned()
}

#[test]
fn grok_is_started_with_the_access_as_its_sandbox_variable_and_always_approve() {
    let standard = access_connection(Some("grok"), "");
    let (env, args) = launched_with(&standard);
    assert_eq!(env.get("GROK_SANDBOX").map(String::as_str), Some("off"));
    assert!(args.is_empty());
    let parts = parts_for(&standard, Some("grok"), &Takes::default(), &lookup);
    assert!(parts.agent_meta.is_empty());
    assert!(access_item(&parts, "access.sandbox").is_none());

    let connection = access_connection(
        Some("grok"),
        "access = { sandbox = \"workspace\", approvals = \"never\" }\n",
    );
    let (env, args) = launched_with(&connection);
    assert_eq!(
        env.get("GROK_SANDBOX").map(String::as_str),
        Some("workspace")
    );
    assert!(args.is_empty(), "Grok's options go before its mode word");
    let parts = parts_for(&connection, Some("grok"), &Takes::default(), &lookup);
    assert_eq!(parts.agent_meta.get("yoloMode"), Some(&Value::Bool(true)));
    for key in ["access.sandbox", "access.approvals"] {
        let each = access_item(&parts, key).expect(key);
        assert_eq!(each["state"], "applied", "{key}: {each}");
    }
    // The session request carries it in `_meta`, beside the access.
    let mut params = Map::new();
    add_parts(&mut params, &parts).expect("added");
    assert_eq!(params["_meta"]["yoloMode"], true, "{params:?}");
    assert_eq!(
        params["_meta"][coqui_acp_harness::ACCESS_META]["sandbox"],
        "workspace"
    );

    let edits = access_connection(Some("grok"), "access = { approvals = \"edits\" }\n");
    let parts = parts_for(&edits, Some("grok"), &Takes::default(), &lookup);
    assert!(parts.agent_meta.is_empty());
    let each = access_item(&parts, "access.approvals").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
}

#[test]
fn opencode_is_started_with_the_access_as_its_permission_variable() {
    let permission = |connection: &Connection| {
        let (env, args) = launched_with(connection);
        assert!(args.is_empty(), "OpenCode takes no argument for it");
        env.get("OPENCODE_PERMISSION").cloned()
    };
    let standard = access_connection(Some("opencode"), "");
    assert_eq!(
        permission(&standard).as_deref(),
        Some(r#"{"edit":"ask","bash":"ask"}"#)
    );
    let parts = parts_for(&standard, Some("opencode"), &Takes::default(), &lookup);
    assert!(parts.agent_meta.is_empty());
    assert!(access_item(&parts, "access.sandbox").is_none());

    let never = access_connection(Some("opencode"), "access = { approvals = \"never\" }\n");
    let allowed: Value = serde_json::from_str(&permission(&never).expect("set")).expect("JSON");
    assert_eq!(allowed["bash"], "allow");
    assert_eq!(allowed["external_directory"], "allow");
    let parts = parts_for(&never, Some("opencode"), &Takes::default(), &lookup);
    let each = access_item(&parts, "access.approvals").expect("reported");
    assert_eq!(each["state"], "applied", "{each}");

    let read_only = access_connection(
        Some("opencode"),
        "access = { sandbox = \"read-only\", approvals = \"never\" }\n",
    );
    assert_eq!(
        permission(&read_only).as_deref(),
        Some(r#"{"edit":"deny"}"#)
    );
    let parts = parts_for(&read_only, Some("opencode"), &Takes::default(), &lookup);
    let sandbox = access_item(&parts, "access.sandbox").expect("reported");
    assert_eq!(sandbox["state"], "applied", "{sandbox}");
    let approvals = access_item(&parts, "access.approvals").expect("reported");
    assert_eq!(approvals["state"], "not supported", "{approvals}");

    let workspace = access_connection(Some("opencode"), "access = { sandbox = \"workspace\" }\n");
    let parts = parts_for(&workspace, Some("opencode"), &Takes::default(), &lookup);
    let each = access_item(&parts, "access.sandbox").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
    let closed = access_connection(Some("opencode"), "access = { network = false }\n");
    let parts = parts_for(&closed, Some("opencode"), &Takes::default(), &lookup);
    let each = access_item(&parts, "access.network").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
}

#[test]
fn antigravity_is_started_with_the_access_as_arguments() {
    let standard = access_connection(Some("antigravity"), "");
    let (env, args) = launched_with(&standard);
    assert!(args.is_empty() && !env.contains_key("GROK_SANDBOX"));
    for (access, expected) in [
        ("{ sandbox = \"workspace\" }", vec!["--sandbox"]),
        (
            "{ sandbox = \"read-only\", approvals = \"never\" }",
            vec!["--sandbox", "--dangerously-skip-permissions"],
        ),
        (
            "{ approvals = \"never\" }",
            vec!["--dangerously-skip-permissions"],
        ),
    ] {
        let connection = access_connection(Some("antigravity"), &format!("access = {access}\n"));
        let (env, args) = launched_with(&connection);
        assert_eq!(args, expected, "{access}");
        assert!(!env.contains_key("GROK_SANDBOX"), "{access}");
        let parts = parts_for(&connection, Some("antigravity"), &Takes::default(), &lookup);
        assert!(parts.agent_meta.is_empty());
        for key in ["access.sandbox", "access.approvals"] {
            if let Some(each) = access_item(&parts, key) {
                assert_eq!(each["state"], "applied", "{access}: {each}");
            }
        }
    }
    let closed = access_connection(Some("antigravity"), "access = { network = false }\n");
    let parts = parts_for(&closed, Some("antigravity"), &Takes::default(), &lookup);
    let each = access_item(&parts, "access.network").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
}

#[test]
fn only_a_program_the_host_started_with_the_access_says_it_applied_it() {
    // Claude Code, Codex and Pi take the access in the session's request, so the launch line carries none of it.
    for harness in ["claude-code", "codex", "pi"] {
        let connection = access_connection(
            Some(harness),
            "access = { sandbox = \"workspace\", approvals = \"never\" }\n",
        );
        let (env, args) = launched_with(&connection);
        assert!(
            args.is_empty() && !env.contains_key("GROK_SANDBOX"),
            "{harness}"
        );
    }
    // An agent that is only reported as Antigravity, from an entry that is not, was started as its entry says.
    let unknown = access_connection(None, "access = { sandbox = \"workspace\" }\n");
    let (_, args) = launched_with(&unknown);
    assert!(args.is_empty());
    let parts = parts_with(
        &unknown,
        Some("antigravity"),
        Some("Antigravity ACP Server"),
        &Takes::default(),
        &lookup,
    );
    let each = access_item(&parts, "access.sandbox").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
    assert!(
        each["how"]
            .as_str()
            .expect("how")
            .contains("started as its entry says"),
        "{each}"
    );
}

#[test]
fn a_variable_a_layer_sets_is_kept_and_the_report_says_the_access_did_not_set_it() {
    let connection = access_connection(
        Some("grok"),
        "access = { sandbox = \"read-only\" }\n[host.profiles.main.env.GROK_SANDBOX]\nvalue = \"strict\"\n",
    );
    let (env, _) = launched_with(&connection);
    assert_eq!(env.get("GROK_SANDBOX").map(String::as_str), Some("strict"));
    let parts = parts_for(&connection, Some("grok"), &Takes::default(), &lookup);
    let each = access_item(&parts, "access.env.GROK_SANDBOX").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
}

#[test]
fn a_layer_reads_the_variables_the_account_sets_before_the_hosts_own() {
    let connection = session_connection(
        "[env]\nSTATE = \"${CODEX_HOME}/x\"\nURL = \"${ANTHROPIC_BASE_URL}/v1\"\n",
        Starting {
            who: Who {
                harness: Some("codex"),
                ..Who::default()
            },
            host: vec![("CODEX_HOME".to_owned(), "/store".to_owned())],
            account: vec![
                ("CODEX_HOME".to_owned(), "/accounts/work/state".to_owned()),
                (
                    "ANTHROPIC_BASE_URL".to_owned(),
                    "https://gateway.example".to_owned(),
                ),
            ],
        },
    );
    assert_eq!(
        env_value(&connection, "STATE"),
        Some("/accounts/work/state/x")
    );
    assert_eq!(
        env_value(&connection, "URL"),
        Some("https://gateway.example/v1")
    );
    // What the host or the account sets is read, not handed on a second time.
    assert_eq!(env_value(&connection, "ANTHROPIC_BASE_URL"), None);
}

#[test]
fn a_layer_that_sets_what_the_account_sets_is_told_the_account_wins() {
    let temp = temp_with_skill();
    let offer = offer_of(
        &temp,
        &config_with(
            "[host.profiles.main.env.ANTHROPIC_BASE_URL]\nvalue = \"https://layer.example\"\n",
        ),
    );
    let connection = Connection::prepare_with(
        &offer,
        0,
        "a",
        &lookup,
        Choices::default(),
        Starting {
            who: Who {
                harness: Some("claude-code"),
                ..Who::default()
            },
            host: Vec::new(),
            account: vec![(
                "ANTHROPIC_BASE_URL".to_owned(),
                "https://gateway.example".to_owned(),
            )],
        },
    )
    .expect("prepared");
    let parts = parts_for(&connection, Some("claude-code"), &Takes::default(), &lookup);
    let each = access_item(&parts, "env.ANTHROPIC_BASE_URL").expect("reported");
    assert_eq!(each["state"], "not supported", "{each}");
    assert!(
        each["how"].as_str().expect("how").contains("the account"),
        "{each}"
    );
    // Another variable of the layer is still applied.
    let token = access_item(&parts, "env.TOKEN").expect("reported");
    assert_eq!(token["state"], "applied", "{token}");
}

#[test]
#[ignore = "needs CANOA_TEST_NODE: a Node.js with npm beside it"]
fn what_a_real_npm_installed_is_on_a_sessions_node_path_after_the_overlays_lib_node() {
    use crate::profile::npm_registry::{Registry, finder_for, node, npm_env};
    let node = node().expect("CANOA_TEST_NODE names a Node.js with npm beside it");
    let temp = temp_with_skill();
    install_files(
        &temp,
        "needy",
        &[
            ("overlay.cfg", "name = \"needy\"\n"),
            (
                "lib/node/package.json",
                "{\"dependencies\": {\"canoa-test-pkg\": \"1.0.0\"}}\n",
            ),
        ],
    );
    let digest = digest_of(&temp, "overlays/needy");
    let root = temp.0.join("overlays/needy");
    let plan = prepare::Plan::of("needy", &root, aos::current_os(), &digest).expect("a plan");
    let registry = Registry::start(
        &node,
        &temp.0.join("registry"),
        &[("canoa-test-pkg", "1.0.0")],
    );
    let env = npm_env(&node, &temp.0.join("npm"), Some(&registry.url()));
    let home = prepare::Home::new(temp.0.join("overlays"));
    prepare::run(&plan, aos::Change::Add, &home, &finder_for(&node), &env).expect("prepared");

    let offer = offer_of(&temp, &config_with("overlays = [\"needy\"]\n"));
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    let path = env_value(&connection, "NODE_PATH")
        .expect("NODE_PATH")
        .to_owned();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let (head, tail) = path.split_once(sep).expect("two folders");
    assert!(
        head.ends_with("lib/node") || head.ends_with("lib\\node"),
        "{path}"
    );
    assert!(
        same_file_name(Path::new(tail), Path::new(&home.packages("needy")["node"])),
        "{path}"
    );
    // The program a session runs finds the package by that path.
    let printed = std::process::Command::new(&node)
        .args(["-e", "console.log(require('canoa-test-pkg').version)"])
        .env("NODE_PATH", &path)
        .output()
        .expect("node starts");
    assert_eq!(
        String::from_utf8_lossy(&printed.stdout).trim(),
        "1.0.0",
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
}

const WITH_DEFAULT: &str = "[profile]\nmodel = \"mine\"\n\
    [profiles.default]\nmodel = \"calm\"\ncontext = \"Be brief.\"\n\
    [profiles.deep]\nmodel = \"big\"\n";

#[test]
fn a_session_that_names_no_profile_starts_with_the_default_profile() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, WITH_DEFAULT);
    let (_, report) = preset_report(&offer, None);
    assert_eq!(item(&report, "model")["value"], "calm", "{report}");
    assert_eq!(item(&report, "model")["layer"], "session", "{report}");
    let context = items_of(&report, "context", "session");
    assert_eq!(context.len(), 1, "{report}");
    // Naming the default is the same as naming none, and another profile replaces it whole, as the default is no layer under it.
    let (_, named) = preset_report(&offer, Some("default"));
    assert_eq!(item(&named, "model")["value"], "calm", "{named}");
    let (_, other) = preset_report(&offer, Some("deep"));
    assert_eq!(item(&other, "model")["value"], "big", "{other}");
    assert!(items_of(&other, "context", "session").is_empty(), "{other}");
}

#[test]
fn a_file_with_no_default_profile_still_lets_a_session_name_it() {
    let temp = temp_with_skill();
    let offer = loadout_offer(&temp, PRESETS);
    let first = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"_meta": {PRESET_META: "default"}}}))
    .expect("json");
    assert_eq!(
        choose_preset(&offer, 0, &first).expect("held"),
        Some("default".to_owned())
    );
    let (_, report) = preset_report(&offer, Some("default"));
    assert_eq!(item(&report, "model")["value"], "mine", "{report}");
}

/// The person's own layer for agent `a` from `text`, offered with the items of `catalog`.
fn offer_with_items(temp: &Temp, text: &str, catalog: crate::profile::items::ItemCatalog) -> Offer {
    let config = Config::from_str_in(&format!("[agents.a]\nprogram = \"agent\"\n{text}"), &temp.0)
        .expect("parses");
    let mut local = config.local_profiles.for_agent("a").expect("a profile");
    local.items = crate::profile::ItemSet::with(catalog);
    Arc::into_inner(
        offer(
            &[],
            &IndexMap::new(),
            Some(&local),
            "launcher".into(),
            Vec::new(),
        )
        .expect("an offer"),
    )
    .expect("one owner")
}

/// A session of `offer` on Claude Code that names `preset`, and the report it starts with.
fn item_report(offer: &Offer, preset: Option<&str>) -> (Connection, Value) {
    let connection = Connection::prepare_with(
        offer,
        0,
        "a",
        &lookup,
        Choices {
            preset,
            ..Choices::default()
        },
        Starting {
            who: crate::profile::session::Who {
                harness: Some("claude-code"),
                ..Default::default()
            },
            ..Starting::default()
        },
    )
    .expect("prepared");
    let report = parts_for(&connection, Some("claude-code"), &claude(), &lookup).report;
    (connection, report)
}

fn var<'a>(connection: &'a Connection, name: &str) -> Option<&'a str> {
    connection
        .env()
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.expose())
}

const REGIONS: &str = "[profile]\nmodel = \"mine\"\n\
    [profiles.north]\nmodel = \"big\"\n\
    [profiles.north.items.region]\nname = \"north\"\nhosts = [\"a.example\", \"b.example\"]\ntier = \"fast\"\n";

#[test]
fn an_item_an_extension_adds_changes_the_session_as_its_provider_says() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        REGIONS,
        crate::profile::items::standin::with_extension(),
    );
    let (connection, report) = item_report(&offer, Some("north"));
    assert_eq!(var(&connection, "REGION"), Some("north"));
    assert_eq!(
        var(&connection, "REGION_HOSTS"),
        Some("a.example,b.example")
    );
    assert_eq!(
        item(&report, "items.region")["state"],
        "applied",
        "{report}"
    );
    assert_eq!(
        item(&report, "items.region")["layer"],
        "session",
        "{report}"
    );
    let how = item(&report, "items.region")["how"].as_str().expect("text");
    assert!(how.contains("REGION"), "{how}");
    assert!(
        !how.contains("a.example"),
        "no value is in the report: {how}"
    );
    // The provider's setting is the session's own: it came after the profile's own model.
    assert_eq!(item(&report, "model")["value"], "fast-model", "{report}");
    assert_eq!(item(&report, "model")["layer"], "session", "{report}");
    // A profile that does not set the item gives the session none of it.
    let (other, report) = item_report(&offer, None);
    assert_eq!(var(&other, "REGION"), None);
    assert!(
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|each| each["key"] != "items.region"),
        "{report}"
    );
}

#[test]
fn the_default_profiles_items_apply_to_a_session_that_names_none() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        "[profiles.default.items.region]\nname = \"south\"\n",
        crate::profile::items::standin::with_extension(),
    );
    let (connection, report) = item_report(&offer, None);
    assert_eq!(var(&connection, "REGION"), Some("south"));
    assert_eq!(
        item(&report, "items.region")["state"],
        "applied",
        "{report}"
    );
}

#[test]
fn an_item_for_another_harness_is_not_applied_and_says_so() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        REGIONS,
        crate::profile::items::standin::with_extension(),
    );
    // `preset_report` starts a session that knows no harness.
    let (connection, report) = preset_report(&offer, Some("north"));
    assert_eq!(var(&connection, "REGION"), None);
    assert_eq!(
        item(&report, "items.region")["state"],
        "not supported",
        "{report}"
    );
    assert!(
        item(&report, "items.region")["how"]
            .as_str()
            .expect("text")
            .contains("claude-code only"),
        "{report}"
    );
}

#[test]
fn an_extension_taken_away_leaves_the_profile_its_values_and_the_session_without_them() {
    let temp = temp_with_skill();
    // With the extension.
    let with = offer_with_items(
        &temp,
        REGIONS,
        crate::profile::items::standin::with_extension(),
    );
    assert_eq!(
        var(&item_report(&with, Some("north")).0, "REGION"),
        Some("north")
    );
    // Without it, on the same file.
    let without = offer_with_items(
        &temp,
        REGIONS,
        crate::profile::items::ItemCatalog::from_registries(Vec::new()),
    );
    let (connection, report) = item_report(&without, Some("north"));
    assert_eq!(var(&connection, "REGION"), None);
    let line = item(&report, "items.region");
    assert_eq!(line["state"], "not supported", "{report}");
    assert!(
        line["how"]
            .as_str()
            .expect("text")
            .contains("kept and not applied"),
        "{line}"
    );
    assert!(
        without.profiles[0].1.presets["north"]
            .items
            .contains_key("region"),
        "the profile still holds its values"
    );
    // The rest of the profile applies as it did.
    assert_eq!(item(&report, "model")["value"], "big", "{report}");
}

#[test]
fn a_variable_the_profiles_own_environment_sets_keeps_its_value() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        &format!("{REGIONS}[profile.env.REGION]\nvalue = \"mine\"\n"),
        crate::profile::items::standin::with_extension(),
    );
    let (connection, report) = item_report(&offer, Some("north"));
    assert_eq!(var(&connection, "REGION"), Some("mine"));
    assert_eq!(
        var(&connection, "REGION_HOSTS"),
        Some("a.example,b.example")
    );
    assert_eq!(
        item(&report, "items.region.env.REGION")["state"],
        "not supported",
        "{report}"
    );
}

#[test]
fn the_bundled_identity_item_gives_the_sessions_programs_the_name_git_commits_under() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        "[profiles.default.items.identity]\nname = \"Ana Ruiz\"\nemail = \"ana@example.test\"\n",
        crate::profile::items::ItemCatalog::from_registries(Vec::new()),
    );
    let (connection, report) = item_report(&offer, None);
    assert_eq!(var(&connection, "GIT_AUTHOR_NAME"), Some("Ana Ruiz"));
    assert_eq!(
        var(&connection, "GIT_COMMITTER_EMAIL"),
        Some("ana@example.test")
    );
    let how = item(&report, "items.identity")["how"]
        .as_str()
        .expect("text");
    assert!(
        how.contains("GIT_AUTHOR_NAME") && !how.contains("Ana"),
        "{how}"
    );
}

#[test]
fn the_bundled_proxy_item_gives_the_sessions_programs_the_network_its_mode_names() {
    let temp = temp_with_skill();
    let offer_for = |items: &str| {
        offer_with_items(
            &temp,
            &format!("[profiles.default.items.proxy]\n{items}"),
            crate::profile::items::ItemCatalog::from_registries(Vec::new()),
        )
    };
    // Its own proxy: the programs are given it as it is.
    let own = offer_for("mode = \"proxy\"\nurl = \"http://gateway.example:3128\"\n");
    let (connection, report) = item_report(&own, None);
    assert_eq!(
        var(&connection, "HTTPS_PROXY"),
        Some("http://gateway.example:3128")
    );
    assert_eq!(
        item(&report, "items.proxy.network")["state"],
        "applied",
        "{report}"
    );
    // None: no proxy, which the host's own starter takes for no system proxy to add.
    let none = offer_for("mode = \"none\"\n");
    let (connection, report) = item_report(&none, None);
    assert_eq!(var(&connection, "HTTPS_PROXY"), None);
    assert_eq!(var(&connection, "NO_PROXY"), Some("*"));
    assert_eq!(
        item(&report, "items.proxy.network")["state"],
        "applied",
        "{report}"
    );
    // The system's, the default: a proxy the host runs for the session, which stops with it.
    let system = offer_for("");
    let (connection, report) = item_report(&system, None);
    let url = var(&connection, "HTTPS_PROXY").expect("a proxy").to_owned();
    assert!(url.starts_with("http://canoa:"), "{url}");
    let how = item(&report, "items.proxy.network")["how"]
        .as_str()
        .expect("text");
    assert!(
        how.contains("following the system") && !how.contains("canoa:"),
        "{how}"
    );
    let address: core::net::SocketAddr = url
        .rsplit_once('@')
        .and_then(|(_, address)| address.parse().ok())
        .expect("an address");
    assert!(
        std::net::TcpStream::connect_timeout(&address, core::time::Duration::from_secs(2)).is_ok()
    );
    drop(connection);
    assert!(
        std::net::TcpStream::connect_timeout(&address, core::time::Duration::from_millis(500))
            .is_err(),
        "the proxy ends with the session"
    );
    // A proxy the profile's own environment names is kept.
    let named = offer_with_items(
        &temp,
        "[profile.env.HTTPS_PROXY]\nvalue = \"http://mine.example:1\"\n[profiles.default.items.proxy]\nmode = \"proxy\"\nurl = \"http://gateway.example:3128\"\n",
        crate::profile::items::ItemCatalog::from_registries(Vec::new()),
    );
    let (connection, _) = item_report(&named, None);
    assert_eq!(
        var(&connection, "HTTPS_PROXY"),
        Some("http://mine.example:1")
    );
}

#[test]
fn the_proxy_item_ends_tls_for_the_hosts_it_names_and_the_authoritys_key_is_not_in_the_session() {
    let temp = temp_with_skill();
    let offer_for = |items: &str| {
        offer_with_items(
            &temp,
            &format!("[profiles.default.items.proxy]\n{items}"),
            crate::profile::items::ItemCatalog::from_registries(Vec::new()),
        )
    };
    let named = offer_for(concat!(
        "client_certificate_hosts = [{ pattern = \"api.example.test\" },",
        " { pattern = \"(db|cache)[.]corp[.]example\", identity = \"system\" }]\n"
    ));
    let (connection, report) = item_report(&named, None);
    let bundle = var(&connection, "SSL_CERT_FILE")
        .expect("a bundle")
        .to_owned();
    assert!(
        std::path::Path::new(&bundle).starts_with(connection.skills()),
        "{bundle} is in the connection's private directory"
    );
    let text = std::fs::read_to_string(&bundle).expect("written");
    assert!(text.contains("BEGIN CERTIFICATE") && !text.contains("PRIVATE KEY"));
    assert!(var(&connection, "NODE_EXTRA_CA_CERTS").is_some());
    let how = item(&report, "items.proxy.network")["how"]
        .as_str()
        .expect("text");
    assert!(
        how.contains("api.example.test") && how.contains("(db|cache)[.]corp[.]example"),
        "{how}"
    );
    assert!(
        !report.to_string().contains("PRIVATE KEY"),
        "no key in the report"
    );
    // With no proxy of the host's own to end TLS in, the profile is told so and nothing is ended.
    let wrong = offer_for(
        "mode = \"none\"\nclient_certificate_hosts = [{ pattern = \"api.example.test\" }]\n",
    );
    let (connection, report) = item_report(&wrong, None);
    assert_eq!(var(&connection, "SSL_CERT_FILE"), None);
    assert_eq!(
        item(&report, "items.proxy")["state"],
        "not supported",
        "{report}"
    );
    assert!(
        item(&report, "items.proxy")["how"]
            .as_str()
            .is_some_and(|how| how.contains("system or pac")),
        "{report}"
    );
}

#[test]
fn a_network_that_cannot_be_made_is_refused_in_the_report_and_the_session_still_starts() {
    let temp = temp_with_skill();
    let offer = offer_with_items(
        &temp,
        "[profiles.default.items.proxy]\nmode = \"command\"\ncommand = [\"/no/such/program\"]\n",
        crate::profile::items::ItemCatalog::from_registries(Vec::new()),
    );
    let (connection, report) = item_report(&offer, None);
    assert_eq!(var(&connection, "HTTPS_PROXY"), None);
    assert_eq!(
        item(&report, "items.proxy.network")["state"],
        "refused",
        "{report}"
    );
}

#[test]
fn a_session_requests_meta_is_merged_with_what_an_agent_adds_key_by_key_and_list_by_list() {
    let mut meta = json!({"a": {"list": [1], "keep": "mine"}, "b": 1});
    merge_json(
        &mut meta,
        &json!({"a": {"list": [2], "keep": "theirs", "new": true}, "c": 2}),
    );
    assert_eq!(
        meta,
        json!({"a": {"list": [1, 2], "keep": "mine", "new": true}, "b": 1, "c": 2})
    );
}

#[test]
fn an_agents_plugin_meta_names_the_folder_it_was_written_to() {
    let dir = std::env::temp_dir().join(format!("canoa-plugin-meta-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    let written = Written(dir.clone());
    let plugin = canoa::vendor::PluginDelivery::new(
        "inner/plug",
        guatiao_serde::from_serde(
            json!({"x": {"plugins": [{"path": "{path}"}, {"path": "elsewhere"}]}}),
        )
        .expect("value"),
    );
    let meta = plugin_meta(&plugin, &written).expect("a meta");
    let path = meta["x"]["plugins"][0]["path"].as_str().expect("text");
    assert!(path.ends_with("plug") && path.contains("inner"), "{path}");
    assert_eq!(meta["x"]["plugins"][1]["path"], "elsewhere");
    let own = canoa::vendor::PluginDelivery::new(
        "",
        guatiao_serde::from_serde(json!({"p": "{path}"})).expect("value"),
    );
    assert_eq!(
        plugin_meta(&own, &written).expect("a meta")["p"],
        dir.to_string_lossy().as_ref()
    );
    assert!(
        plugin_meta(
            &canoa::vendor::PluginDelivery::new(
                "",
                guatiao_serde::from_serde(json!([])).expect("value")
            ),
            &written
        )
        .is_err(),
        "not an object"
    );
    drop(written);
}

#[test]
fn antigravity_overlay_context_applies_across_system_prompt_instruction_file_and_briefing() {
    let temp = temp_with_skill();
    temp.write(
        "overlays/team/OVERLAY.md",
        "---\nname: team\ndescription: Team\n---\nUse overlay rule alpha.\n",
    );
    temp.write("overlays/team/rules/guide.md", "Follow overlay guide.\n");
    let digest = digest_of(&temp, "overlays/team");
    temp.write(
        "overlays/team.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    let cfg = "[host.agents.Antigravity]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
               [host.profiles.main]\noverlays = [\"team\"]\n";
    let offer = offer_of(&temp, cfg);
    let connection = Connection::prepare(&offer, 0, "Antigravity", &lookup).expect("prepared");

    // 1. Real agy capability shape (`_meta.antigravity.systemPrompt`): delivered via ACP systemPrompt.
    let agy_init = antigravity_initialize();
    let (agy_takes, agy_reported) = described(&agy_init);
    let agy_parts = parts_with(
        &connection,
        harness_of(agy_reported, "Antigravity"),
        agy_reported,
        &agy_takes,
        &lookup,
    );
    assert!(
        agy_parts.system_prompt.as_deref().is_some_and(
            |p| p.contains("Use overlay rule alpha.") && p.contains("Follow overlay guide.")
        ),
        "{agy_parts:?}"
    );
    assert_eq!(item(&agy_parts.report, "context")["state"], "applied");

    // 2. Without systemPrompt, with additionalDirectories: written as `AGENTS.md`.
    let dir_takes = Takes {
        version: 1,
        capabilities: json!({"sessionCapabilities": {"additionalDirectories": {}}}),
    };
    let dir_parts = parts_for(&connection, Some("antigravity"), &dir_takes, &lookup);
    assert!(dir_parts.system_prompt.is_none());
    assert_eq!(item(&dir_parts.report, "context")["state"], "applied");
    let dir = PathBuf::from(&dir_parts.directories[0]);
    let agents_md = std::fs::read_to_string(dir.join("AGENTS.md")).expect("written");
    assert!(agents_md.contains("Use overlay rule alpha."));

    // 3. Without systemPrompt or additionalDirectories: prepended to first session/prompt turn once.
    let bare_takes = Takes::default();
    let bare_parts = parts_for(&connection, Some("antigravity"), &bare_takes, &lookup);
    assert_eq!(item(&bare_parts.report, "context")["state"], "applied");
    assert!(!bare_parts.briefing_pieces.is_empty());

    struct DummyChannel;
    impl coqui_transport::Channel for DummyChannel {
        fn send(&self, _: &[u8]) -> Result<(), coqui_transport::Failure> {
            Ok(())
        }
        fn receive(
            &self,
            _: Option<core::time::Duration>,
        ) -> Result<Option<Vec<u8>>, coqui_transport::Failure> {
            Ok(None)
        }
        fn close(&self) {}
        fn set_max_message(&self, _: usize) {}
    }
    let dummy = DummyChannel;
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&dummy, &scope, &connection);
    hosted.hold().parts = Some(bare_parts);
    let _ = hosted
        .forward(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "session/new",
                "params": {"cwd": "/work/proj"}
            }))
            .expect("json"),
            Vec::new(),
        )
        .expect("forwarded");
    // Simulate agent sessionId response in Hold.
    hosted
        .hold()
        .session_cwds
        .insert("sess-1".to_owned(), Some("/work/proj".to_owned()));
    let first = hosted
        .forward(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/prompt",
                "params": {"sessionId": "sess-1", "prompt": [{"type": "text", "text": "Hello"}]}
            }))
            .expect("json"),
            Vec::new(),
        )
        .expect("forwarded");
    let first_val: Value = serde_json::from_slice(&first).expect("json");
    let blocks = first_val["params"]["prompt"]
        .as_array()
        .expect("prompt array");
    assert_eq!(blocks.len(), 2);
    assert!(
        blocks[0]["text"]
            .as_str()
            .unwrap()
            .contains("Use overlay rule alpha.")
    );
    assert_eq!(blocks[1]["text"], "Hello");

    // Second prompt on the same session must not repeat the briefing.
    let second = hosted
        .forward(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": {"sessionId": "sess-1", "prompt": [{"type": "text", "text": "Next"}]}
            }))
            .expect("json"),
            Vec::new(),
        )
        .expect("forwarded");
    let second_val: Value = serde_json::from_slice(&second).expect("json");
    assert_eq!(
        second_val["params"]["prompt"]
            .as_array()
            .expect("prompt")
            .len(),
        1
    );
}

#[test]
fn session_environment_exports_agents_python_agents_home_and_overlay_root() {
    let temp = temp_with_skill();
    temp.write(&format!("bin/{}", program("python")), "");
    #[cfg(unix)]
    std::fs::set_permissions(
        temp.0.join(format!("bin/{}", program("python"))),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("executable");
    temp.write(
        "overlays/duho/overlay.cfg",
        "name = \"duho\"\ndescription = \"Tools\"\n[scripts]\nduho-cli = \"bin/tool.py\"\n",
    );
    temp.write("overlays/duho/bin/tool.py", "print('help')\n");
    let digest = digest_of(&temp, "overlays/duho");
    temp.write(
        "overlays/duho.json",
        &json!({"source": "x", "commit": "y", "digest": digest, "files": {}}).to_string(),
    );
    let cfg = "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
               [host.profiles.main]\noverlays = [\"duho\"]\n";
    let offer = offer_of(&temp, cfg);
    let connection = Connection::prepare(&offer, 0, "a", &lookup).expect("prepared");
    assert!(var(&connection, "AGENTS_PYTHON").is_some());
    assert!(var(&connection, "AGENTS_HOME").is_some());
    assert_eq!(
        var(&connection, "DUHO_OVERLAY_ROOT"),
        Some(
            temp.0
                .join("overlays")
                .join("duho")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn mcp_skills_deduplicate_between_scripts_mode_and_mcp_only_mode() {
    let temp = temp_with_skill();
    temp.write(&format!("bin/{}", program("python")), "");
    #[cfg(unix)]
    std::fs::set_permissions(
        temp.0.join(format!("bin/{}", program("python"))),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("executable");
    temp.write(
        "overlays/duho/overlay.cfg",
        "name = \"duho\"\ndescription = \"Tools\"\n\
         [scripts]\nduho-cli = \"bin/tool.py\"\n\
         [[mcp.servers]]\nname = \"duho-mcp\"\ntransport = \"stdio\"\ncommand = \"probe\"\n\
         [[mcp.skills]]\nname = \"duho-skill\"\nserver = \"duho-mcp\"\n",
    );
    temp.write("overlays/duho/bin/tool.py", "print('ok')\n");
    temp.write(
        "overlays/duho/skills/duho-skill/SKILL.md",
        "---\nname: duho-skill\ndescription: Tool help\n---\nRun `duho-cli --help-agents`.\n",
    );
    let overlay_digest = digest_of(&temp, "overlays/duho");
    temp.write(
        "overlays/duho.json",
        &json!({"source": "x", "commit": "y", "digest": overlay_digest, "files": {}}).to_string(),
    );

    // Compute exact MCP definition digest without allow_mcp first.
    let base_cfg = "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
                    [host.profiles.main]\noverlays = [\"duho\"]\n";
    let base_offer = offer_of(&temp, base_cfg);
    let base_conn = Connection::prepare(&base_offer, 0, "a", &lookup).expect("prepared");

    // 1. Coding harness (Claude with skills delivery + writable sandbox): keeps SKILL.md, deduplicates duplicate MCP schema.
    let coding = parts_for(&base_conn, Some("claude-code"), &claude(), &lookup);
    assert!(
        coding.servers.is_empty(),
        "MCP server suppressed when [scripts] + SKILL.md apply"
    );
    assert!(coding.written.is_some());
    assert_eq!(
        item(&coding.report, "skills.duho-skill")["state"],
        "applied"
    );
    assert_eq!(item(&coding.report, "mcp.duho-mcp")["state"], "applied");
    assert!(
        item(&coding.report, "mcp.duho-mcp")["how"]
            .as_str()
            .unwrap()
            .contains("deduplicated")
    );

    // Compute digest for allow_mcp using a shell-less agent take.
    let mcp_takes = Takes {
        version: 1,
        capabilities: json!({"sessionCapabilities": {"systemPrompt": {}}}),
    };
    let probe_parts = parts_for(&base_conn, None, &mcp_takes, &lookup);
    let digest = item(&probe_parts.report, "mcp.duho-mcp")["digest"]
        .as_str()
        .expect("digest")
        .to_owned();
    let mcp_cfg = format!(
        "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
         [host.profiles.main]\noverlays = [\"duho\"]\n\
         allow_mcp = {{ duho-mcp = \"{digest}\" }}\n"
    );
    let mcp_offer = offer_of(&temp, &mcp_cfg);
    let mcp_conn = Connection::prepare(&mcp_offer, 0, "a", &lookup).expect("prepared");

    // 2. Shell-less / read-only harness: delivers MCP server and suppresses redundant SKILL.md.
    let ro_parts = parts_for(&mcp_conn, None, &mcp_takes, &lookup);
    assert_eq!(ro_parts.servers.len(), 1);
    assert!(
        ro_parts.written.is_none(),
        "SKILL.md suppressed when MCP server is delivered"
    );
    assert_eq!(
        item(&ro_parts.report, "skills.duho-skill")["state"],
        "applied"
    );
    assert!(
        item(&ro_parts.report, "skills.duho-skill")["how"]
            .as_str()
            .unwrap()
            .contains("deduplicated")
    );
}
