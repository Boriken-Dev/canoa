//! Where the host finds Antigravity's program when its folder is not on the `PATH`: the installer's own folder
//! (`%LOCALAPPDATA%/agy/bin`) is where a session looks, so the models Antigravity lists with nothing started are asked of the
//! program there, and a host with no such program says it is not installed. The program is a fixture in a folder of the test's own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::Path;

use serde_json::Value;
use support::{Scratch, host_command};

/// A stand-in for `agy` in `folder`: `agy models` lists one model.
fn install(folder: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    if cfg!(windows) {
        std::fs::write(
            folder.join("agy.cmd"),
            "@echo off\r\necho Fetching available models...\r\necho stand-in-model\tStand-in Model\r\n",
        )
        .expect("a program");
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = folder.join("agy");
            std::fs::write(
                &path,
                "#!/bin/sh\necho 'Fetching available models...'\nprintf 'stand-in-model\tStand-in Model\n'\n",
            )
            .expect("a program");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("runnable");
        }
    }
}

/// What `canoa options Antigravity` says with `LOCALAPPDATA` at `local` and a `PATH` that holds no `agy`.
fn options(scratch: &Scratch, local: &Path) -> Value {
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(
        &config,
        "[host.agents.Antigravity]\nharness = \"antigravity\"\n",
    )
    .expect("a config");
    let path = if cfg!(windows) {
        std::env::var("SystemRoot").map_or_else(
            |_| "C:/Windows/System32".to_owned(),
            |root| format!("{root}/System32"),
        )
    } else {
        "/bin:/usr/bin".to_owned()
    };
    let output = host_command()
        .args(["options", "Antigravity", "--entry", "--config"])
        .arg(&config)
        .args(["--format", "json"])
        .env("PATH", path)
        .env("LOCALAPPDATA", local)
        .env("HOME", scratch.0.join("user"))
        .env("USERPROFILE", scratch.0.join("user"))
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("canoa runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON object")
}

#[test]
fn a_program_in_the_installers_folder_is_found_though_the_path_lacks_it() {
    let scratch = Scratch::new();
    let local = scratch.0.join("local");
    install(&local.join("agy").join("bin"));
    let answer = options(&scratch, &local);
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(answer["models"][0]["id"], "stand-in-model", "{answer}");
}

#[test]
fn with_no_program_anywhere_antigravity_is_not_installed() {
    let scratch = Scratch::new();
    let local = scratch.0.join("local");
    std::fs::create_dir_all(&local).expect("a folder");
    let answer = options(&scratch, &local);
    assert_eq!(answer["known"], false, "{answer}");
    assert_eq!(answer["unknown_kind"], "not_installed", "{answer}");
}
