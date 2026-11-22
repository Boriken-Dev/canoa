#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::PathBuf;

use clap::CommandFactory as _;
use clap::Parser as _;

use super::args::{Cli, Command};
use super::commands::adopt_legacy_name;
use crate::Config;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-config-name-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    dir
}

#[cfg(unix)]
#[test]
fn a_legacy_name_is_renamed_once_when_nothing_has_the_new_one() {
    let dir = scratch("rename");
    std::fs::write(dir.join("Canoa.cfg"), "[host]\n").expect("written");
    assert!(adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()));
    assert_eq!(
        std::fs::read_to_string(dir.join("canoa.cfg")).expect("read"),
        "[host]\n"
    );
    assert!(!dir.join("Canoa.cfg").exists());
    assert!(
        !adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()),
        "nothing is left to rename"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_legacy_name_never_replaces_the_new_one() {
    let dir = scratch("keep");
    std::fs::write(dir.join("Canoa.cfg"), "old").expect("written");
    std::fs::write(dir.join("canoa.cfg"), "new").expect("written");
    assert!(!adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()));
    assert_eq!(
        std::fs::read_to_string(dir.join("canoa.cfg")).expect("read"),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("canoa.cfg")).expect("read"),
        "old"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn a_canoa_cfg_that_exists_is_read_by_the_new_name_on_windows() {
    let dir = scratch("windows");
    std::fs::write(dir.join("Canoa.cfg"), "[host]\n").expect("written");
    assert!(
        !adopt_legacy_name(&dir.join("canoa.cfg"), &|_| ()),
        "nothing to rename: it is the same file"
    );
    Config::load(&dir.join("canoa.cfg")).expect("the file with the old spelling loads");
    let names: Vec<_> = std::fs::read_dir(&dir)
        .expect("listed")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["Canoa.cfg"], "it keeps the name it was given");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_command_line_is_well_formed() {
    Cli::command().debug_assert();
}

#[test]
fn serve_takes_a_config_flag() {
    let cli = Cli::try_parse_from(["canoa", "serve", "--config", "x.cfg"]).expect("parses");
    let Command::Serve(args) = cli.command else {
        panic!("serve");
    };
    assert_eq!(args.config.config.as_deref(), Some("x.cfg"));
    assert!(!args.until_stdin_closes);
}

#[test]
fn serve_takes_the_web_app_with_or_without_a_directory() {
    let parsed = |args: &[&str]| {
        let cli = Cli::try_parse_from(["canoa", "serve"].iter().chain(args)).expect("parses");
        let Command::Serve(args) = cli.command else {
            panic!("serve");
        };
        (args.web, args.web_listen)
    };
    assert_eq!(parsed(&[]), (None, None));
    assert_eq!(parsed(&["--web"]), (Some(None), None));
    assert_eq!(parsed(&["--web", "--config", "x.cfg"]), (Some(None), None));
    assert_eq!(
        parsed(&["--web", "site", "--web-listen", "127.0.0.1:0"]),
        (
            Some(Some(PathBuf::from("site"))),
            Some("127.0.0.1:0".parse().expect("an address"))
        )
    );
    assert!(Cli::try_parse_from(["canoa", "serve", "--web-listen", "nowhere"]).is_err());
}

