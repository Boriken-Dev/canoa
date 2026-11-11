#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;

const PEER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn base() -> PathBuf {
    std::env::temp_dir().join("canoa-config-test")
}

fn parse(text: &str) -> Result<Config, ConfigError> {
    Config::from_str_in(text, &base())
}

#[test]
fn a_file_with_no_host_table_serves_nothing() {
    let config =
        parse("[desktop]\nstart_hidden = true\n[appearance]\nlook = \"canoa\"\n").expect("parses");
    assert!(config.entries.is_empty());
}

#[test]
fn entries_keep_the_files_order_agents_first() {
    let config = parse(
        r#"
[host.providers.zeta]
library = "echo.dll"
provider = "echo_provider"
[host.agents.beta]
program = "node"
args = ["agent.mjs"]
[host.agents.alpha]
program = "claude-agent-acp"
listen = "127.0.0.1:7801"
"#,
    )
    .expect("parses");
    let names: Vec<&str> = config
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["beta", "alpha", "zeta"]);
    let beta = config.entry("beta").expect("there");
    assert_eq!(beta.listen, "127.0.0.1:0".parse().expect("an address"));
    assert_eq!(beta.max_peers, DEFAULT_MAX_PEERS);
    assert_eq!(beta.peers, None);
    assert_eq!(
        beta.kind,
        EntryKind::Agent {
            program: "node".into(),
            args: vec!["agent.mjs".into()],
            profiles: Vec::new(),
            usage: None,
        }
    );
    let EntryKind::Provider {
        origin,
        settings,
        directory,
        max_sessions,
        ..
    } = &config.entry("zeta").expect("there").kind
    else {
        panic!("a provider");
    };
    assert_eq!(
        *origin,
        Origin::Library(std::path::absolute(base().join("echo.dll")).expect("absolute"))
    );
    assert_eq!(*settings, None);
    assert_eq!(*directory, std::path::absolute(base()).expect("absolute"));
    assert_eq!(*max_sessions, DEFAULT_MAX_SESSIONS);
}

fn provider_kind(text: &str) -> (Origin, Option<Settings>) {
    let config = parse(text).expect("parses");
    let EntryKind::Provider {
        origin, settings, ..
    } = config.entries[0].kind.clone()
    else {
        panic!("a provider");
    };
    (origin, settings)
}

#[test]
fn a_source_entry_takes_its_settings_as_a_table() {
    let (origin, settings) = provider_kind(
        r#"
[host.providers.python]
source = "guatiao_source_py.dll"
provider = "acp_echo_echo"
[host.providers.python.source_settings]
dirs = ["python"]
paths = ["/opt/guatiao", "kinds"]
[host.providers.python.settings]
prefix = "hi"
nested = { depth = 2, ratio = 0.5, on = true, list = [1, "two"] }
"#,
    );
    let Origin::Source {
        file,
        settings: source,
        ..
    } = origin
    else {
        panic!("a source: {origin:?}");
    };
    assert_eq!(
        file,
        std::path::absolute(base().join("guatiao_source_py.dll")).expect("absolute")
    );
    let absolute = |path: &str| {
        std::path::absolute(base().join(path))
            .expect("absolute")
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        serde_json::Value::Object(source.as_map().clone()),
        serde_json::json!({"dirs": [absolute("python")], "paths": [absolute("/opt/guatiao"), absolute("kinds")]})
    );
    assert_eq!(
        serde_json::Value::Object(settings.expect("settings").as_map().clone()),
        serde_json::json!({"prefix": "hi", "nested": {"depth": 2, "ratio": 0.5, "on": true, "list": [1, "two"]}})
    );
}

#[test]
fn a_source_needs_no_settings() {
    let (origin, settings) =
        provider_kind("[host.providers.p]\nsource = \"s.so\"\nprovider = \"p\"\n");
    assert!(
        matches!(&origin, Origin::Source { settings, .. } if settings.as_map().is_empty()),
        "{origin:?}"
    );
    assert_eq!(settings, None);
}

#[test]
fn a_library_takes_settings_and_no_source_settings() {
    let (origin, settings) = provider_kind(
        "[host.providers.p]\nlibrary = \"l.dll\"\nprovider = \"p\"\n[host.providers.p.settings]\nprefix = \"hi\"\n",
    );
    assert!(matches!(origin, Origin::Library(_)), "{origin:?}");
    assert_eq!(
        settings.expect("settings").as_map().get("prefix"),
        Some(&serde_json::json!("hi"))
    );
    let error = parse(
        "[host.providers.p]\nlibrary = \"l.dll\"\nprovider = \"p\"\n[host.providers.p.source_settings]\ndirs = []\n",
    )
    .expect_err("refused");
    assert!(error.to_string().contains("source_settings"), "{error}");
}

