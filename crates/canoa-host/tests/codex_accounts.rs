//! Codex's automatic `local` account over the control channel: its plan limits are read from the login Codex stores in its home,
//! with no `codex` process started. The login is a fixture in a folder of the test's own that `CODEX_HOME` names, ChatGPT's
//! backend is a loopback stand-in, and the program the entry names only leaves a mark if it is ever run.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use linked::Hosted;
use serde_json::{Value, json};
use support::quoted;

/// A login as Codex stores it, whose access token is a JWT that expires in 2100; `account_id` is what the backend is told.
const LOGIN: &str = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"i","access_token":"eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.","refresh_token":"fixture-refresh-token","account_id":"fixture-account"}}"#;

const DOCUMENT: &str = r#"{"plan_type":"plus","rate_limit":{"allowed":true,"limit_reached":false,"primary_window":{"used_percent":17,"limit_window_seconds":18000,"reset_after_seconds":3600,"reset_at":4102450000},"secondary_window":{"used_percent":42,"limit_window_seconds":604800,"reset_after_seconds":90000,"reset_at":4102500000}},"additional_rate_limits":[],"credits":{"has_credits":false,"unlimited":false,"balance":null}}"#;

/// The test's own Codex home, named by `CODEX_HOME` of the process before any host starts, and the login put in it.
fn codex_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("canoa-codex-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("a home");
    std::fs::write(home.join("auth.json"), LOGIN).expect("the stored login");
    // SAFETY: this file has one test, which sets it before its host starts.
    unsafe { std::env::set_var("CODEX_HOME", &home) };
    for name in ["CLAUDE_CONFIG_DIR", "PI_CODING_AGENT_DIR", "GROK_HOME"] {
        // SAFETY: as above.
        unsafe { std::env::set_var(name, home.join(name)) };
    }
    home
}

/// A stand-in for `codex` that only leaves a mark: reading the account must never start it.
fn marking(dir: &Path) -> PathBuf {
    let mark = dir.join("codex-ran");
    let (name, body) = if cfg!(windows) {
        (
            "codex.cmd",
            format!(
                "@echo off\r\necho ran> \"{}\"\r\nexit /b 1\r\n",
                mark.display()
            ),
        )
    } else {
        (
            "codex",
            format!("#!/bin/sh\necho ran > \"{}\"\nexit 1\n", mark.display()),
        )
    };
    let path = dir.join(name);
    std::fs::write(&path, body).expect("script written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("script executable");
    }
    path
}

/// The stand-in for ChatGPT's backend: it keeps each request's head and answers the usage document.
struct Backend {
    address: SocketAddr,
    heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("its address");
        let stop = Arc::new(AtomicBool::new(false));
        let heads = Arc::new(Mutex::new(Vec::new()));
        let (stopping, seen) = (Arc::clone(&stop), Arc::clone(&heads));
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(byte[0]);
                }
                seen.lock()
                    .expect("not poisoned")
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{DOCUMENT}",
                    DOCUMENT.len()
                );
                let _ = stream.flush();
            }
        });
        Self {
            address,
            heads,
            stop,
            thread: Some(thread),
        }
    }

    fn heads(&self) -> Vec<String> {
        self.heads.lock().expect("not poisoned").clone()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn a_stored_codex_login_is_read_for_its_plan_limits_and_nothing_is_started() {
    let home = codex_home();
    let backend = Backend::start();
    let address = backend.address;
    let host = Hosted::start_with_setup(
        |dir| {
            let program = quoted(&marking(dir).to_string_lossy());
            format!(
                "[secrets]\naccounts = \"file\"\n\n[host.agents.Codex]\nharness = \"codex\"\nexecutable = {program}\n\
                 account_settings = {{ executable = {program}, usage_endpoint = \"http://{address}/backend-api\" }}\n"
            )
        },
        &[],
    );
    let ran = host.scratch.0.join("codex-ran");
    let link = host.local.open("@control").expect("a link");
    let (read, _) = linked::ask(
        &link,
        1,
        "usage/read",
        json!({"account": "local", "harness": "codex"}),
    )
    .expect("usage/read");
    let row = &read["account"];
    assert_eq!(row["supported"], true, "{read}");
    let ids: Vec<&str> = row["buckets"]
        .as_array()
        .expect("buckets")
        .iter()
        .filter_map(|bucket| bucket["id"].as_str())
        .collect();
    assert_eq!(ids, ["codex:five_hour", "codex:weekly"], "{read}");
    let heads = backend.heads();
    assert_eq!(heads.len(), 1, "{heads:?}");
    let head = heads[0].to_ascii_lowercase();
    assert!(head.starts_with("get /backend-api/wham/usage "), "{head}");
    assert!(
        head.contains("chatgpt-account-id: fixture-account"),
        "{head}"
    );
    assert!(!ran.exists(), "codex was started to read the account");
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).expect("still there"),
        LOGIN,
        "the login is read and never written"
    );
    let text: Value = read;
    assert!(!text.to_string().contains("fixture-refresh-token"));
    host.stop();
    let _ = std::fs::remove_dir_all(&home);
}