#[test]
fn a_provider_needs_its_library_and_id() {
    assert!(Cli::try_parse_from(["canoa", "provider", "--library", "x"]).is_err());
    let cli = Cli::try_parse_from([
        "canoa",
        "provider",
        "--library",
        "x",
        "--provider",
        "echo",
        "--working-root",
        "/a",
        "--working-root",
        "/b",
    ])
    .expect("parses");
    let Command::Provider {
        working_roots,
        max_sessions,
        ..
    } = cli.command
    else {
        panic!("provider");
    };
    assert_eq!(working_roots, [PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(max_sessions, crate::config::DEFAULT_MAX_SESSIONS);
}

#[test]
fn only_what_a_host_runs_for_itself_counts_as_its_own() {
    let own = |args: &[&str]| {
        Cli::try_parse_from(["canoa"].iter().chain(args))
            .expect("parses")
            .is_a_hosts_own()
    };
    assert!(own(&["launch", "x"]));
    assert!(own(&["mcp-bridge"]));
    assert!(own(&["helper", "a-library", "--model", "m"]));
    assert!(own(&["hook-forward", "--channel", "x", "start"]));
    #[cfg(unix)]
    assert!(own(&[crate::spawn::SPAWNER_COMMAND]));
    assert!(own(&["tool", "--name", "t", "--spec", "{}"]));
    assert!(own(&["provider", "--library", "x", "--provider", "p"]));
    assert!(!own(&["serve"]));
    assert!(!own(&["peers"]));
}

#[test]
fn serve_takes_allow_empty() {
    let cli = Cli::try_parse_from(["canoa", "serve", "--allow-empty"]).expect("parses");
    let Command::Serve(args) = cli.command else {
        panic!("serve");
    };
    assert!(args.allow_empty);
}

#[test]
fn serve_knows_which_library_flags_the_command_line_gave() {
    let given = |line: &[&str]| {
        let cli = Cli::try_parse_from(line).expect("parses");
        let Command::Serve(args) = cli.command else {
            panic!("serve");
        };
        args.given.0.clone()
    };
    assert!(given(&["canoa", "serve"]).is_empty());
    assert!(given(&["canoa", "serve", "--method", "noise", "--key-limit", "5"]).is_empty());
    assert_eq!(
        given(&["canoa", "serve", "--max-peers", "4"]),
        ["--max-peers"]
    );
    assert_eq!(
        given(&[
            "canoa",
            "serve",
            "--no-peer-proof",
            "--listen",
            "127.0.0.1:1"
        ]),
        ["--listen", "--no-peer-proof"]
    );
}

#[test]
fn accept_needs_a_peer() {
    assert!(Cli::try_parse_from(["canoa", "accept"]).is_err());
}

fn agent_run(args: &[&str]) -> crate::harness::AgentRun {
    let cli = Cli::try_parse_from(["canoa", "agent"].iter().chain(args)).expect("parses");
    assert!(cli.is_a_hosts_own(), "it takes the standard streams");
    let Command::Agent(agent) = cli.command else {
        panic!("agent");
    };
    agent.run()
}

#[test]
fn agent_serves_the_id_it_is_given_with_the_settings_it_is_given_and_what_a_host_narrows() {
    let run = agent_run(&[
        "claude",
        "--executable",
        "/opt/claude",
        "--model",
        "claude-haiku-5-5",
        "--effort",
        "low",
        "--setting",
        "setting_sources=[]",
        "--setting",
        "allow_client_plugins=true",
        "--setting",
        "permission_mode=plan",
        "--working-root",
        "/w",
        "--no-client-mcp-servers",
        "--max-acp-version",
        "1",
    ]);
    assert_eq!(run.id, "claude");
    assert_eq!(
        run.executable.as_deref(),
        Some(std::ffi::OsStr::new("/opt/claude"))
    );
    let settings = &run.settings;
    assert_eq!(settings["model"], "claude-haiku-5-5");
    assert_eq!(settings["effort"], "low");
    assert_eq!(settings["setting_sources"], serde_json::json!([]));
    assert_eq!(settings["allow_client_plugins"], true);
    assert_eq!(settings["permission_mode"], "plan", "text stays text");
    assert_eq!(settings["client_mcp_servers"], false);
    assert_eq!(settings["max_acp_version"], 1);
    assert_eq!(run.roots, [PathBuf::from("/w")]);

    let bare = agent_run(&["claude"]);
    assert!(
        bare.settings.is_empty(),
        "a setting nobody gave is the provider's own: {:?}",
        bare.settings
    );
    assert!(!bare.access_asked && bare.roots.is_empty());
}

#[test]
fn agent_takes_a_cache_lifetime_for_any_agent() {
    use coqui_acp_api::CacheLifetime;
    for lifetime in crate::config::CACHE_LIFETIMES {
        assert_eq!(
            agent_run(&["claude", "--cache-lifetime", lifetime]).lifetime,
            crate::config::cache_lifetime_named(lifetime).expect("a lifetime")
        );
    }
    assert_eq!(agent_run(&["codex"]).lifetime, CacheLifetime::Default);
    assert!(
        Cli::try_parse_from(["canoa", "agent", "claude", "--cache-lifetime", "forever"]).is_err()
    );
}

/// Whether the provider of `run` takes the settings `canoa agent` would give it.
fn provider_takes(run: &crate::harness::AgentRun) -> Result<(), String> {
    let registry = std::sync::Arc::new(crate::kind::registry(None).map_err(|e| e.to_string())?);
    let settings = run.provider_settings(&registry)?;
    crate::kind::Harness::new(
        &registry,
        &run.id,
        (&settings, &crate::config::Settings::default()),
        false,
        Vec::new(),
        std::sync::Arc::default(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[test]
fn the_command_lines_that_start_an_agent_are_accepted_by_every_bundled_agents_provider() {
    // What the host's tests and documents start `canoa agent` with, and what every agent takes alike (a flag that sets a provider setting is
    // left out of the loop over every agent: a provider need not declare it, and says so).
    let documented: [&[&str]; 3] = [
        &[
            "claude",
            "--executable",
            "none",
            "--setting",
            "setting_sources=[\"user\",\"project\"]",
            "--setting",
            "allow_client_plugins=true",
        ],
        &["claude", "--executable", "none"],
        &["codex", "--executable", "none"],
    ];
    for args in documented {
        let run = agent_run(args);
        provider_takes(&run).unwrap_or_else(|why| panic!("{args:?}: {why}"));
    }
    for id in crate::bundled::harnesses() {
        let run = agent_run(&[
            id,
            "--executable",
            "none",
            "--working-root",
            "/w",
            "--cache-lifetime",
            "short",
        ]);
        provider_takes(&run).unwrap_or_else(|why| panic!("{id}: {why}"));
    }
}

#[test]
fn a_canoa_cfg_from_two_layers_dispatches_cleanly() {
    let dir = scratch("two-layers");
    std::fs::write(dir.join("Canoa.cfg"), "[host]\n").expect("written");
    Config::load(&dir.join("Canoa.cfg")).expect("loads");
    let _ = std::fs::remove_dir_all(&dir);
}