#[test]
fn library_and_source_exclude_each_other_and_one_is_needed() {
    let both = "[host.providers.p]\nlibrary = \"l\"\nsource = \"s\"\nprovider = \"p\"\n";
    assert!(
        parse(both)
            .expect_err("refused")
            .to_string()
            .contains("exclude")
    );
    let neither = "[host.providers.p]\nprovider = \"p\"\n";
    assert!(
        parse(neither)
            .expect_err("refused")
            .to_string()
            .contains("library or source")
    );
    for empty in ["library = \"\"", "source = \"\""] {
        let text = format!("[host.providers.p]\n{empty}\nprovider = \"p\"\n");
        assert!(parse(&text).is_err(), "{empty}");
    }
}

#[test]
fn settings_are_tables_of_what_json_carries() {
    for bad in [
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n[host.providers.p.source_settings]\nwhen = 1979-05-27\n",
        "[host.providers.p]\nlibrary = \"l\"\nprovider = \"p\"\n[host.providers.p.settings]\nx = nan\n",
        "[host.providers.p]\nlibrary = \"l\"\nprovider = \"p\"\n[host.providers.p.settings]\nx = [inf]\n",
        "[host.providers.p]\nlibrary = \"l\"\nprovider = \"p\"\nsettings = \"x\"\n",
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\nsource_settings = [1]\n",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_setting_is_never_shown() {
    let secret = "sk-ant-oat01-SECRETSECRET";
    let text = format!(
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n[host.providers.p.source_settings]\ntoken = \"{secret}\"\n[host.providers.p.settings]\nkey = \"{secret}\"\n"
    );
    let config = parse(&text).expect("parses");
    let shown = format!("{config:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(shown.contains("token") && shown.contains("key"), "{shown}");
    let refused = format!("{text}when = 1979-05-27\n");
    let error = parse(&refused).expect_err("a date is refused");
    assert!(!error.to_string().contains("SECRET"), "{error}");
}

#[test]
fn a_misspelled_setting_is_refused_with_its_position_and_not_its_text() {
    let Err(error) = parse("[host.agents.a]\nprogram = \"x\"\nlsten = \"127.0.0.1:1\"\n") else {
        panic!("refused");
    };
    let ConfigError::Parse { line, .. } = &error else {
        panic!("a parse error: {error}");
    };
    assert_eq!(*line, 3, "{error}");
    assert!(!error.to_string().contains("lsten"), "{error}");
}

#[test]
fn a_value_is_never_quoted_in_an_error() {
    let secret = "sk-ant-oat01-SECRETSECRET";
    for text in [
        format!("[host.agents.a]\nprogram = [\"{secret}\"]\n"),
        format!("[host.agents.a]\nprogram = \"x\"\nlisten = \"{secret}\"\n"),
        format!("[host.agents.a]\nprogram = \"x\"\npeers = [\"{secret}\"]\n"),
        format!("[host]\norigins = [\"{secret} \"]\n"),
        format!("[host.agents.a]\nprogram = \"x\"\nmax_peers = \"{secret}\"\n"),
        format!("[host.agents.a]\nprogram = \"{secret}\n"),
    ] {
        let error = parse(&text).expect_err("refused");
        assert!(!error.to_string().contains("SECRET"), "{error}");
        assert!(!format!("{error:?}").contains("SECRET"), "{error:?}");
    }
}

#[test]
fn a_syntax_error_says_where() {
    let error = parse("[desktop]\n\n[host.agents.a]\nprogram = \n").expect_err("refused");
    let ConfigError::Parse { line, column, .. } = error else {
        panic!("a parse error");
    };
    assert_eq!((line, column), (4, 11));
}

#[test]
fn a_missing_required_setting_is_named() {
    let error = parse("[host.providers.p]\nlibrary = \"x\"\n").expect_err("refused");
    assert!(error.to_string().contains("provider"), "{error}");
}

#[test]
fn listening_beyond_loopback_is_plain_configuration() {
    for listen in ["0.0.0.0:7801", "192.0.2.1:7801", "[::]:7801", "[::1]:7801"] {
        let text = format!("[host.agents.a]\nprogram = \"x\"\nlisten = \"{listen}\"\n");
        let config = parse(&text).expect("an address is accepted whatever it is");
        assert_eq!(config.entries[0].listen, listen.parse().expect("address"));
    }
}

#[test]
fn names_and_addresses_are_unique() {
    let shared_name =
        "[host.agents.a]\nprogram = \"x\"\n[host.providers.a]\nlibrary = \"l\"\nprovider = \"p\"\n";
    assert!(parse(shared_name).is_err());
    let shared_port = "[host.agents.a]\nprogram = \"x\"\nlisten = \"127.0.0.1:7801\"\n[host.agents.b]\nprogram = \"y\"\nlisten = \"127.0.0.1:7801\"\n";
    assert!(parse(shared_port).is_err());
    let any_port = "[host.agents.a]\nprogram = \"x\"\n[host.agents.b]\nprogram = \"y\"\n";
    assert_eq!(parse(any_port).expect("port 0 twice").entries.len(), 2);
}

#[test]
fn an_entry_name_is_an_identifier() {
    for name in ["\"a b\"", "\"\"", "\"a/b\"", "\"\u{202e}x\""] {
        let text = format!("[host.agents.{name}]\nprogram = \"x\"\n");
        assert!(parse(&text).is_err(), "{name}");
    }
}

#[test]
fn a_name_is_checked_before_an_error_quotes_it() {
    for body in ["program = \"\"", "program = \"x\"\nlisten = \"nowhere\""] {
        let text = format!("[host.agents.\"\\u001b]0;x\\u0007\"]\n{body}\n");
        let error = parse(&text).expect_err("refused");
        assert!(
            !error.to_string().chars().any(char::is_control),
            "{error:?}"
        );
    }
    let provider = "[host.providers.\"\\u001b[2J\"]\nlibrary = \"\"\nprovider = \"p\"\n";
    let error = parse(provider).expect_err("refused");
    assert!(
        !error.to_string().chars().any(char::is_control),
        "{error:?}"
    );
}

#[test]
fn peers_are_peer_ids() {
    let text = format!("[host.agents.a]\nprogram = \"x\"\npeers = [\"{PEER}\"]\n");
    let config = parse(&text).expect("parses");
    assert_eq!(config.entries[0].peers.as_ref().map(Vec::len), Some(1));
    assert!(parse("[host.agents.a]\nprogram = \"x\"\npeers = [\"nobody\"]\n").is_err());
}

#[test]
fn a_working_root_is_absolute_and_plain() {
    let root = std::env::temp_dir();
    let good = format!(
        "[host.agents.a]\nprogram = \"x\"\nworking_roots = [{}]\n",
        serde_json::to_string(&root.to_string_lossy()).expect("JSON")
    );
    assert_eq!(
        parse(&good).expect("parses").entries[0].working_roots,
        core::slice::from_ref(&root)
    );
    let under = root.to_string_lossy();
    for bad in [
        "relative".to_owned(),
        format!("{under}/a/../b"),
        format!("{under}/a/./b"),
        format!("{under}/a/."),
        format!("{under}\\a\\.\\b"),
    ] {
        let text = format!(
            "[host.agents.a]\nprogram = \"x\"\nworking_roots = [{}]\n",
            serde_json::to_string(&bad).expect("JSON")
        );
        assert!(parse(&text).is_err(), "{bad}");
    }
}

#[test]
fn limits_of_zero_are_refused() {
    assert!(parse("[host.agents.a]\nprogram = \"x\"\nmax_peers = 0\n").is_err());
    assert!(
        parse("[host.providers.a]\nlibrary = \"l\"\nprovider = \"p\"\nmax_sessions = 0\n").is_err()
    );
}

#[test]
fn a_byte_order_mark_is_ignored() {
    assert_eq!(
        parse("\u{feff}[host.agents.a]\nprogram = \"x\"\n")
            .expect("parses")
            .entries
            .len(),
        1
    );
}

#[test]
fn any_text_gives_a_value_or_an_error() {
    let sample = "[host]\norigins = [\"https://a\"]\n[host.agents.a]\nprogram = \"x\"\nargs = [\"y\"]\nlisten = \"127.0.0.1:1\"\n[host.providers.p]\nlibrary = \"l\"\nprovider = \"p\"\npeers = []\n[host.providers.q]\nsource = \"s\"\nprovider = \"q\"\n[host.providers.q.source_settings]\ndirs = [\"d\", 1.5]\n[host.providers.q.settings]\nx = { y = [true] }\n";
    for end in 0..=sample.len() {
        if sample.is_char_boundary(end) {
            let _ = parse(&sample[..end]);
        }
    }
    const ALPHABET: &[u8] = b"[]=\"'.,{}\n abhostagentsprovidrlm0123456789#";
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..2000 {
        let text: String = (0..64)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ALPHABET[(state % ALPHABET.len() as u64) as usize] as char
            })
            .collect();
        let _ = parse(&text);
    }
}

