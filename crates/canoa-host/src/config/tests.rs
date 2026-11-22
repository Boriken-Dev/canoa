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
    assert_eq!(
        beta.listen,
        Listen::Address("127.0.0.1:0".parse().expect("an address"))
    );
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
    assert_eq!((line, column), (Some(4), Some(11)));
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
        assert_eq!(
            config.entries[0].listen,
            Listen::Address(listen.parse().expect("address"))
        );
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
fn shared_sessions_are_off_unless_an_agent_entry_asks_and_keep_a_day_by_default() {
    let config = parse(
        "[host.agents.a]
program = \"x\"
[host.agents.b]
program = \"x\"
shared = true
         [host.agents.c]
program = \"x\"
shared = true
shared_idle_seconds = 0
         [host.agents.d]
program = \"x\"
shared = true
shared_idle_seconds = 90
",
    )
    .expect("parses");
    let kept: Vec<_> = config
        .entries
        .iter()
        .map(|entry| (entry.shared, entry.shared_idle))
        .collect();
    assert_eq!(
        kept,
        [
            (false, None),
            (
                true,
                Some(core::time::Duration::from_secs(DEFAULT_SHARED_IDLE_SECONDS))
            ),
            (true, None),
            (true, Some(core::time::Duration::from_secs(90))),
        ]
    );
    assert!(
        parse(
            "[host.agents.a]
program = \"x\"
shared_idle_seconds = 5
"
        )
        .is_err()
    );
    assert!(
        parse(
            "[host.providers.a]
library = \"l\"
provider = \"p\"
shared = true
"
        )
        .is_err()
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
fn the_web_mode_is_local_or_remote_and_unset_by_default() {
    let mode = |text: &str| {
        parse(&format!(
            "[host.agents.a]\nprogram = \"x\"\n[host.web]\n{text}"
        ))
        .map(|config| config.web.expect("a web table").mode)
    };
    assert_eq!(mode("").expect("parses"), None);
    assert_eq!(
        mode("mode = \"local\"\n").expect("parses"),
        Some(WebMode::Local)
    );
    assert_eq!(
        mode("mode = \"remote\"\n").expect("parses"),
        Some(WebMode::Remote)
    );
    let refused = mode("mode = \"nearby\"\n").expect_err("refused");
    assert!(
        refused.to_string().contains("mode is local or remote"),
        "{refused}"
    );
}

/// A pipe's `listen` on this platform, and the same name on the other.
#[cfg(windows)]
const PIPE: &str = "pipe:canoa-test";
#[cfg(unix)]
const PIPE: &str = "unix:/tmp/canoa-test.sock";

#[test]
fn an_entry_may_listen_on_a_pipe_or_a_unix_socket() {
    let text = format!("[host.agents.a]\nprogram = \"x\"\nlisten = \"{PIPE}\"\n");
    let config = parse(&text).expect("a pipe is accepted");
    let Listen::Pipe(name) = &config.entries[0].listen else {
        panic!("a pipe: {:?}", config.entries[0].listen);
    };
    assert_eq!(name.to_string(), PIPE);
    assert_eq!(config.entries[0].listen.to_string(), PIPE);
    assert_eq!(config.entries[0].listen.address(), None);
    assert_eq!(Listen::parse(PIPE), Ok(config.entries[0].listen.clone()));
}

#[test]
fn two_entries_cannot_listen_on_one_pipe_and_a_pipe_is_not_an_address() {
    let text = format!(
        "[host.agents.a]\nprogram = \"x\"\nlisten = \"{PIPE}\"\n[host.agents.b]\nprogram = \"x\"\nlisten = \"{PIPE}\"\n"
    );
    let refused = parse(&text).expect_err("refused");
    assert!(refused.to_string().contains("same address"), "{refused}");
    // A pipe never counts against the web app's address.
    let text = format!(
        "[host.agents.a]\nprogram = \"x\"\nlisten = \"{PIPE}\"\n[host.web]\nlisten = \"127.0.0.1:7800\"\n"
    );
    assert!(parse(&text).is_ok());
}

#[test]
fn a_pipe_that_cannot_be_used_is_refused_for_its_entry_without_the_name() {
    let mut bad = vec!["pipe:", "pipe:a/b", "pipe:..", "unix:"];
    if cfg!(unix) {
        bad.push("unix:relative/path.sock");
    } else {
        // A Unix socket is no use on Windows.
        bad.push("unix:/tmp/a.sock");
    }
    for listen in bad {
        let text = format!("[host.agents.a]\nprogram = \"x\"\nlisten = \"{listen}\"\n");
        let refused = parse(&text).expect_err(listen);
        let said = refused.to_string();
        assert!(said.contains("listen"), "{said}");
        for part in ["a/b", "relative"] {
            assert!(!said.contains(part), "{said}");
        }
    }
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
            account_settings: None,
            profiles: vec!["plain".to_owned()],
            accounts: Vec::new(),
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
            account_settings: None,
            profiles: Vec::new(),
            accounts: Vec::new(),
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
fn the_keys_the_profiles_access_replaces_are_refused_with_no_shim() {
    for key in [
        "modes = [\"plan\"]",
        "bypass = true",
        "permission_mode = \"plan\"",
        "approval_policy = \"never\"",
        "sandbox = \"read-only\"",
    ] {
        let text = format!("[host.agents.a]\nharness = \"claude\"\n{key}\n");
        let error = parse(&text).expect_err(&text).to_string();
        assert!(error.contains("does not know"), "{text}: {error}");
    }
    for key in ["permission_mode", "approval_policy", "sandbox", "access"] {
        let text =
            format!("[host.agents.a]\nharness = \"codex\"\nsettings = {{ {key} = \"x\" }}\n");
        let error = parse(&text).expect_err(&text).to_string();
        assert!(
            error.contains("the access is the profile's"),
            "{text}: {error}"
        );
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
            "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nsetting_sources = []\n",
            "setting_sources",
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
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(what), "{text}: {error}");
    }
    // The built-in ids take a settings table too.
    assert!(parse("[host.agents.a]\nharness = \"claude\"\nsettings = { model = \"m\" }\n").is_ok());
}

#[test]
fn a_harness_entry_holds_its_program_to_canoa_cfg() {
    let config = parse(
        r#"
[host.agents.Claude]
harness = "claude"
executable = "bin/claude"
[host.agents.Codex]
harness = "codex"
executable = "codex-nightly"
"#,
    )
    .expect("parses");
    let executable_of = |name: &str| match config.entry(name).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { executable, .. }) => executable.clone(),
        _ => panic!("a harness"),
    };
    assert_eq!(
        executable_of("Claude").as_deref(),
        Some(directory_of(&base()).join("bin/claude").as_os_str()),
        "a path is taken from canoa.cfg's directory, never a session's"
    );
    assert_eq!(
        executable_of("Codex").as_deref(),
        Some(std::ffi::OsStr::new("codex-nightly")),
        "a bare name is found as a command is"
    );
    let error = parse("[host.agents.a]\nprogram = \"x\"\nexecutable = \"e\"\n")
        .expect_err("a program has no executable")
        .to_string();
    assert!(error.contains("harness"), "{error}");
}

#[test]
fn a_preset_may_set_the_access_and_it_is_the_session_layers() {
    use coqui_acp_harness::{Approvals, Sandbox};
    let config = parse(
        "[profiles.Strict]\naccess = { sandbox = \"read-only\", approvals = \"never\" }\nmodel = \"m\"\n",
    )
    .expect("parses");
    let preset = config
        .local_profiles
        .presets
        .get("Strict")
        .expect("a preset");
    let layer = preset.session_layer();
    assert_eq!(layer.access.sandbox, Some(Sandbox::ReadOnly));
    assert_eq!(layer.access.approvals, Some(Approvals::Never));
    assert_eq!(
        layer.access.network, None,
        "a part left out is a lower layer's"
    );
    assert!(
        parse("[profiles.Bad]\naccess = { sandbox = \"open\" }\n").is_err(),
        "a sandbox that is none of the three"
    );
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
        &config.entries[0].kind,
        EntryKind::Agent { usage: Some(reader), .. } if reader == "antigravity"
    ));
    // A reader a library or source offers is found when the host serves, so any id parses.
    parse("[host.agents.a]\nprogram = \"x\"\nusage = \"from-a-library\"\n").expect("parses");
    for text in [
        "[host.agents.a]\nprogram = \"agy\"\nusage = \"not an id\"\n",
        "[host.agents.a]\nprogram = \"agy\"\nusage = \"\"\n",
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
        "[host.agents.a]\nharness = \"claude\"\nsettings = { executable = \"bin/claude\", model = \"m\" }\n",
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

#[test]
fn a_harness_entry_sets_its_cache_lifetime_and_a_bad_one_is_refused_naming_the_key() {
    use coqui_acp_api::CacheLifetime;
    let config = parse(
        r#"
[host.agents.Unset]
harness = "claude"
[host.agents.Short]
harness = "claude"
cache_lifetime = "short"
[host.agents.Long]
harness = "codex"
cache_lifetime = "long"
[host.agents.Default]
harness = "claude"
cache_lifetime = "default"
[host.agents.Off]
harness = "claude"
cache_lifetime = "off"
[host.agents.Outside]
harness = "x"
library = "l"
cache_lifetime = "short"
"#,
    )
    .expect("parses");
    let lifetime = |name: &str| match config.entry(name).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { options, .. }) => options.cache_lifetime,
        _ => panic!("a harness"),
    };
    assert_eq!(lifetime("Unset"), None, "unset is the client's to default");
    assert_eq!(lifetime("Short"), Some(CacheLifetime::Short));
    assert_eq!(lifetime("Long"), Some(CacheLifetime::Long));
    assert_eq!(lifetime("Default"), Some(CacheLifetime::Default));
    assert_eq!(lifetime("Off"), Some(CacheLifetime::Off));
    assert_eq!(lifetime("Outside"), Some(CacheLifetime::Short));
    for text in [
        "[host.agents.a]\nharness = \"claude\"\ncache_lifetime = \"forever\"\n",
        "[host.agents.a]\nharness = \"claude\"\ncache_lifetime = \"Short\"\n",
        "[host.agents.a]\nprogram = \"x\"\ncache_lifetime = \"short\"\n",
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains("cache_lifetime"), "{text}: {error}");
    }
}

