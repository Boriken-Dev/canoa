//! The portal's settings are `[portal]` of canoa.cfg: read from the system's file (or the files `--config` names) and never from the
//! account's own, and `canoa portal config` reads and writes them.
#![cfg(all(unix, feature = "portal"))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("canoa-portal-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `canoa portal ARGS` with `--config` the files named, and the account's own configuration folder in `home`.
fn run(config: &str, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["portal", "--config", config])
        .args(args)
        .env("XDG_CONFIG_HOME", home)
        .env("HOME", home)
        .env_remove("CANOA_CONFIG")
        .output()
        .expect("canoa runs")
}

fn json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("a JSON object")
}

#[test]
fn the_files_named_are_merged_and_the_higher_one_wins_key_by_key() {
    let dir = Folder::new("merge");
    let (low, high) = (dir.0.join("low.cfg"), dir.0.join("high.cfg"));
    std::fs::write(
        &low,
        "[portal]\nidle_seconds = 5\nhosts = [\"a.example\"]\n[host.web]\nlisten = \"127.0.0.1:1\"\n",
    )
    .unwrap();
    std::fs::write(&high, "[portal]\nidle_seconds = 9\n").unwrap();
    let both = format!("{}:{}", low.display(), high.display());
    let said = json(&run(&both, &dir.0, &["config", "get"]));
    assert_eq!(
        said,
        serde_json::json!({"idle_seconds": 9, "hosts": ["a.example"]})
    );
    let one = json(&run(&both, &dir.0, &["config", "get", "idle_seconds"]));
    assert_eq!(one, 9);
}

#[test]
fn set_checks_the_whole_table_first_and_writes_the_last_file_only() {
    let dir = Folder::new("set");
    let (low, high) = (dir.0.join("low.cfg"), dir.0.join("high.cfg"));
    std::fs::write(&low, "[portal]\nidle_seconds = 5\n").unwrap();
    std::fs::write(
        &high,
        "# the administrator's\n[desktop]\nstart_hidden = true\n",
    )
    .unwrap();
    let both = format!("{}:{}", low.display(), high.display());
    let done = run(&both, &dir.0, &["config", "set", "max_hosts", "7"]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let written = std::fs::read_to_string(&high).unwrap();
    assert!(written.starts_with("# the administrator's\n"), "{written}");
    assert!(written.contains("max_hosts = 7"), "{written}");
    assert_eq!(
        std::fs::read_to_string(&low).unwrap(),
        "[portal]\nidle_seconds = 5\n"
    );

    // A value the portal would refuse is not written.
    let refused = run(&both, &dir.0, &["config", "set", "max_hosts", "0"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("max_hosts"));
    // A secret written out in a provider's config is refused, and the value is not printed.
    let secret = run(
        &both,
        &dir.0,
        &[
            "config",
            "set",
            "authenticators",
            "[{\"provider\":\"p\",\"config\":{\"bind_password\":\"hunter2\"}}]",
        ],
    );
    assert!(!secret.status.success());
    assert!(!String::from_utf8_lossy(&secret.stderr).contains("hunter2"));
    assert!(!std::fs::read_to_string(&high).unwrap().contains("hunter2"));

    let gone = run(&both, &dir.0, &["config", "unset", "max_hosts"]);
    assert!(gone.status.success());
    assert!(
        !std::fs::read_to_string(&high)
            .unwrap()
            .contains("max_hosts")
    );
}

#[test]
fn a_portal_table_in_the_accounts_own_file_is_ignored_with_a_word() {
    let dir = Folder::new("own");
    let own = dir.0.join("Canoa");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("canoa.cfg"), "[portal]\nmin_uid = 1\n").unwrap();
    // No --config: the installed and the system's files, never the account's own.
    let said = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["portal", "config", "get"])
        .env("XDG_CONFIG_HOME", &dir.0)
        .env("HOME", &dir.0)
        .env_remove("CANOA_CONFIG")
        .output()
        .expect("canoa runs");
    let table = json(&said);
    assert!(table.get("min_uid").is_none(), "{table}");
    assert!(
        String::from_utf8_lossy(&said.stderr).contains("ignoring [portal]"),
        "{}",
        String::from_utf8_lossy(&said.stderr)
    );
}

#[test]
fn get_refuses_a_table_the_portal_would_not_start_with() {
    let dir = Folder::new("get-refused");
    let file = dir.0.join("canoa.cfg");
    std::fs::write(
        &file,
        "[portal]
idle_seconds = 5
[portal.instances]
x = 1
",
    )
    .unwrap();
    let said = run(file.to_str().unwrap(), &dir.0, &["config", "get"]);
    assert!(!said.status.success());
    let stderr = String::from_utf8_lossy(&said.stderr);
    assert!(stderr.contains("instances"), "{stderr}");
    assert!(stderr.contains("would refuse to start"), "{stderr}");
    // The key can still be removed.
    let gone = run(
        file.to_str().unwrap(),
        &dir.0,
        &["config", "unset", "instances"],
    );
    assert!(gone.status.success());
    assert_eq!(
        json(&run(file.to_str().unwrap(), &dir.0, &["config", "get"])),
        serde_json::json!({"idle_seconds": 5})
    );
}
