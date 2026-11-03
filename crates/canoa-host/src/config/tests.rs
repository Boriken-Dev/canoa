#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;

const PEER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn base() -> PathBuf {
    std::env::temp_dir().join("canoa-host-config-test")
}

fn parse(text: &str) -> Result<Config, ConfigError> {
    Config::from_str_in(text, &base())
}

#[test]
fn a_file_with_no_host_table_serves_nothing() {
    let config = parse("[desktop]\nstart_hidden = true\n[appearance]\nlook = \"bubbles\"\n")
        .expect("parses");
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
    } = origin
    else {
        panic!("a source: {origin:?}");
    };
    assert_eq!(
        file,
        std::path::absolute(base().join("guatiao_source_py.dll")).expect("absolute")
    );
    assert_eq!(
        serde_json::Value::Object(source.as_map().clone()),
        serde_json::json!({"dirs": ["python"], "paths": ["/opt/guatiao", "kinds"]})
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
fn listening_beyond_loopback_is_refused() {
    for listen in ["0.0.0.0:7801", "192.0.2.1:7801", "[::]:7801"] {
        let text = format!("[host.agents.a]\nprogram = \"x\"\nlisten = \"{listen}\"\n");
        let error = parse(&text).expect_err("refused");
        assert!(error.to_string().contains("loopback"), "{error}");
    }
    parse("[host.agents.a]\nprogram = \"x\"\nlisten = \"[::1]:7801\"\n").expect("IPv6 loopback");
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