#[test]
fn a_harness_entry_names_its_own_shell_and_a_bad_one_is_refused_naming_the_key() {
    let config = parse(
        r#"
[host]
shell = "pwsh"
[host.agents.Claude]
harness = "claude"
shell = "git-bash"
[host.agents.Plain]
harness = "codex"
"#,
    )
    .expect("parses");
    let shell = |name: &str| config.entry(name).expect("an entry").shell.clone();
    assert_eq!(shell("Claude").as_deref(), Some("git-bash"));
    assert_eq!(shell("Plain"), None, "unset takes [host] shell");
    assert_eq!(config.shell.as_deref(), Some("pwsh"));
    for text in [
        "[host.agents.a]\nharness = \"claude\"\nshell = \"\"\n",
        "[host.agents.a]\nharness = \"claude\"\nshell = \"a\\u0001b\"\n",
        "[host.agents.a]\nprogram = \"x\"\nshell = \"git-bash\"\n",
        "[host.agents.a]\nharness = \"x\"\nlibrary = \"l\"\nshell = \"git-bash\"\n",
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains("shell"), "{text}: {error}");
    }
}

#[test]
fn the_top_level_profile_overlays_and_an_agents_layer_and_choices_are_read_for_the_hosts_own_sources()
 {
    let config = parse(
        "[profile]\nmodel = \"small\"\n\
         [overlays]\nteam = \"https://example.invalid/team.git@main#team\"\n\
         [agents.Claude.profile]\nmodel = \"large\"\n\
         [agents.Claude.mcp]\nfiles = false\ndocs = true\nteam = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
    )
    .expect("parses");
    let local = &config.local_profiles;
    assert_eq!(local.overlays, ["team"]);
    let claude = local.for_agent("Claude").expect("a profile for Claude");
    assert_eq!(claude.overlays, ["team"]);
    assert_eq!(
        claude.agent.as_ref().map(|(label, _)| label.as_str()),
        Some("Claude")
    );
    assert!(claude.off_mcp.contains("files") && !claude.off_mcp.contains("docs"));
    assert_eq!(claude.allow_mcp.len(), 1);
    // The person's layer and the overlays reach every source; an agent's own layer, only that agent.
    assert!(
        local
            .for_agent("Codex")
            .is_some_and(|codex| codex.agent.is_none())
    );
    assert!(
        parse("")
            .expect("parses")
            .local_profiles
            .for_agent("Claude")
            .is_none()
    );
}

#[test]
fn a_choice_that_is_not_on_off_or_a_digest_is_refused_naming_the_setting() {
    let error = parse("[agents.Claude.mcp]\nfiles = \"yes\"\n").expect_err("refused");
    let said = error.to_string();
    assert!(said.contains("agents.Claude.mcp.files"), "{said}");
    assert!(parse("[overlays]\n\"bad name\" = \"x\"\n").is_err());
}

#[test]
fn the_store_table_holds_two_lists_as_text_or_lists() {
    let none = parse("[host]\n").expect("parses");
    assert_eq!(none.store, StoreConfig::default());
    let text = parse(
        "[host.store]\nprotected = \"\"\"\n# mine\n~/.ssh\n!~/.ssh/known_hosts\n\"\"\"\ncredential_variables = [\"*TOKEN\"]\n",
    )
    .expect("parses");
    assert_eq!(
        text.store.protected,
        Some(vec![
            "# mine".to_owned(),
            "~/.ssh".to_owned(),
            "!~/.ssh/known_hosts".to_owned()
        ])
    );
    assert_eq!(
        text.store.credential_variables,
        Some(vec!["*TOKEN".to_owned()])
    );
    let custom_folders =
        parse("[host.store]\nskill_folders = [\"custom/skills\", \".other\"]\n").expect("parses");
    assert_eq!(
        custom_folders.store.skill_folders,
        Some(vec!["custom/skills".to_owned(), ".other".to_owned()])
    );
    let empty = parse("[host.store]\nprotected = []\n").expect("parses");
    assert_eq!(empty.store.protected, Some(Vec::new()));
    assert!(parse("[host.store]\nprotected = 3\n").is_err());
    assert!(parse("[host.store]\nother = 1\n").is_err());
}

#[test]
fn an_overlays_hooks_are_allowed_at_a_digest_by_a_profile_or_by_an_agents_table() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let config = parse(&format!(
        "[host.profiles.p]\noverlays = [\"team\"]\nallow_hooks = {{ team = \"{digest}\" }}\n\
         [agents.Claude.hooks]\nteam = \"{digest}\"\n"
    ))
    .expect("parses");
    assert_eq!(config.profiles["p"].allow_hooks["team"], digest);
    let claude = config
        .local_profiles
        .for_agent("Claude")
        .expect("a profile for Claude");
    assert_eq!(claude.allow_hooks["team"], digest);
    assert!(
        config
            .local_profiles
            .for_agent("Codex")
            .is_none_or(|codex| codex.allow_hooks.is_empty()),
        "an agent's allowance is its own"
    );
}

#[test]
fn an_allowance_that_is_not_a_digest_or_names_no_overlay_is_refused_naming_the_setting() {
    for (text, what) in [
        (
            "[host.profiles.p]\nallow_hooks = { team = \"yes\" }\n",
            "allow_hooks.team",
        ),
        (
            "[host.profiles.p]\nallow_hooks = { team = true }\n",
            "allow_hooks.team",
        ),
        (
            "[host.profiles.p]\nallow_hooks = [\"team\"]\n",
            "allow_hooks",
        ),
        (
            "[host.profiles.p]\nallow_hooks = { \"../up\" = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\" }\n",
            "allow_hooks",
        ),
        (
            "[agents.Claude.hooks]\nteam = \"sha256:12\"\n",
            "agents.Claude.hooks.team",
        ),
        ("[agents.Claude]\nhooks = 3\n", "agents.Claude.hooks"),
    ] {
        let said = parse(text).expect_err(text).to_string();
        assert!(said.contains(what), "{text}: {said}");
        assert!(!said.contains("sha256:12"), "never a value: {said}");
    }
}

#[test]
fn host_history_has_defaults_takes_each_limit_and_refuses_zero_and_the_unknown() {
    let none = Config::from_str_in("", &base()).expect("parses");
    assert_eq!(none.history, HistoryConfig::default());
    assert!(none.history.record);
    let set = Config::from_str_in(
        "[host.history]\nrecord = false\nmax_session_bytes = 1000\nmax_sessions = 3\nmax_total_bytes = 5000\n",
        &base(),
    )
    .expect("parses");
    assert!(!set.history.record);
    assert_eq!(
        (
            set.history.max_session_bytes,
            set.history.max_sessions,
            set.history.max_total_bytes
        ),
        (1000, 3, 5000)
    );
    for bad in [
        "max_sessions = 0",
        "max_session_bytes = 0",
        "max_total_bytes = 0",
        "unknown = 1",
    ] {
        assert!(
            Config::from_str_in(&format!("[host.history]\n{bad}\n"), &base()).is_err(),
            "{bad}"
        );
    }
    let large = Config::from_str_in(
        "[host.history]
max_session_bytes = 999999999999
",
        &base(),
    )
    .expect("a limit has no ceiling of the host's own");
    assert_eq!(large.history.max_session_bytes, 999_999_999_999);
}