#[test]
fn relative_paths_a_source_reads_are_made_absolute_against_the_configs_directory() {
    let (origin, _) = provider_kind(
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n[host.providers.p.source_settings]\ndirs = [\"python/echo\"]\nmanifests = [\"one.guatiao\"]\npaths = [\"python/guatiao\"]\nimport_dirs = [\"python\"]\nhome = \"python/home\"\nisolation = \"isolated\"\nprefix_file = \"stays.txt\"\n",
    );
    let Origin::Source { settings, .. } = origin else {
        panic!("a source");
    };
    let absolute_one = |relative: &str| {
        std::path::absolute(base().join(relative))
            .expect("absolute")
            .to_str()
            .expect("UTF-8")
            .to_owned()
    };
    let absolute = |relative: &str| serde_json::json!([absolute_one(relative)]);
    let map = settings.as_map();
    assert_eq!(map["dirs"], absolute("python/echo"));
    assert_eq!(map["manifests"], absolute("one.guatiao"));
    assert_eq!(map["paths"], absolute("python/guatiao"));
    assert_eq!(map["import_dirs"], absolute("python"));
    assert_eq!(map["home"], absolute_one("python/home"));
    assert_eq!(map["isolation"], "isolated", "not a path, so as written");
    assert_eq!(
        map["prefix_file"], "stays.txt",
        "a key no source reads as a path list is passed as written"
    );
}

#[test]
fn a_source_names_its_import_search_and_takes_the_narrow_one_where_import_dirs_say_where_to_look() {
    let search = |extra: &str, settings: &str| {
        let (origin, _) = provider_kind(&format!(
            "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\n{extra}[host.providers.p.source_settings]\n{settings}"
        ));
        let Origin::Source { import_search, .. } = origin else {
            panic!("a source");
        };
        import_search
    };
    assert_eq!(
        search("", ""),
        ImportSearch::Path,
        "nothing names where python3.dll is: PATH, as before"
    );
    assert_eq!(
        search("", "import_dirs = [\"python\"]\n"),
        ImportSearch::Library
    );
    assert_eq!(
        search("import_search = \"library\"\n", ""),
        ImportSearch::Library
    );
    assert_eq!(
        search("import_search = \"path\"\n", "import_dirs = [\"python\"]\n"),
        ImportSearch::Path,
        "a choice is never taken away"
    );
    assert_eq!(
        search("", "import_dirs = []\n"),
        ImportSearch::Path,
        "an empty list names nowhere"
    );
    for bad in [
        "[host.providers.p]\nsource = \"s\"\nprovider = \"p\"\nimport_search = \"everywhere\"\n",
        "[host.providers.p]\nlibrary = \"l\"\nprovider = \"p\"\nimport_search = \"library\"\n",
    ] {
        let error = parse(bad).expect_err("refused");
        assert!(error.to_string().contains("import_search"), "{error}");
    }
}

#[test]
fn the_apps_profile_tables_are_left_to_the_app() {
    let config = parse(
        "[overlays]\nteam = \"https://example.invalid/p.git@main\"\n[profile]\nmodel = \"m\"\n[agents.Claude.profile]\neffort = \"high\"\n",
    )
    .expect("parses");
    assert!(config.entries.is_empty());
}

#[test]
fn worktrees_are_on_for_agents_and_tools_unless_switched_off_and_never_for_providers() {
    let tool = "runs = \"session\"\nunasked = \"x\"\nprompt = { type = \"prompt\", message = \"{prompt}\" }\n";
    let on = parse(&format!(
        "[host.agents.a]\nprogram = \"x\"\n[host.providers.p]\nlibrary = \"e.dll\"\nprovider = \"e\"\n\
         [host.tools.t]\nprogram = \"pi\"\n{tool}"
    ))
    .expect("parses");
    let worktrees: Vec<bool> = on.entries.iter().map(|entry| entry.worktrees).collect();
    assert_eq!(worktrees, [true, false, true]);
    let off = parse(&format!(
        "[host.agents.a]\nprogram = \"x\"\nworktrees = false\n[host.tools.t]\nprogram = \"pi\"\nworktrees = false\n{tool}"
    ))
    .expect("parses");
    assert!(off.entries.iter().all(|entry| !entry.worktrees));
    assert!(
        parse("[host.providers.p]\nlibrary = \"e.dll\"\nprovider = \"e\"\nworktrees = true\n")
            .is_err()
    );
    assert!(parse("[host.agents.a]\nprogram = \"x\"\nworktrees = \"no\"\n").is_err());
    assert!(
        parse(&format!(
            "[host.tools.t]\nprogram = \"pi\"\nworktrees = 0\n{tool}"
        ))
        .is_err()
    );
}

#[test]
fn a_file_without_a_web_table_serves_no_web_app() {
    let config = parse("[host.agents.a]\nprogram = \"x\"\n").expect("parses");
    assert_eq!(config.web, None);
}

#[test]
fn the_web_table_takes_its_defaults_and_paths_from_the_files_directory() {
    let config = parse("[host.agents.a]\nprogram = \"x\"\n[host.web]\n").expect("parses");
    assert_eq!(config.web, Some(WebConfig::new()));
    let web = WebConfig::new();
    assert_eq!(web.listen, "127.0.0.1:7800".parse().expect("an address"));
    assert_eq!(web.dir, None);
    assert_eq!(web.tls, None);

    let config = parse(
        r#"
[host.agents.a]
program = "x"
[host.web]
dir = "site"
listen = "0.0.0.0:8443"
cert = "tls/web.crt"
key = "tls/web.key"
hosts = ["canoa.example", "Canoa-Box"]
connect = ["https://fonts.gstatic.com", "wss://relay.example:8443"]
max_connections = 8
max_connections_per_address = 2
connection_seconds = 120
max_request_bytes = 4096
max_requests = 50
request_seconds = 5
idle_seconds = 7
send_seconds = 9
immutable_seconds = 60
"#,
    )
    .expect("parses");
    let web = config.web.expect("a web table");
    let directory = std::path::absolute(base()).expect("absolute");
    assert_eq!(web.dir, Some(directory.join("site")));
    assert_eq!(web.listen, "0.0.0.0:8443".parse().expect("an address"));
    assert_eq!(
        web.tls,
        Some(WebTls::new(
            directory.join("tls").join("web.crt"),
            directory.join("tls").join("web.key")
        ))
    );
    assert_eq!(web.hosts, ["canoa.example", "canoa-box"]);
    assert_eq!(
        web.connect,
        ["https://fonts.gstatic.com", "wss://relay.example:8443"]
    );
    assert_eq!(
        web.limits,
        crate::web::Limits::default()
            .with_max_connections(8)
            .with_max_connections_per_address(2)
            .with_connection_time(core::time::Duration::from_secs(120))
            .with_max_request_bytes(4096)
            .with_max_requests(50)
            .with_request_time(core::time::Duration::from_secs(5))
            .with_idle_time(core::time::Duration::from_secs(7))
            .with_send_time(core::time::Duration::from_secs(9))
            .with_immutable_age(core::time::Duration::from_secs(60))
    );
}

#[test]
fn plain_http_beyond_loopback_is_refused() {
    let refused = parse("[host.web]\nlisten = \"0.0.0.0:7800\"\n").expect_err("refused");
    assert!(refused.to_string().contains("cert and key"), "{refused}");
    assert!(parse("[host.web]\nlisten = \"10.0.0.5:7800\"\ncert = \"a.crt\"\n").is_err());
    assert!(parse("[host.web]\nlisten = \"[::]:7800\"\nkey = \"a.key\"\n").is_err());
    assert!(parse("[host.web]\nlisten = \"127.0.0.1:7800\"\ncert = \"a.crt\"\n").is_err());
    assert!(parse("[host.web]\nlisten = \"[::1]:7800\"\n").is_ok());
    assert!(
        parse("[host.web]\nlisten = \"0.0.0.0:7800\"\ncert = \"a.crt\"\nkey = \"a.key\"\n").is_ok()
    );
}

#[test]
fn a_web_setting_that_cannot_serve_is_refused_by_name() {
    for (setting, value) in [
        ("listen", "\"nowhere\""),
        ("dir", "\"\""),
        ("cert", "\"\""),
        ("hosts", "[\"\"]"),
        ("hosts", "[\"bad host\"]"),
        ("hosts", "[\"host:80\"]"),
        ("hosts", "[\"a/b\"]"),
        ("connect", "[\"*\"]"),
        ("connect", "[\"'unsafe-eval'\"]"),
        ("connect", "[\"https://a.example; script-src *\"]"),
        ("connect", "[\"https://a.example b.example\"]"),
        ("connect", "[\"https:\"]"),
        ("connect", "[\"ftp://a.example\"]"),
        ("connect", "[\"https://a.example,b\"]"),
        ("max_connections", "0"),
        ("max_connections_per_address", "0"),
        ("connection_seconds", "0"),
        ("max_request_bytes", "0"),
        ("max_requests", "0"),
        ("request_seconds", "0"),
        ("idle_seconds", "0"),
        ("send_seconds", "0"),
        ("unknown", "1"),
    ] {
        let text = format!("[host.web]\n{setting} = {value}\n");
        let refused = parse(&text).expect_err(&text);
        let said = refused.to_string();
        assert!(
            !said.contains("bad host") && !said.contains("nowhere"),
            "{said}"
        );
    }
    assert!(parse("[host.web]\nimmutable_seconds = 0\n").is_ok());
}

#[test]
fn the_web_app_and_an_entry_cannot_share_an_address() {
    let refused =
        parse("[host.agents.a]\nprogram = \"x\"\nlisten = \"127.0.0.1:7800\"\n[host.web]\n")
            .expect_err("refused");
    assert!(refused.to_string().contains("listen"), "{refused}");
    assert!(
        parse("[host.agents.a]\nprogram = \"x\"\nlisten = \"127.0.0.1:0\"\n[host.web]\nlisten = \"127.0.0.1:0\"\n").is_ok()
    );
}

#[test]
fn an_agent_entry_names_a_harness_the_host_runs_itself_instead_of_a_program() {
    let config = parse(
        r#"
[host.agents.Claude]
harness = "claude"
profiles = ["plain"]
[host.agents.Codex]
harness = "codex"
executable = "codex"
[host.profiles.plain]
"#,
    )
    .expect("parses");
    assert_eq!(
        config.entry("Claude").expect("there").kind,
        EntryKind::Harness {
            harness: "claude".to_owned(),
            origin: None,
            executable: None,
            options: HarnessOptions::default(),
            settings: None,
            profiles: vec!["plain".to_owned()],
        }
    );
    assert_eq!(
        config.entry("Codex").expect("there").kind,
        EntryKind::Harness {
            harness: "codex".to_owned(),
            origin: None,
            executable: Some("codex".into()),
            options: HarnessOptions::default(),
            settings: None,
            profiles: Vec::new(),
        }
    );
    for (text, what) in [
        ("[host.agents.a]\nharness = \"gemini\"\n", "harness"),
        ("[host.agents.a]\nharness = \"\"\n", "harness"),
        (
            "[host.agents.a]\nharness = \"claude\"\nprogram = \"claude\"\n",
            "program or harness",
        ),
        ("[host.agents.a]\nargs = [\"x\"]\n", "program or harness"),
        (
            "[host.agents.a]\nharness = \"claude\"\nargs = [\"x\"]\n",
            "args",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nexecutable = \"y\"\n",
            "executable",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nexecutable = \"\"\n",
            "executable",
        ),
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
}

#[test]
fn a_harness_entry_names_the_modes_a_client_may_choose_or_leaves_the_default() {
    use coqui_acp_harness::Mode;
    let config = parse(
        r#"
[host.agents.Default]
harness = "claude"
[host.agents.Named]
harness = "codex"
modes = ["plan", "accept_edits"]
"#,
    )
    .expect("parses");
    let modes = |name: &str| match config.entry(name).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { options, .. }) => options.modes.clone(),
        _ => panic!("a harness"),
    };
    assert_eq!(
        modes("Default"),
        None,
        "a client is capped at the entry's own mode"
    );
    assert_eq!(modes("Named"), Some(vec![Mode::Plan, Mode::AcceptEdits]));
    for text in [
        "[host.agents.a]\nharness = \"claude\"\nmodes = [\"yolo\"]\n",
        "[host.agents.a]\nprogram = \"x\"\nmodes = [\"plan\"]\n",
        "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nmodes = [\"plan\"]\n",
        "[host.agents.a]\nharness = \"claude\"\nmodes = [\"plan\"]\nsettings = { modes = [\"auto\"] }\n",
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}

#[test]
fn setting_sources_belong_to_claude_and_name_user_project_or_local() {
    let config = parse(
        "[host.agents.a]\nharness = \"claude\"\nsetting_sources = [\"project\"]\n[host.agents.b]\nharness = \"claude\"\nsetting_sources = []\n",
    )
    .expect("parses");
    let sources = |name: &str| match config.entry(name).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { options, .. }) => options.setting_sources.clone(),
        _ => panic!("a harness"),
    };
    assert_eq!(sources("a"), Some(vec!["project".to_owned()]));
    assert_eq!(sources("b"), Some(Vec::new()), "none is not unset");
    for (text, what) in [
        (
            "[host.agents.a]\nharness = \"claude\"\nsetting_sources = [\"elsewhere\"]\n",
            "setting_sources",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nsetting_sources = [\"user\"]\n",
            "setting_sources",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nsetting_sources = [\"user\"]\n",
            "harness",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\nsetting_sources = [\"user\"]\nsettings = { setting_sources = [] }\n",
            "twice",
        ),
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
}

#[test]
fn an_outside_harness_is_a_provider_id_with_a_library_or_a_source_and_its_own_settings() {
    let config = parse(
        r#"
[host.agents.Echo]
harness = "echo_harness"
library = "providers/echo.dll"
settings = { title = "Echo" }
profiles = ["plain"]
[host.agents.Py]
harness = "py_harness"
source = "source.dll"
import_search = "library"
[host.agents.Py.source_settings]
dirs = ["manifests"]
[host.profiles.plain]
"#,
    )
    .expect("parses");
    let Some(EntryKind::Harness {
        harness,
        origin,
        settings,
        options,
        ..
    }) = config.entry("Echo").map(|entry| &entry.kind)
    else {
        panic!("a harness");
    };
    assert_eq!(harness, "echo_harness");
    assert_eq!(
        origin,
        &Some(Origin::Library(
            std::path::absolute(base().join("providers/echo.dll")).expect("absolute")
        ))
    );
    assert_eq!(
        settings.as_ref().and_then(|s| s.as_map().get("title")),
        Some(&serde_json::json!("Echo"))
    );
    assert_eq!(options, &HarnessOptions::default());
    let Some(EntryKind::Harness { origin, .. }) = config.entry("Py").map(|entry| &entry.kind)
    else {
        panic!("a harness");
    };
    assert!(matches!(
        origin,
        Some(Origin::Source {
            import_search: ImportSearch::Library,
            import_search_named: true,
            ..
        })
    ));
    for (text, what) in [
        // Nothing loads an id the host does not carry.
        ("[host.agents.a]\nharness = \"echo\"\n", "library or source"),
        (
            "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nsource = \"s\"\n",
            "exclude",
        ),
        (
            "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nimport_search = \"path\"\n",
            "import_search",
        ),
        (
            "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\npermission_mode = \"plan\"\n",
            "permission_mode",
        ),
        (
            "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nexecutable = \"e\"\n",
            "executable",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nlibrary = \"l\"\n",
            "harness",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nsettings = { a = 1 }\n",
            "harness",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\npermission_mode = \"plan\"\nsettings = { permission_mode = \"auto\" }\n",
            "twice",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\nbypass = true\nsettings = { allow_bypass = false }\n",
            "twice",
        ),
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
    // The built-in ids take a settings table too.
    assert!(parse("[host.agents.a]\nharness = \"claude\"\nsettings = { model = \"m\" }\n").is_ok());
}

#[test]
fn a_harness_entry_offers_bypass_only_when_its_canoa_cfg_says_so() {
    let config = parse(
        r#"
[host.agents.Off]
harness = "claude"
[host.agents.On]
harness = "codex"
bypass = true
"#,
    )
    .expect("parses");
    let bypass = |name: &str| match config.entry(name).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { options, .. }) => options.bypass,
        _ => panic!("a harness"),
    };
    assert!(!bypass("Off"), "off unless the entry turns it on");
    assert!(bypass("On"));
    for text in [
        "[host.agents.a]\nprogram = \"x\"\nbypass = true\n",
        "[host.agents.a]\nharness = \"claude\"\nbypass = \"yes\"\n",
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}

#[test]
fn a_harness_entry_holds_its_command_lines_permissions_and_its_program_to_canoa_cfg() {
    let config = parse(
        r#"
[host.agents.Claude]
harness = "claude"
executable = "bin/claude"
permission_mode = "acceptEdits"
[host.agents.Codex]
harness = "codex"
executable = "codex-nightly"
approval_policy = "untrusted"
sandbox = "read-only"
"#,
    )
    .expect("parses");
    let Some(EntryKind::Harness {
        executable,
        options,
        ..
    }) = config.entry("Claude").map(|entry| &entry.kind)
    else {
        panic!("a harness");
    };
    assert_eq!(
        executable.as_deref(),
        Some(directory_of(&base()).join("bin/claude").as_os_str()),
        "a path is taken from canoa.cfg's directory, never a session's"
    );
    assert_eq!(options.permission_mode.as_deref(), Some("acceptEdits"));
    let Some(EntryKind::Harness {
        executable,
        options,
        ..
    }) = config.entry("Codex").map(|entry| &entry.kind)
    else {
        panic!("a harness");
    };
    assert_eq!(
        executable.as_deref(),
        Some(std::ffi::OsStr::new("codex-nightly")),
        "a bare name is found as a command is"
    );
    assert_eq!(options.approval_policy.as_deref(), Some("untrusted"));
    assert_eq!(options.sandbox.as_deref(), Some("read-only"));
    for (text, what) in [
        (
            "[host.agents.a]\nharness = \"claude\"\napproval_policy = \"never\"\n",
            "approval_policy",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\npermission_mode = \"plan\"\n",
            "permission_mode",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\npermission_mode = \"yolo\"\n",
            "permission_mode",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nsandbox = \"open\"\n",
            "sandbox",
        ),
        (
            "[host.agents.a]\nprogram = \"x\"\nsandbox = \"read-only\"\n",
            "harness",
        ),
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
}

#[test]
fn usage_limit_phrases_are_unset_a_list_or_empty() {
    assert_eq!(parse("").expect("parses").limit_phrases, None);
    // The app's own key in the same table is let go by.
    let unset = parse("[usage]\nwarn_permille = 500\n").expect("parses");
    assert_eq!(unset.limit_phrases, None);
    let set = parse("[usage]\nlimit_phrases = [\"over quota\", \"429\"]\n").expect("parses");
    assert_eq!(
        set.limit_phrases,
        Some(vec!["over quota".to_owned(), "429".to_owned()])
    );
    let none =
        parse("[usage]\nlimit_phrases = []\n[host.agents.a]\nprogram = \"x\"\n").expect("parses");
    assert_eq!(none.limit_phrases, Some(Vec::new()));
    assert_eq!(none.entries.len(), 1);
}

#[test]
fn an_empty_usage_phrase_is_refused() {
    for text in [
        "[usage]\nlimit_phrases = [\"\"]\n",
        "[usage]\nlimit_phrases = [\" \"]\n",
    ] {
        assert!(
            matches!(parse(text), Err(ConfigError::Invalid { .. })),
            "{text}"
        );
    }
    assert!(matches!(
        parse("[usage]\nlimit_phrases = \"quota\"\n"),
        Err(ConfigError::Parse { .. })
    ));
}

#[test]
fn an_agent_entry_names_where_its_usage_is_read_from() {
    let config =
        parse("[host.agents.a]\nprogram = \"agy\"\nargs = [\"acp\"]\nusage = \"antigravity\"\n")
            .expect("parses");
    assert!(matches!(
        config.entries[0].kind,
        EntryKind::Agent {
            usage: Some(UsageSource::Antigravity),
            ..
        }
    ));
    for text in [
        "[host.agents.a]\nprogram = \"agy\"\nusage = \"other\"\n",
        "[host.agents.a]\nharness = \"claude\"\nusage = \"antigravity\"\n",
    ] {
        assert!(
            matches!(parse(text), Err(ConfigError::Invalid { .. })),
            "{text}"
        );
    }
}

#[test]
fn claude_and_codex_are_built_in_and_a_library_or_source_cannot_stand_for_them() {
    for text in [
        "[host.agents.a]\nharness = \"claude\"\nlibrary = \"l.dll\"\n",
        "[host.agents.a]\nharness = \"codex\"\nsource = \"s.dll\"\n",
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains("built in"), "{text}: {error}");
    }
}

#[test]
fn the_settings_of_a_built_in_harness_are_held_to_what_its_keys_are() {
    let config = parse(
        "[host.agents.a]\nharness = \"claude\"\nsettings = { executable = \"bin/claude\", permission_mode = \"plan\" }\n",
    )
    .expect("parses");
    let Some(EntryKind::Harness {
        settings: Some(settings),
        ..
    }) = config.entry("a").map(|entry| &entry.kind)
    else {
        panic!("a harness with settings");
    };
    assert_eq!(
        settings.as_map()["executable"],
        serde_json::json!(directory_of(&base()).join("bin/claude").to_string_lossy()),
        "a relative path is canoa.cfg's, never a session's"
    );
    for (text, what) in [
        (
            "[host.agents.a]\nharness = \"claude\"\nexecutable = \"x\"\nsettings = { executable = \"y\" }\n",
            "twice",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\nsettings = { permission_mode = 3 }\n",
            "text",
        ),
        (
            "[host.agents.a]\nharness = \"claude\"\nsettings = { permission_mode = \"yolo\" }\n",
            "permission_mode",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nsettings = { sandbox = \"open\" }\n",
            "sandbox",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nsettings = { approval_policy = true }\n",
            "text",
        ),
        (
            "[host.agents.a]\nharness = \"codex\"\nsettings = { executable = \"\" }\n",
            "executable",
        ),
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
}

#[test]
fn an_agent_entry_may_carry_a_title() {
    let config = parse("[host.agents.a]\nprogram = \"x\"\ntitle = \"Review\"\n").expect("parses");
    assert_eq!(config.entries[0].title.as_deref(), Some("Review"));
    let config = parse("[host.agents.a]\nharness = \"claude\"\n").expect("parses");
    assert_eq!(config.entries[0].title, None);
    let long = "x".repeat(65);
    for text in [
        "[host.agents.a]\nprogram = \"x\"\ntitle = \"\"\n".to_owned(),
        "[host.agents.a]\nprogram = \"x\"\ntitle = \" \"\n".to_owned(),
        "[host.agents.a]\nprogram = \"x\"\ntitle = \"a\nb\"\n".to_owned(),
        format!("[host.agents.a]\nprogram = \"x\"\ntitle = \"{long}\"\n"),
        "[host.agents.a]\nprogram = \"x\"\ntitle = 1\n".to_owned(),
        "[host.providers.p]\nlibrary = \"l.dll\"\nprovider = \"p\"\ntitle = \"T\"\n".to_owned(),
    ] {
        assert!(
            matches!(
                parse(&text),
                Err(ConfigError::Invalid { .. } | ConfigError::Parse { .. })
            ),
            "{text}"
        );
    }
}
