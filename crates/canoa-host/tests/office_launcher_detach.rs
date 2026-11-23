//! `canoa office-launcher --detach`, which the Windows sign-in entry runs: it starts the launcher again with no console and ends at
//! once, and the launcher it left keeps holding the add-in's address and writing to its log.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::fs::File;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-detach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn free_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("a port")
        .local_addr()
        .expect("its address")
}

/// Ends the process `id`, which this test started.
fn stop(id: u32) {
    let _ = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/PID", &id.to_string(), "/F"])
            .output()
    } else {
        Command::new("kill").arg(id.to_string()).output()
    };
}

#[test]
fn detach_ends_at_once_and_the_launcher_it_started_holds_the_address_and_logs() {
    let dir = scratch("holds");
    let address = free_address();
    let config = dir.join("canoa.cfg");
    std::fs::write(
        &config,
        format!("[host.web]\nlisten = \"{address}\"\noffice = true\noffice_local = true\n"),
    )
    .expect("a config");
    let home = dir.join("home");

    // Files, not pipes: a process started on Windows holds every inheritable handle of its starter, so the launcher would keep a pipe
    // open and a reader of it would wait for the launcher to end.
    let said_to = dir.join("said.json");
    let complained_to = dir.join("complained.txt");
    let mut starter = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args([
            "office-launcher",
            "--detach",
            "--format",
            "json",
            "--config",
        ])
        .arg(&config)
        .env("COQUI_AGENTIC_PEER_HOME", &home)
        .env_remove("CANOA_CONFIG")
        .stdin(Stdio::null())
        .stdout(File::create(&said_to).expect("a file"))
        .stderr(File::create(&complained_to).expect("a file"))
        .spawn()
        .expect("canoa starts");
    // The launcher runs until it is stopped: a starter that waited for it would never end.
    let until = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = starter.try_wait().expect("a status") {
            break status;
        }
        if Instant::now() >= until {
            let _ = starter.kill();
            panic!("--detach did not end");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{}",
        std::fs::read_to_string(&complained_to).unwrap_or_default()
    );
    let said: Value = serde_json::from_slice(&std::fs::read(&said_to).expect("its output"))
        .expect("one JSON object");
    let id = u32::try_from(said["pid"].as_u64().expect("a process id")).expect("fits");
    let log = PathBuf::from(said["log"].as_str().expect("a log"));
    assert!(log.starts_with(&home), "{log:?}");

    // It keeps running after its starter ended: it takes the address once no host has held the home for a while. A loaded
    // machine (a whole workspace's tests at once) takes far longer than an idle one, hence the long wait.
    let until = Instant::now() + Duration::from_secs(90);
    let mut holds = false;
    while Instant::now() < until {
        if TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok() {
            holds = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The address can answer before the line saying so is written: wait for the line too.
    let mut logged = std::fs::read_to_string(&log).unwrap_or_default();
    while holds && !logged.contains("no Canoa runs: offering to start it") && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(50));
        logged = std::fs::read_to_string(&log).unwrap_or_default();
    }
    stop(id);
    assert!(holds, "the launcher holds {address}: {logged}");
    assert!(
        logged.contains("no Canoa runs: offering to start it"),
        "its lines reach the log: {logged}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn detach_refuses_a_config_with_no_address_to_hold_and_starts_nothing() {
    let dir = scratch("refuses");
    let config = dir.join("canoa.cfg");
    std::fs::write(&config, "[host]\n").expect("a config");
    let home = dir.join("home");
    let output = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["office-launcher", "--detach", "--config"])
        .arg(&config)
        .env("COQUI_AGENTIC_PEER_HOME", &home)
        .stdin(Stdio::null())
        .output()
        .expect("canoa runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        !home.join("office").join("launcher.log").exists(),
        "no launcher was started"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