#[test]
fn host_delegate_waits_and_dials_for_as_long_as_it_is_told() {
    let none = Config::from_str_in("", &base()).expect("parses");
    assert_eq!(
        (none.delegate.wait_seconds, none.delegate.dial_seconds),
        (300, 20)
    );
    let set = Config::from_str_in(
        "[host.delegate]
wait_seconds = 7200
dial_seconds = 90
",
        &base(),
    )
    .expect("a limit has no ceiling of the host's own");
    assert_eq!(
        (set.delegate.wait_seconds, set.delegate.dial_seconds),
        (7200, 90)
    );
}

#[test]
fn host_history_and_the_hooks_tables_are_read_together() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let config = parse(&format!(
        "[host.history]\nmax_sessions = 5\n\
         [host.profiles.p]\noverlays = [\"team\"]\nallow_hooks = {{ team = \"{digest}\" }}\n\
         [agents.Claude.hooks]\nteam = \"{digest}\"\n"
    ))
    .expect("parses");
    assert_eq!(config.history.max_sessions, 5);
    assert_eq!(config.profiles["p"].allow_hooks["team"], digest);
    assert!(config.local_profiles.for_agent("Claude").is_some());
}

/// One configuration, written in each format a canoa.cfg may name on its first line.
#[cfg(all(feature = "yaml", feature = "json"))]
const SAME_TOML: &str = r#"[host]
origins = ["https://app.example"]
[host.agents.alpha]
program = "claude-agent-acp"
args = ["--acp", "x"]
listen = "127.0.0.1:7801"
max_peers = 2
[host.providers.zeta]
library = "echo.dll"
provider = "echo_provider"
[host.providers.zeta.settings]
volume = 3
loud = true
[usage]
limit_phrases = ["rate limit"]
[permissions]
protected = ["secrets/"]
[desktop]
start_hidden = true
[overlays]
team = "https://example.org/profiles.git@main#team"
[skill_pins]
review = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
"#;

const SAME_YAML: &str = r#"#!yaml
host:
  origins: ["https://app.example"]
  agents:
    alpha:
      program: claude-agent-acp
      args: ["--acp", "x"]
      listen: 127.0.0.1:7801
      max_peers: 2
  providers:
    zeta:
      library: echo.dll
      provider: echo_provider
      settings:
        volume: 3
        loud: true
usage:
  limit_phrases: [rate limit]
permissions:
  protected: [secrets/]
desktop:
  start_hidden: true
overlays:
  team: https://example.org/profiles.git@main#team
skill_pins:
  review: sha256:0000000000000000000000000000000000000000000000000000000000000000
"#;

const SAME_JSON: &str = r##"#!json
{
  "host": {
    "origins": ["https://app.example"],
    "agents": {"alpha": {"program": "claude-agent-acp", "args": ["--acp", "x"],
      "listen": "127.0.0.1:7801", "max_peers": 2, "title": null}},
    "providers": {"zeta": {"library": "echo.dll", "provider": "echo_provider",
      "settings": {"volume": 3, "loud": true}}}
  },
  "usage": {"limit_phrases": ["rate limit"]},
  "permissions": {"protected": ["secrets/"]},
  "desktop": {"start_hidden": true},
  "overlays": {"team": "https://example.org/profiles.git@main#team"},
  "skill_pins": {"review": "sha256:0000000000000000000000000000000000000000000000000000000000000000"}
}
"##;

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn the_same_configuration_loads_the_same_in_toml_yaml_and_json() {
    let toml = parse(SAME_TOML).expect("TOML parses");
    assert_eq!(toml.entries.len(), 2);
    assert_eq!(parse(SAME_YAML).expect("YAML parses"), toml);
    assert_eq!(parse(SAME_JSON).expect("JSON parses"), toml);
    let marked = format!("#!toml\n{SAME_TOML}");
    assert_eq!(parse(&marked).expect("a TOML marker"), toml);
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn a_file_is_read_in_the_format_its_name_or_first_line_gives() {
    let dir = std::env::temp_dir().join(format!("canoa-config-formats-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("directory");
    let expected = Config::from_str_in(SAME_TOML, &dir).expect("TOML parses");
    for (name, text) in [
        ("canoa.cfg", SAME_TOML),
        ("canoa.cfg", SAME_YAML),
        ("canoa.cfg", SAME_JSON),
        ("canoa.yaml", SAME_YAML.trim_start_matches("#!yaml\n")),
        ("canoa.json", SAME_JSON.trim_start_matches("#!json\n")),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write");
        assert_eq!(Config::load(&path).expect(name), expected, "{name}");
    }
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn an_unknown_format_is_refused_naming_the_ones_this_build_reads() {
    let error = parse("#!ini\n[host]\n").expect_err("refused");
    let shown = error.to_string();
    assert!(shown.contains("toml"), "{shown}");
    #[cfg(feature = "yaml")]
    assert!(shown.contains("yaml"), "{shown}");
    #[cfg(feature = "json")]
    assert!(shown.contains("json"), "{shown}");
    assert!(!shown.contains("ini"), "{shown}");
}

#[cfg(not(feature = "yaml"))]
#[test]
fn a_build_without_yaml_refuses_it_in_words() {
    let shown = parse(SAME_YAML).expect_err("refused").to_string();
    assert!(
        shown.contains("not read") && shown.contains("toml"),
        "{shown}"
    );
}

#[cfg(not(feature = "json"))]
#[test]
fn a_build_without_json_refuses_it_in_words() {
    let shown = parse(SAME_JSON).expect_err("refused").to_string();
    assert!(
        shown.contains("not read") && shown.contains("toml"),
        "{shown}"
    );
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn a_wrong_type_names_the_key_and_never_the_value_in_each_format() {
    let secret = "sk-ant-oat01-SECRETSECRET";
    for text in [
        format!("[host.agents.a]\nprogram = \"x\"\nmax_peers = \"{secret}\"\n"),
        format!(
            "#!yaml\nhost:\n  agents:\n    a:\n      program: x\n      max_peers: \"{secret}\"\n"
        ),
        format!(
            "#!json\n{{\"host\": {{\"agents\": {{\"a\": {{\"program\": \"x\", \"max_peers\": \"{secret}\"}}}}}}}}\n"
        ),
    ] {
        let error = parse(&text).expect_err("refused");
        let shown = error.to_string();
        assert!(shown.contains("max_peers"), "{shown}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(!format!("{error:?}").contains("SECRET"), "{error:?}");
    }
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn a_misspelled_setting_is_refused_without_its_text_in_each_format() {
    for text in [
        "[host.agents.a]\nprogram = \"x\"\nlsten = \"127.0.0.1:1\"\n",
        "#!yaml\nhost:\n  agents:\n    a:\n      program: x\n      lsten: 127.0.0.1:1\n",
        "#!json\n{\"host\": {\"agents\": {\"a\": {\"program\": \"x\", \"lsten\": \"y\"}}}}\n",
    ] {
        let shown = parse(text).expect_err("refused").to_string();
        assert!(shown.contains("host.agents.a"), "{shown}");
        assert!(!shown.contains("lsten"), "{shown}");
    }
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn a_syntax_error_says_where_in_each_format() {
    for (text, line) in [
        ("[host]\n[host.agents.a\n", 2),
        ("#!yaml\nhost:\n  agents: [\n", 3),
        ("#!json\n{\n  \"host\": ,\n}\n", 3),
    ] {
        let ConfigError::Parse { line: at, .. } = parse(text).expect_err("refused") else {
            panic!("a parse error: {text}");
        };
        assert_eq!(at, Some(line), "{text}");
    }
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn yaml_and_json_have_no_null_a_key_with_one_is_unset() {
    let config = parse("#!yaml\nhost:\n  agents:\n    a:\n      program: x\n      title: ~\n")
        .expect("parses");
    assert_eq!(config.entries.len(), 1);
    assert!(parse("#!yaml\nhost:\n  origins: [null]\n").is_err());
    assert!(parse("#!json\n[1]\n").is_err());
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn a_repeated_key_is_refused_in_each_format() {
    for text in [
        "[host]\n[host]\n",
        "#!yaml\nhost: {}\nhost: {}\n",
        "#!json\n{\"host\": {}, \"host\": {}}\n",
    ] {
        let shown = parse(text).expect_err("refused").to_string();
        assert!(
            shown.contains("twice") || shown.contains("duplicate"),
            "{shown}"
        );
    }
}

#[cfg(all(feature = "yaml", feature = "json"))]
fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/config")
        .join(name)
}

#[cfg(all(feature = "yaml", feature = "json"))]
#[test]
fn the_yaml_and_json_examples_load_to_the_same_configuration() {
    let yaml = Config::load(&example("canoa-yaml.cfg")).expect("YAML example");
    let json = Config::load(&example("canoa-json.cfg")).expect("JSON example");
    assert_eq!(yaml, json);
    assert_eq!(yaml.entries.len(), 1);
    assert_eq!(yaml.entries[0].name, "claude");
    assert_eq!(yaml.origins, ["https://app.example"]);
    assert_eq!(
        yaml.limit_phrases,
        Some(vec![
            "RESOURCE_EXHAUSTED".to_owned(),
            "Error 429".to_owned()
        ])
    );
    Config::load(&example("canoa.cfg")).expect("the TOML example");
}

#[test]
fn an_overlays_options_are_read_from_its_entry_for_the_host_and_every_profile() {
    let config = parse(
        "[overlays.docs]\nsource = \"x\"\noptions = { depth = \"deep\", rounds = 3, strict = true }\n\
         [overlays.plain]\nsource = \"y\"\n\
         [host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
         [host.profiles.main]\noverlays = [\"docs\"]\n",
    )
    .expect("parses");
    let expected = serde_json::json!({"depth": "deep", "rounds": 3, "strict": true});
    let local = &config.local_profiles.options;
    assert_eq!(local.len(), 1, "an overlay with no options has none");
    assert_eq!(serde_json::Value::Object(local["docs"].clone()), expected);
    assert_eq!(config.profiles["main"].options, *local);
}

#[test]
fn options_that_are_not_a_table_refuse_the_file_naming_the_overlay_and_no_value() {
    let refused = parse("[overlays.docs]\noptions = \"secret-value\"\n").expect_err("refused");
    let words = refused.to_string();
    assert!(words.contains("overlays.docs.options"), "{words}");
    assert!(!words.contains("secret-value"), "{words}");
}

#[test]
fn a2a_keys_and_agents_on_another_machine_are_read_and_a_wrong_one_is_refused() {
    let text = format!(
        "[host.delegate]\na2a = [\"{PEER}\"]\nsources = [\"far\"]\n[agents.far]\na2a = {{ url = \"ws://10.0.0.2:7000\", peer = \"{PEER}\", source = \"shared\", title = \"Far\" }}\n"
    );
    let config = parse(&text).expect("parses");
    assert_eq!(config.delegate.a2a.len(), 1);
    let far = &config.remote_agents[0];
    assert_eq!(
        (far.name.as_str(), far.url.as_str()),
        ("far", "ws://10.0.0.2:7000")
    );
    assert_eq!(far.source.as_deref(), Some("shared"));
    assert!(config.local_agents.is_empty(), "it is no program");
    assert!(parse("[host.delegate]\na2a = [\"nobody\"]\n").is_err());
    for bad in [
        format!("a2a = {{ url = \"http://x\", peer = \"{PEER}\" }}"),
        "a2a = { url = \"ws://x:1\" }".to_owned(),
        format!("a2a = {{ url = \"ws://x:1\", peer = \"{PEER}\", extra = 1 }}"),
        format!("program = \"x\"\na2a = {{ url = \"ws://x:1\", peer = \"{PEER}\" }}"),
    ] {
        assert!(parse(&format!("[agents.far]\n{bad}\n")).is_err(), "{bad}");
    }
}

#[test]
fn loadouts_are_named_collections_and_the_default_names_one_of_them() {
    let config = Config::from_str_in(
        "[overlays]\nteam = \"https://x/team.git\"\nother = \"https://x/other.git\"\n\
         [loadouts.review]\ndescription = \"Reading code\"\noverlays = [\"other\", \"team\"]\nskills = [\"Audit\", \"audit\", \"lint\"]\nmcp = [\"files\"]\n\
         [loadouts.bare]\n[defaults]\nloadout = \"review\"\ntheme = \"the app's\"\n",
        Path::new("."),
    )
    .expect("parses");
    let local = &config.local_profiles;
    assert_eq!(local.default_loadout.as_deref(), Some("review"));
    assert_eq!(
        local
            .loadouts
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["review", "bare"]
    );
    let review = &local.loadouts["review"];
    assert_eq!(review.description, "Reading code");
    assert_eq!(review.skills, ["Audit", "lint"]);
    assert_eq!(review.mcp, ["files"]);
    // Overlays apply in the order `[overlays]` composes them, not the order the loadout lists them.
    let resolved = local
        .resolve_loadout(None)
        .expect("known")
        .expect("the default");
    assert_eq!(resolved.overlays, ["team", "other"]);
    let bare = local
        .resolve_loadout(Some("bare"))
        .expect("known")
        .expect("named");
    assert!(bare.overlays.is_empty() && bare.skills.is_empty() && bare.mcp.is_empty());
}

#[test]
fn a_loadout_that_is_malformed_or_a_default_that_names_none_refuses_the_file() {
    for text in [
        "[loadouts.x]\nskills = \"audit\"\n",
        "[loadouts.x]\nskills = [1]\n",
        "[loadouts.x]\nskills = [\"../x\"]\n",
        "[loadouts.x]\noverlays = [\"not valid\"]\n",
        "[loadouts.x]\nmcp = [\"a b\"]\n",
        "[loadouts.x]\ndescription = 3\n",
        "[loadouts.x]\norder = [\"a\"]\n",
        "[loadouts.\"a b\"]\n",
        "loadouts = 1\n",
        "[defaults]\nloadout = \"x\"\n",
        "[loadouts.y]\n[defaults]\nloadout = \"x\"\n",
        "[loadouts.x]\n[defaults]\nloadout = 1\n",
    ] {
        assert!(Config::from_str_in(text, Path::new(".")).is_err(), "{text}");
    }
}

#[test]
fn a_kind_of_client_has_a_default_loadout_of_its_own() {
    let config = Config::from_str_in(
        "[agents.a]\nprogram = \"agent\"\n[loadouts.mail]\n[loadouts.docs]\n[defaults]\nloadout = \"docs\"\ntheme = \"the app's\"\n\
         [defaults.clients]\noutlook = \"mail\"\nword = \"docs\"\n",
        Path::new("."),
    )
    .expect("parses");
    let local = &config.local_profiles;
    assert_eq!(local.default_loadout.as_deref(), Some("docs"));
    assert_eq!(
        local.client_loadouts,
        BTreeMap::from([
            ("outlook".to_owned(), "mail".to_owned()),
            ("word".to_owned(), "docs".to_owned())
        ])
    );
    let profile = local.for_agent("a").expect("a profile");
    assert_eq!(profile.client_loadout("outlook"), Some("mail"));
    assert_eq!(profile.client_loadout("excel"), None);
    // Nothing written, nothing held.
    let bare = Config::from_str_in("[loadouts.mail]\n", Path::new(".")).expect("parses");
    assert!(bare.local_profiles.client_loadouts.is_empty());
}

#[test]
fn a_client_default_that_names_no_kind_or_is_not_text_refuses_the_file() {
    for text in [
        "[loadouts.x]\n[defaults.clients]\nfridge = \"x\"\n",
        "[loadouts.x]\n[defaults.clients]\noutlook = 1\n",
        "[loadouts.x]\n[defaults]\nclients = \"x\"\n",
    ] {
        assert!(Config::from_str_in(text, Path::new(".")).is_err(), "{text}");
    }
}

#[test]
fn a_client_default_that_names_a_loadout_the_file_lost_does_not_refuse_it() {
    let config = Config::from_str_in(
        "[loadouts.x]\n[defaults.clients]\noutlook = \"y\"\n",
        Path::new("."),
    )
    .expect("a hand-deleted loadout leaves the rest of the file readable");
    assert_eq!(config.local_profiles.client_loadouts["outlook"], "y");
}

#[test]
fn a_session_names_a_loadout_the_file_does_not_hold_and_it_is_refused_with_its_name() {
    let config = Config::from_str_in("[loadouts.a]\n", Path::new(".")).expect("parses");
    let error = config
        .local_profiles
        .resolve_loadout(Some("ghost"))
        .expect_err("unknown");
    assert!(error.to_string().contains("ghost"), "{error}");
    // Nothing asked and no default: the profile's own, unnarrowed.
    assert_eq!(config.local_profiles.resolve_loadout(None), Ok(None));
}

#[test]
fn an_entry_may_name_a_built_in_transport_and_the_endpoint_it_listens_on() {
    let text = "[host.agents.a]\nprogram = \"x\"\ntransport = \"websocket\"\nlisten = \"ws://127.0.0.1:0\"\n\
                transport_settings = { listen = { frames = \"binary\" } }\n";
    let config = parse(text).expect("a built-in transport is accepted");
    let Listen::Transport(spec) = &config.entries[0].listen else {
        panic!("a transport: {:?}", config.entries[0].listen);
    };
    assert_eq!(spec.id, "websocket");
    assert_eq!(spec.endpoint, "ws://127.0.0.1:0");
    assert_eq!(spec.origin, None);
    assert!(spec.settings.is_some());
    assert_eq!(config.entries[0].listen.to_string(), "ws://127.0.0.1:0");
    assert_eq!(config.entries[0].listen.address(), None);
}

#[test]
fn an_entry_with_no_transport_key_listens_as_before() {
    let config =
        parse("[host.agents.a]\nprogram = \"x\"\nlisten = \"127.0.0.1:7801\"\n").expect("parses");
    assert!(matches!(config.entries[0].listen, Listen::Address(_)));
    let config = parse("[host.agents.a]\nprogram = \"x\"\n").expect("parses");
    assert!(matches!(config.entries[0].listen, Listen::Address(_)));
}

#[test]
fn an_outside_transport_loads_from_the_library_or_source_its_own_keys_name() {
    let text = "[host.providers.p]\nlibrary = \"/p.dll\"\nprovider = \"echo\"\ntransport = \"mine\"\n\
                transport_source = \"/s.dll\"\nlisten = \"mine:1\"\n\
                [host.providers.p.transport_source_settings]\ndirs = [\"/d\"]\n";
    let config = parse(text).expect("an outside transport is accepted");
    let Listen::Transport(spec) = &config.entries[0].listen else {
        panic!("a transport: {:?}", config.entries[0].listen);
    };
    assert!(matches!(spec.origin, Some(Origin::Source { .. })));
    // The entry's own library is the provider's, and is untouched by the transport's.
    let EntryKind::Provider { origin, .. } = &config.entries[0].kind else {
        panic!("a provider");
    };
    assert!(matches!(origin, Origin::Library(_)));
}

#[test]
fn a_tool_entry_may_name_a_transport_too() {
    let tool = "runs = \"session\"\nunasked = \"x\"\nprompt = { type = \"prompt\", message = \"{prompt}\" }\n";
    let text = format!(
        "[host.tools.t]\nprogram = \"pi\"\ntransport = \"pipe\"\nlisten = \"pipe:t\"\n{tool}"
    );
    let config = parse(&text).expect("a tool names a transport");
    assert!(matches!(config.entries[0].listen, Listen::Transport(_)));
    let text =
        format!("[host.tools.t]\nprogram = \"pi\"\ntransport_settings = {{ a = 1 }}\n{tool}");
    let refused = parse(&text).expect_err("a stray transport key");
    assert!(
        refused.to_string().contains("goes with transport"),
        "{refused}"
    );
}

#[test]
fn a_transport_that_is_misnamed_or_half_given_is_refused_naming_the_entry_and_the_key() {
    let cases = [
        // No endpoint to listen on.
        ("transport = \"pipe\"\n", "listen"),
        ("transport = \"pipe\"\nlisten = \"\"\n", "listen is empty"),
        // An id nothing built in offers, with nowhere to load it from.
        (
            "transport = \"mine\"\nlisten = \"x\"\n",
            "transport is pipe",
        ),
        ("transport = \"\"\nlisten = \"x\"\n", "must not be empty"),
        // Built in: nothing to load.
        (
            "transport = \"pipe\"\ntransport_library = \"/t.dll\"\nlisten = \"x\"\n",
            "built in",
        ),
        // Keys of a transport that is not named.
        ("transport_settings = { a = 1 }\n", "goes with transport"),
        ("transport_library = \"/t.dll\"\n", "goes with transport"),
        (
            "transport = \"mine\"\ntransport_library = \"/t.dll\"\ntransport_source = \"/s.dll\"\nlisten = \"x\"\n",
            "exclude each other",
        ),
        (
            "transport = \"mine\"\ntransport_library = \"/t.dll\"\ntransport_import_search = \"path\"\nlisten = \"x\"\n",
            "import_search goes with source",
        ),
    ];
    for (keys, said) in cases {
        let refused = parse(&format!("[host.agents.a]\nprogram = \"x\"\n{keys}")).expect_err(keys);
        let text = refused.to_string();
        assert!(text.contains('a'), "{text}");
        assert!(text.contains(said), "{keys}: {text}");
    }
}

#[test]
fn two_entries_cannot_listen_on_one_transport_endpoint() {
    let text = "[host.agents.a]\nprogram = \"x\"\ntransport = \"pipe\"\nlisten = \"pipe:t\"\n\
                [host.agents.b]\nprogram = \"x\"\ntransport = \"pipe\"\nlisten = \"pipe:t\"\n";
    let refused = parse(text).expect_err("refused");
    assert!(refused.to_string().contains("same address"), "{refused}");
}

#[test]
fn transport_entries_conflict_by_endpoint_and_two_asking_for_port_zero_do_not() {
    let entry = |name: &str, id: &str, listen: &str, extra: &str| {
        format!(
            "[host.agents.{name}]\nprogram = \"x\"\ntransport = \"{id}\"\nlisten = \"{listen}\"\n{extra}"
        )
    };
    // Port 0 takes whatever is free, so two entries may both ask for it.
    let both = format!(
        "{}{}",
        entry("a", "websocket", "ws://127.0.0.1:0", ""),
        entry("b", "websocket", "ws://127.0.0.1:0/", "")
    );
    assert!(parse(&both).is_ok());
    // One endpoint is one place, whatever else the entries set.
    let same = format!(
        "{}{}",
        entry("a", "websocket", "ws://127.0.0.1:7801", ""),
        entry(
            "b",
            "websocket",
            "ws://127.0.0.1:7801",
            "transport_settings = { listen = { frames = \"binary\" } }\n"
        )
    );
    let refused = parse(&same).expect_err("one endpoint, two entries");
    assert!(refused.to_string().contains("same address"), "{refused}");
    let pipes = format!(
        "{}{}",
        entry("a", "pipe", "pipe:t", ""),
        entry("b", "pipe", "pipe:t", "")
    );
    assert!(parse(&pipes).is_err());
}

#[test]
fn host_native_is_on_with_a_narrow_control_by_default_and_takes_a_list_that_never_installs() {
    use crate::control::Grant;
    let none = Config::from_str_in("", &base()).expect("parses");
    assert!(none.native.enabled);
    assert_eq!(none.native.control, NativeConfig::default_control());
    assert!(
        none.native.control.contains(&Grant::Discover),
        "a browser asks its host who is nearby by default"
    );
    assert!(
        none.native.control.contains(&Grant::ToolsProvide),
        "a browser may offer its tools by default"
    );
    for never in [
        Grant::ConfigWriteSessions,
        Grant::OverlayWrite,
        Grant::HostRestart,
        Grant::Upload,
        Grant::Terminal,
    ] {
        assert!(!none.native.control.contains(&never), "{never:?}");
    }
    let off = Config::from_str_in("[host.native]\nenabled = false\n", &base()).expect("parses");
    assert!(!off.native.enabled);
    let narrow = Config::from_str_in("[host.native]\ncontrol = [\"ledger.read\"]\n", &base())
        .expect("parses");
    assert_eq!(
        narrow.native.control,
        [Grant::LedgerRead].into_iter().collect()
    );
    let empty = Config::from_str_in("[host.native]\ncontrol = []\n", &base()).expect("parses");
    assert!(
        empty.native.control.is_empty(),
        "an empty list grants nothing"
    );
    for (text, what) in [
        (
            "[host.native]\ncontrol = [\"host.restart\"]\n",
            "host.native",
        ),
        (
            "[host.native]\ncontrol = [\"config.write.sessions\"]\n",
            "host.native",
        ),
        (
            "[host.native]\ncontrol = [\"overlay.write\"]\n",
            "host.native",
        ),
        ("[host.native]\ncontrol = [\"upload\"]\n", "host.native"),
        (
            "[host.native]\ncontrol = [\"files.write\"]\n",
            "host.native",
        ),
        (
            "[host.native]\ncontrol = [\"files.write.host\"]\n",
            "host.native",
        ),
        ("[host.native]\ncontrol = [\"terminal\"]\n", "host.native"),
        (
            "[host.native]\ncontrol = [\"voice.setup\"]\n",
            "host.native",
        ),
        ("[host.native]\ncontrol = [\"nonsense\"]\n", "host.native"),
        ("[host.native]\nlisten = \"x\"\n", "native"),
    ] {
        let said = parse(text).expect_err(text).to_string();
        assert!(said.contains(what), "{text}: {said}");
    }
}

#[test]
fn the_store_index_takes_a_ttl_in_seconds_and_a_byte_budget() {
    let none = parse("[host.store]\nprotected = []\n").expect("parses");
    assert_eq!(
        (none.store.index_ttl, none.store.index_cache_bytes),
        (None, None)
    );
    let set = parse("[host.store]\nindex_ttl = 0\nindex_cache_bytes = 1024\n").expect("parses");
    assert_eq!(set.store.index_ttl, Some(0));
    assert_eq!(set.store.index_cache_bytes, Some(1024));
    assert!(parse("[host.store]\nindex_ttl = \"5m\"\n").is_err());
}

#[test]
fn host_usage_keeps_four_hundred_days_unless_told_and_refuses_zero_and_the_unknown() {
    let none = Config::from_str_in("", &base()).expect("parses");
    assert_eq!(none.usage, UsageConfig::default());
    assert_eq!(none.usage.keep_days, 400);
    let set = Config::from_str_in("[host.usage]\nkeep_days = 30\n", &base()).expect("parses");
    assert_eq!(set.usage.keep_days, 30);
    // The app's own `[usage]` table is another table and is let go.
    assert!(Config::from_str_in("[usage]\nwarn_permille = 800\n", &base()).is_ok());
    for bad in ["keep_days = 0", "keep_days = -1", "unknown = 1"] {
        assert!(
            Config::from_str_in(&format!("[host.usage]\n{bad}\n"), &base()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn the_store_tools_are_on_unless_the_file_says_false() {
    let none = parse("[host.store]\nprotected = []\n").expect("parses");
    assert_eq!(none.store.tools, None, "absent is the default, which is on");
    assert_eq!(
        parse("[host.store]\ntools = false\n")
            .expect("parses")
            .store
            .tools,
        Some(false)
    );
    assert!(parse("[host.store]\ntools = \"no\"\n").is_err());
}

#[test]
fn host_mail_names_accounts_that_hold_no_password_and_refuses_what_would_send_one_in_the_clear() {
    let none = parse("").expect("parses");
    assert!(none.mail.accounts.is_empty() && none.mail.tools.is_none());
    let both = parse(
        "[host.mail]\ntools = true\n\
         [host.mail.accounts.work]\nbackend = \"ews\"\nemail = \"ana@example.com\"\nuser = \"EXAMPLE\\\\ana\"\npassword_env = \"WORK_PW\"\n\
         [host.mail.accounts.desktop]\nbackend = \"outlook\"\nprofile = \"Work\"\ntimeout_secs = 90\n",
    )
    .expect("parses");
    let names: Vec<&str> = both.mail.accounts.keys().map(String::as_str).collect();
    assert_eq!(names, ["work", "desktop"], "the file's order");
    let work = &both.mail.accounts["work"];
    assert_eq!(work.backend, "ews");
    assert_eq!(work.password_env.as_deref(), Some("WORK_PW"));
    assert_eq!(work.timeout_secs, 60);
    assert_eq!(work.settings.as_map()["email"], "ana@example.com");
    let desktop = &both.mail.accounts["desktop"];
    assert_eq!(
        (desktop.backend.as_str(), desktop.timeout_secs),
        ("outlook", 90)
    );
    let stored = parse(
        "[host.mail.accounts.s]\nbackend = \"ews\"\nserver = \"https://mail.example.com/EWS/Exchange.asmx\"\nuser = \"u\"\nid = \"0b8d-1\"\n",
    )
    .expect("a stored password is named by its id");
    let stored = &stored.mail.accounts["s"];
    assert_eq!(
        (stored.id.as_deref(), stored.password_env.as_deref()),
        (Some("0b8d-1"), None)
    );
    let custom = parse(
        "[host.mail.accounts.fastmail]\nbackend = \"imap\"\nhost = \"imap.fastmail.com\"\nuser = \"ana@fastmail.com\"\nid = \"fm-1\"\n",
    )
    .expect("any custom mail_backend id can be configured in canoa.cfg");
    assert_eq!(custom.mail.accounts["fastmail"].backend, "imap");
    let slow = parse(
        "[host.mail.accounts.slow]
backend = \"outlook\"
timeout_secs = 86400
",
    )
    .expect("a timeout has no ceiling of the host's own");
    assert_eq!(slow.mail.accounts["slow"].timeout_secs, 86400);

    for (bad, why) in [
        (
            "backend = \"ews\"\nemail = \"a@b\"\nuser = \"u\"\nid = \"\"",
            "canoa mail add",
        ),
        (
            "backend = \"ews\"\nemail = \"a@b\"\nuser = \"u\"\npassword = \"hunter2\"",
            "a setting this host does not",
        ),
        ("", "backend must be"),
        ("backend = \"My Backend\"", "backend must be"),
        ("backend = \"outlook\"\ntimeout_secs = 0", "timeout_secs"),
        (
            "backend = \"outlook\"\npassword = \"x\"",
            "a setting this host does not",
        ),
    ] {
        let error = parse(&format!("[host.mail.accounts.a]\n{bad}\n")).expect_err(bad);
        assert!(error.to_string().contains(why), "{bad}: {error}");
    }
    assert!(parse("[host.mail.accounts.Bad_Name]\nbackend = \"outlook\"\n").is_err());
    assert!(parse("[host.mail]\ntools = \"no\"\n").is_err());
}

#[test]
fn host_shell_is_a_program_name_and_defaults_to_the_persons_own() {
    let none = Config::from_str_in("", &base()).expect("parses");
    assert_eq!(none.shell, None);
    let named = Config::from_str_in("[host]\nshell = \"pwsh\"\n", &base()).expect("parses");
    assert_eq!(named.shell.as_deref(), Some("pwsh"));
    let spaced = Config::from_str_in(
        "[host]\nshell = 'C:\\Program Files\\PowerShell\\7\\pwsh.exe'\n",
        &base(),
    )
    .expect("parses");
    assert_eq!(
        spaced.shell.as_deref(),
        Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe")
    );
    for text in [
        "[host]\nshell = \"\"\n",
        "[host]\nshell = \"a\\u0001b\"\n",
        "[host]\nshell = \"a\\u0000b\"\n",
    ] {
        let error = Config::from_str_in(text, &base()).expect_err("refused");
        assert!(error.to_string().contains("shell"), "{error}");
    }
}

#[test]
fn the_secrets_table_is_carried_for_who_keeps_the_accounts_and_the_old_account_store_key_is_gone() {
    assert_eq!(parse("").expect("parses").secrets, None);
    let config = parse("[secrets]\naccounts = \"file\"\n").expect("parses");
    assert_eq!(
        config.secrets,
        Some(serde_json::json!({"accounts": "file"}))
    );
    let refused = parse("[host]\naccount_store = \"file\"\n").expect_err("refused");
    let refused = refused.to_string();
    assert!(
        refused.contains("host: account_store is not a setting this host knows"),
        "{refused}"
    );
    assert!(
        refused.contains("`accounts` under `[secrets]`"),
        "{refused}"
    );
    // Any other key is not told: it may be a secret pasted in the wrong place.
    let refused = parse("[host]\nunheard_of = \"hunter2\"\n")
        .expect_err("refused")
        .to_string();
    assert!(
        refused.contains("host: a setting this host does not know"),
        "{refused}"
    );
    assert!(
        !refused.contains("unheard_of") && !refused.contains("hunter2"),
        "{refused}"
    );
}

#[test]
fn a_harness_entry_configures_its_account_provider_apart_from_the_harness() {
    let config = parse(
        "[host.agents.Claude]\nharness = \"claude\"\naccount_settings = { renew_poll_ms = 100 }\n",
    )
    .expect("parses");
    let EntryKind::Harness {
        settings,
        account_settings,
        ..
    } = &config.entry("Claude").expect("there").kind
    else {
        panic!("a harness entry");
    };
    assert!(settings.is_none());
    let account_settings = account_settings.as_ref().expect("the account provider's");
    assert_eq!(account_settings.as_map()["renew_poll_ms"], 100);
    let refused =
        parse("[host.agents.a]\nprogram = \"x\"\naccount_settings = { renew_poll_ms = 1 }\n")
            .expect_err("a program has no account provider");
    assert!(
        refused.to_string().contains("account_settings"),
        "{refused}"
    );
}

#[test]
fn the_name_local_is_taken_by_the_automatic_account_and_an_entry_may_name_it() {
    let refused = parse(
        "[host.accounts.local]
kind = \"claude-login\"
id = \"id-local\"
",
    )
    .expect_err("taken");
    assert!(refused.to_string().contains("taken"), "{refused}");
    let config = parse(
        "[host.accounts.work]
kind = \"claude-login\"
id = \"id-work\"
[host.agents.Claude]
harness = \"claude\"
accounts = [\"work\", \"local\"]
",
    )
    .expect("an entry names it among its accounts");
    let EntryKind::Harness { accounts, .. } = &config.entry("Claude").expect("there").kind else {
        panic!("a harness entry");
    };
    assert_eq!(accounts, &["work", "local"]);
}

const TWO_ACCOUNTS: &str = "[host.accounts.one]
kind = \"anthropic-api-key\"
id = \"id-one\"
[host.accounts.two]
kind = \"anthropic-api-key\"
id = \"id-two\"
";

#[test]
fn a_pool_names_a_standard_and_the_accounts_it_holds_and_an_entry_may_offer_it() {
    let config = parse(&format!(
        "{TWO_ACCOUNTS}[host.pools.keys]
standard = \"anthropic-api\"
accounts = [\"two\", \"one\", \"local\"]
[host.pools.empty]
standard = \"openai-api\"
[host.agents.Claude]
harness = \"claude\"
accounts = [\"keys\", \"any-anthropic-api\", \"one\"]
"
    ))
    .expect("parses");
    let keys = &config.pools["keys"];
    assert_eq!(keys.standard, "anthropic-api");
    assert_eq!(
        keys.accounts,
        ["two", "one", "local"],
        "the order is the pool's"
    );
    assert!(config.pools["empty"].accounts.is_empty());
    let order: Vec<&String> = config.pools.keys().collect();
    assert_eq!(order, ["keys", "empty"], "the file's order");
    let EntryKind::Harness { accounts, .. } = &config.entry("Claude").expect("there").kind else {
        panic!("a harness entry");
    };
    assert_eq!(accounts, &["keys", "any-anthropic-api", "one"]);
}

#[test]
fn a_pool_that_names_what_does_not_exist_or_is_taken_is_refused_by_name() {
    let refused = |extra: &str| {
        parse(&format!("{TWO_ACCOUNTS}{extra}"))
            .expect_err("refused")
            .to_string()
    };
    // A member is an account the file holds, or the automatic local one.
    let why = refused("[host.pools.p]\nstandard = \"anthropic-api\"\naccounts = [\"nobody\"]\n");
    assert!(why.contains("pool p") && why.contains("nobody"), "{why}");
    let why =
        refused("[host.pools.p]\nstandard = \"anthropic-api\"\naccounts = [\"one\", \"one\"]\n");
    assert!(why.contains("twice"), "{why}");
    // A pool and an account do not share a name, and nor do a name and the host's own pools.
    let why = refused("[host.pools.one]\nstandard = \"anthropic-api\"\n");
    assert!(why.contains("pool one") && why.contains("account"), "{why}");
    let why = refused("[host.pools.local]\nstandard = \"anthropic-api\"\n");
    assert!(why.contains("pool local"), "{why}");
    let why = refused("[host.pools.any-openai-api]\nstandard = \"openai-api\"\n");
    assert!(why.contains("taken"), "{why}");
    let taken = parse("[host.accounts.any-openai-api]\nkind = \"codex-api-key\"\nid = \"id-a\"\n")
        .expect_err("an account may not take the host's pool's name");
    assert!(taken.to_string().contains("taken"), "{taken}");
    // The standard is any id the standards vocabulary allows, and the file has to say it.
    let why = refused("[host.pools.p]\n");
    assert!(why.contains("standard is missing"), "{why}");
    let why = refused("[host.pools.p]\nstandard = \"Not A Standard\"\n");
    assert!(why.contains("standard"), "{why}");
    let why = refused("[host.pools.p]\nstandard = \"anthropic-api\"\napi_key = \"sk-secret\"\n");
    assert!(!why.contains("sk-secret"), "{why}");
    // An entry names an account, a pool, or the host's own pool of some standard, and nothing else.
    let why = refused("[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"nothing\"]\n");
    assert!(why.contains("pool"), "{why}");
    parse(&format!(
        "{TWO_ACCOUNTS}[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"any-codex\"]\n"
    ))
    .expect("the host's own pool of a standard is named without being in the file");
}

#[test]
fn a_shared_entrys_permission_window_is_off_unless_set_and_needs_shared() {
    let config = parse(
        "[host.agents.a]
program = \"x\"
shared = true
[host.agents.b]
program = \"x\"
shared = true
shared_permission_window_seconds = 0
[host.agents.c]
program = \"x\"
shared = true
shared_permission_window_seconds = 120
",
    )
    .expect("parses");
    let windows: Vec<_> = config
        .entries
        .iter()
        .map(|entry| entry.shared_permission_window)
        .collect();
    assert_eq!(
        windows,
        [None, None, Some(core::time::Duration::from_secs(120))]
    );
    assert!(
        parse(
            "[host.agents.a]
program = \"x\"
shared_permission_window_seconds = 5
"
        )
        .is_err()
    );
}

#[test]
fn a_shared_entrys_log_is_kept_unless_turned_off_and_its_bounds_need_shared() {
    let config = parse(
        "[host.agents.a]
program = \"x\"
shared = true
[host.agents.b]
program = \"x\"
shared = true
shared_keep_log = false
shared_log_entries = 50
shared_log_bytes = 4096
",
    )
    .expect("parses");
    let kept: Vec<_> = config
        .entries
        .iter()
        .map(|entry| {
            (
                entry.shared_keep_log,
                entry.shared_log_entries,
                entry.shared_log_bytes,
            )
        })
        .collect();
    assert_eq!(kept, [(true, None, None), (false, Some(50), Some(4096))]);
    for key in [
        "shared_keep_log = false",
        "shared_log_entries = 5",
        "shared_log_bytes = 5",
    ] {
        let text = format!("[host.agents.a]\nprogram = \"x\"\n{key}\n");
        assert!(parse(&text).is_err(), "{key} without shared = true");
    }
}

#[test]
fn the_office_add_in_is_off_unless_the_web_table_asks_for_it() {
    let web = |text: &str| {
        parse(&format!("[host.web]\n{text}"))
            .expect("parses")
            .web
            .expect("a web table")
    };
    let off = web("");
    assert!(!off.office);
    assert!(off.office_frames.is_empty());
    let on = web("office = true\noffice_frames = [\"https://*.officeapps.live.com\"]\n");
    assert!(on.office);
    assert_eq!(on.office_frames, ["https://*.officeapps.live.com"]);
}

#[test]
fn an_office_frame_needs_the_add_in_and_is_one_https_origin() {
    for text in [
        "office_frames = [\"https://a.example\"]\n",
        "office = false\noffice_frames = [\"https://a.example\"]\n",
        "office = true\noffice_frames = [\"http://a.example\"]\n",
        "office = true\noffice_frames = [\"https://*.com\"]\n",
        "office = true\noffice_frames = [\"https://a.example; script-src *\"]\n",
        "office = true\noffice_frames = [\"*\"]\n",
        "office = \"yes\"\n",
    ] {
        let text = format!("[host.web]\n{text}");
        let said = parse(&text).expect_err(&text).to_string();
        assert!(!said.contains("script-src"), "{said}");
    }
}

#[test]
fn the_add_in_admits_its_own_page_only_when_asked_and_only_on_loopback() {
    let web = |text: &str| parse(&format!("[host.web]\n{text}"));
    assert!(!web("office = true\n").unwrap().web.unwrap().office_local);
    assert!(
        web("office = true\noffice_local = true\n")
            .unwrap()
            .web
            .unwrap()
            .office_local
    );
    for text in [
        "office_local = true\n",
        "office = true\noffice_local = true\nlisten = \"0.0.0.0:7800\"\ncert = \"a.crt\"\nkey = \"a.key\"\n",
        "office = true\noffice_local = \"yes\"\n",
    ] {
        assert!(web(text).is_err(), "{text}");
    }
}

#[test]
fn the_add_ins_secret_is_off_unless_asked_and_needs_the_page_to_be_admitted() {
    let web = |text: &str| parse(&format!("[host.web]\n{text}"));
    let on = web("office = true\noffice_local = true\n")
        .unwrap()
        .web
        .unwrap();
    assert!(
        !on.office_secret,
        "off by default: the computer is the person's own"
    );
    let asked = web("office = true\noffice_local = true\noffice_secret = true\n")
        .unwrap()
        .web
        .unwrap();
    assert!(asked.office_secret);
    for text in [
        "office_secret = true\n",
        "office = true\noffice_secret = true\n",
        "office = true\noffice_local = true\noffice_secret = \"yes\"\n",
    ] {
        assert!(web(text).is_err(), "{text}");
    }
}

#[test]
fn a_control_channel_with_no_key_is_fine_when_the_add_in_page_is_admitted() {
    let channel = "[host.control]\nlisten = \"127.0.0.1:0\"\n";
    assert!(
        parse(channel).is_err(),
        "no key at all turns every key away"
    );
    assert!(
        parse(&format!(
            "{channel}[host.web]\noffice = true\noffice_local = true\n"
        ))
        .is_ok()
    );
    assert!(
        parse(&format!("{channel}[host.web]\noffice = true\n")).is_err(),
        "the add-in alone admits nobody"
    );
}

#[test]
fn with_no_network_table_the_host_follows_the_system_in_everything() {
    let none = parse("[host]\n").expect("parses");
    assert_eq!(none.network, NetworkConfig::default());
    assert_eq!(none.network.proxy, canoa::network::ProxyChoice::System);
    assert_eq!(
        none.network.client_certificate,
        canoa::network::ClientCertificate::System
    );
    assert_eq!(none.network.git_tls, canoa::network::GitTls::System);
    assert!(none.network.no_proxy.is_none());
    assert_eq!(none.network.proxy_idle_seconds, None);
    let settings = none.network.settings().expect("settings");
    assert_eq!(settings.proxy, canoa::network::ProxyChoice::System);
}

#[test]
fn the_network_table_overrides_each_part_and_every_part_is_explicit() {
    let config = parse(
        "[host.network]\nproxy = \"http://gateway.corp:3128\"\nproxy_user = \"alice\"\nproxy_password_env = \"CANOA_TEST_NO_SUCH_VARIABLE\"\n\
         no_proxy = \"a.example, *.corp\\n10.0.0.0/8\"\nnegotiate = false\nclient_certificate = \"me.pem\"\nca_file = \"roots.pem\"\n\
         git_tls = \"git\"\nendpoints = [\"https://tools.example/\"]\nproxy_idle_seconds = 90\n",
    )
    .expect("parses");
    let network = &config.network;
    assert_eq!(
        network.proxy,
        canoa::network::ProxyChoice::Proxy("http://gateway.corp:3128".to_owned())
    );
    assert_eq!(network.proxy_user.as_deref(), Some("alice"));
    assert_eq!(
        network.no_proxy,
        Some(vec![
            "a.example".to_owned(),
            "*.corp".to_owned(),
            "10.0.0.0/8".to_owned()
        ])
    );
    assert_eq!(network.negotiate, Some(false));
    assert!(matches!(
        &network.client_certificate,
        canoa::network::ClientCertificate::File(path) if path.ends_with("me.pem") && path.is_absolute()
    ));
    assert!(
        network
            .ca_file
            .as_ref()
            .is_some_and(|path| path.ends_with("roots.pem"))
    );
    assert_eq!(network.git_tls, canoa::network::GitTls::Git);
    assert_eq!(network.endpoints, ["https://tools.example/"]);
    assert_eq!(network.proxy_idle_seconds, Some(90));
    // The password is read when the network is made, from the variable the file names, never from the file.
    assert!(network.settings().is_err());
}

#[test]
fn the_network_proxy_is_system_none_a_pac_location_or_a_proxy() {
    let proxy = |text: &str| {
        parse(&format!("[host.network]\nproxy = \"{text}\"\n"))
            .expect("parses")
            .network
            .proxy
    };
    assert_eq!(proxy("system"), canoa::network::ProxyChoice::System);
    assert_eq!(proxy("none"), canoa::network::ProxyChoice::None);
    assert_eq!(
        proxy("pac:http://wpad.corp/proxy.pac"),
        canoa::network::ProxyChoice::Pac("http://wpad.corp/proxy.pac".to_owned())
    );
    assert_eq!(
        parse("[host.network]\nclient_certificate = \"none\"\n")
            .expect("parses")
            .network
            .client_certificate,
        canoa::network::ClientCertificate::None
    );
    assert!(parse("[host.network]\nproxy = \"\"\n").is_err());
    assert!(parse("[host.network]\nproxy = \"pac:\"\n").is_err());
    assert!(parse("[host.network]\ngit_tls = \"openssl\"\n").is_err());
    assert!(parse("[host.network]\nproxy_password_env = \"X\"\n").is_err());
    assert!(parse("[host.network]\nother = 1\n").is_err());
}

#[test]
fn the_default_profile_is_always_there_and_is_the_one_a_session_with_none_starts_with() {
    let none = parse(
        "[profiles.other]
model = \"x\"
",
    )
    .expect("parses");
    let profile = none.local_profiles.for_agent("a").expect("a profile");
    // With no `[profiles.default]` the default sets nothing, and naming it is not an error.
    assert_eq!(profile.default_preset(), &Preset::default());
    assert_eq!(
        profile.resolve_preset(crate::config::DEFAULT_PRESET),
        Ok(&Preset::default())
    );
    assert_eq!(
        profile.resolve_preset("ghost"),
        Err(crate::config::UnknownPreset("ghost".to_owned()))
    );
    // With one, it is that table's.
    let set =
        parse("[profiles.default]\nmodel = \"m\"\ncontext = \"Be brief.\"\n").expect("parses");
    let profile = set.local_profiles.for_agent("a").expect("a profile");
    assert_eq!(profile.default_preset().model.as_deref(), Some("m"));
    assert_eq!(
        profile
            .resolve_preset("default")
            .expect("held")
            .context
            .as_deref(),
        Some("Be brief.")
    );
}

#[test]
fn the_longest_message_is_a_setting_with_no_ceiling_of_its_own() {
    let program = "[host.agents.a]\nprogram = \"x\"\n";
    assert_eq!(
        parse(program).expect("parses").max_message_bytes,
        DEFAULT_MAX_MESSAGE_BYTES
    );
    let huge = format!("[host]\nmax_message_bytes = 68719476736\n{program}");
    assert_eq!(
        parse(&huge).expect("parses").max_message_bytes,
        68_719_476_736
    );
    assert!(parse(&format!("[host]\nmax_message_bytes = 0\n{program}")).is_err());
}

#[test]
fn a_config_made_by_default_takes_messages_of_the_default_size() {
    assert_eq!(
        Config::default().max_message_bytes,
        DEFAULT_MAX_MESSAGE_BYTES
    );
    assert_eq!(Config::default().openers, None);
}

#[test]
fn host_openers_accepts_custom_editors_and_refuses_bad_entries() {
    let cfg = parse(
        r#"
[[host.openers]]
id = "cursor"
label = "Open in Cursor"
program = "cursor"
args = ["--reuse-window", "{path}"]

[[host.openers]]
id = "reveal"
label = "Reveal in tree"
program = "xdg-open"
args = ["{parent}"]
item = true
"#,
    )
    .expect("parses");
    let openers = cfg.openers.expect("configured");
    assert_eq!(openers.len(), 2);
    assert_eq!(openers[0].id, "cursor");
    assert_eq!(openers[0].args, ["--reuse-window", "{path}"]);
    assert_eq!(openers[0].item, None);
    assert_eq!(openers[1].item, Some(true));

    for bad in [
        "[[host.openers]]\nid = \"bad id\"\nlabel = \"X\"\nprogram = \"x\"\n",
        "[[host.openers]]\nid = \"x\"\nlabel = \"\"\nprogram = \"x\"\n",
        "[[host.openers]]\nid = \"x\"\nlabel = \"X\"\nprogram = \"x\"\nargs = [\"--no-path\"]\n",
        "[[host.openers]]\nid = \"a\"\nlabel = \"A\"\nprogram = \"a\"\n[[host.openers]]\nid = \"a\"\nlabel = \"B\"\nprogram = \"b\"\n",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

/// A canoa.cfg with the tables and keys a person's application writes, and a Claude and a Codex entry, loads as before: the
/// harness names are the providers' ids, which the providers' own change leaves alone.
#[test]
fn a_config_with_the_claude_and_codex_harness_entries_reads_as_it_did() {
    let config =
        parse(include_str!("../../tests/fixtures/owner_cfg/canoa.cfg")).expect("the file parses");
    let harness_of = |name: &str| match &config.entry(name).expect("the entry is there").kind {
        EntryKind::Harness {
            harness,
            executable,
            accounts,
            profiles,
            ..
        } => {
            assert!(executable.is_none() && accounts.is_empty() && profiles.is_empty());
            harness.clone()
        }
        other => panic!("{name} is {other:?}"),
    };
    assert_eq!(harness_of("Claude"), "claude");
    assert_eq!(harness_of("Codex"), "codex");
    assert_eq!(config.entries.len(), 2);
}

#[test]
fn listing_mcp_servers_is_held_to_settings_with_no_ceiling_of_their_own() {
    let program = "[host.agents.a]\nprogram = \"x\"\n";
    let defaults = parse(program).expect("parses").mcp_list;
    assert_eq!(defaults, McpListConfig::default());
    assert_eq!(defaults.query_seconds, 5);
    assert_eq!(defaults.max_items_per_server, 1000);
    assert_eq!(defaults.max_reply_bytes, 4 * 1024 * 1024);
    let raised = parse(&format!(
        "[host.mcp_list]\nquery_seconds = 600\nmax_items_per_server = 1000000\nmax_reply_bytes = 68719476736\n{program}"
    ))
    .expect("parses")
    .mcp_list;
    assert_eq!(raised.query_seconds, 600);
    assert_eq!(raised.max_items_per_server, 1_000_000);
    assert_eq!(raised.max_reply_bytes, 68_719_476_736);
    for key in ["query_seconds", "max_items_per_server", "max_reply_bytes"] {
        assert!(parse(&format!("[host.mcp_list]\n{key} = 0\n{program}")).is_err());
    }
    assert!(parse(&format!("[host.mcp_list]\nmax_pending = 1\n{program}")).is_err());
}

#[test]
fn keep_awake_is_off_unless_the_power_table_of_settings_turns_it_on() {
    let program = "[host.agents.a]\nprogram = \"x\"\n";
    assert!(!Config::default().keep_awake);
    assert!(!parse(program).expect("parses").keep_awake);
    let with = |table: &str| parse(&format!("{table}\n{program}"));
    assert!(
        !with("[settings.power]\nkeep_awake = false\n")
            .expect("parses")
            .keep_awake
    );
    assert!(
        with("[settings.power]\nkeep_awake = true\n")
            .expect("parses")
            .keep_awake
    );
    // Another section's table is not the host's to read.
    assert!(
        !with("[settings.region]\nkeep_awake = true\n")
            .expect("parses")
            .keep_awake
    );
    assert!(with("[settings.power]\nkeep_awake = \"yes\"\n").is_err());
}
