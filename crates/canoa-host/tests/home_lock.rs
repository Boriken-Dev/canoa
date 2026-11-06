//! One host on one home: a second says who holds it and does not start, a dead holder's lock is free, and a host stopped
//! through the library gives back its addresses, its agents and its home.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use canoa_host::{Config, Home, ServeOptions, Serving};
use serde_json::{Value, json};
use support::{Host, Scratch, WAIT, example, host_binary, quoted, running, with_link};

fn agent_entry() -> String {
    format!(
        "[host.agents.fixture]\nprogram = {}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

/// `canoa serve` on `host`'s configuration and home, to its end.
fn second_serve(host: &Host) -> Output {
    let mut command = Command::new(host_binary());
    command
        .args(["serve", "--config"])
        .arg(host.scratch.0.join("canoa.cfg"))
        .args(["--method", support::method().name()])
        .env("COQUI_AGENTIC_PEER_HOME", host.scratch.0.join("home"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("it starts");
    let until = Instant::now() + WAIT;
    while child.try_wait().expect("a status").is_none() {
        assert!(Instant::now() < until, "a second host did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().expect("its output")
}

#[test]
fn a_second_host_on_a_home_says_which_is_running_and_does_not_start() {
    let mut first = Host::start(&agent_entry(), 1);
    let second = second_serve(&first);
    assert_eq!(second.status.code(), Some(3), "{second:?}");
    assert!(second.stdout.is_empty(), "it serves nothing: {second:?}");
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(said.starts_with("canoa: canoa (process "), "{said}");
    assert!(said.contains("already hosts this home"), "{said}");
    assert!(
        first.ended_within(Duration::from_millis(200)).is_none(),
        "the first goes on"
    );
    first.kill();
}

#[test]
fn a_dead_holders_lock_is_taken_over() {
    let mut first = Host::start(&agent_entry(), 1);
    first.kill();
    let path = first.scratch.0.join("canoa.cfg");
    let mut next = Command::new(host_binary())
        .args(["serve", "--config"])
        .arg(path)
        .args(["--method", support::method().name()])
        .env("COQUI_AGENTIC_PEER_HOME", first.scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("it starts");
    let out = next.stdout.take().expect("its output");
    let mut line = String::new();
    BufReader::new(out).read_line(&mut line).expect("a line");
    let value: Value = serde_json::from_str(&line).expect("an entry, so it started");
    assert_eq!(value["name"], "fixture");
    let holder = Home::at(first.scratch.0.join("home"))
        .holder()
        .expect("it says who holds the home");
    assert_eq!(holder.pid, next.id());
    assert_eq!(holder.name, "canoa");
    let _ = next.kill();
    let _ = next.wait();
}

#[test]
fn with_nothing_to_serve_it_ends_unless_told_to_stay() {
    let scratch = Scratch::new();
    let path = scratch.0.join("canoa.cfg");
    std::fs::write(&path, "").expect("written");
    let refused = Command::new(host_binary())
        .args(["serve", "--config"])
        .arg(&path)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("it runs");
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");

    let mut stays = Command::new(host_binary())
        .args(["serve", "--allow-empty", "--until-stdin-closes", "--config"])
        .arg(&path)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("it starts");
    let out = stays.stdout.take().expect("its output");
    let mut line = String::new();
    BufReader::new(out).read_line(&mut line).expect("a line");
    let value: Value = serde_json::from_str(&line).expect("one JSON object");
    assert_eq!(value["ready"], true, "{line}");
    assert!(value["peer"].is_string(), "{line}");
    drop(stays.stdin.take());
    let until = Instant::now() + WAIT;
    loop {
        if let Some(status) = stays.try_wait().expect("a status") {
            assert!(status.success(), "{status:?}");
            break;
        }
        assert!(Instant::now() < until, "it did not end with its input");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a person gets who has never written a canoa.cfg: a program that hosts by default has to start all the same.
#[test]
fn with_no_canoa_cfg_at_all_it_stays_only_when_told_to() {
    let scratch = Scratch::new();
    let path = scratch.0.join("never-written").join("canoa.cfg");
    let refused = Command::new(host_binary())
        .args(["serve", "--config"])
        .arg(&path)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("it runs");
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");

    let mut stays = Command::new(host_binary())
        .args(["serve", "--allow-empty", "--until-stdin-closes", "--config"])
        .arg(&path)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("it starts");
    let out = stays.stdout.take().expect("its output");
    let mut line = String::new();
    BufReader::new(out).read_line(&mut line).expect("a line");
    drop(stays.stdin.take());
    let ended = stays.wait_with_output().expect("it ends");
    let value: Value = serde_json::from_str(&line).unwrap_or_else(|_| {
        panic!(
            "one JSON object, not {line:?}: {}",
            String::from_utf8_lossy(&ended.stderr)
        )
    });
    assert_eq!(value["ready"], true, "{line}");
    assert!(ended.status.success(), "{ended:?}");
    assert!(!path.exists(), "it writes no canoa.cfg");
}

#[test]
fn allow_empty_does_not_excuse_a_canoa_cfg_that_cannot_be_read() {
    let scratch = Scratch::new();
    let refused = Command::new(host_binary())
        .args(["serve", "--allow-empty", "--config"])
        .arg(&scratch.0)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("it runs");
    assert_eq!(refused.status.code(), Some(2), "a directory: {refused:?}");
}

/// The library run in this process, as an application hosting it does.
#[test]
fn a_host_stopped_through_the_library_gives_back_its_address_and_agents() {
    let scratch = Scratch::new();
    let home = Home::at(scratch.0.join("home"));
    let lock = home.lock("a test").expect("a free home");
    let config = Config::from_str_in(&agent_entry(), &scratch.0).expect("parses");
    let identity = home.identity(Some(support::method())).expect("an identity");
    let serving = Serving::bind(
        &config,
        identity,
        home.store().expect("a store"),
        &ServeOptions::new(host_binary()),
    )
    .expect("it binds");
    let url = serving.listening()[0].url.clone();
    let peer = serving.peer();
    let stopper = serving.stopper();
    let run = std::thread::spawn(move || serving.run());

    let client = support::key();
    home.store()
        .expect("a store")
        .accept_key(&client.key_material())
        .expect("the key is accepted");

    let (pids, pid) = mpsc::channel();
    let link = std::thread::spawn(move || {
        with_link(&url, &client, peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            let said = rpc.request("_fixture/pid", json!({})).expect("it answers");
            pids.send(said["pid"].as_u64().expect("a pid"))
                .expect("the test waits");
            rpc.next_message(Duration::from_secs(30))
        })
    });
    let pid = u32::try_from(pid.recv_timeout(WAIT).expect("the agent's pid")).expect("a pid");
    assert!(running(pid), "the agent runs");

    stopper.stop();
    let until = Instant::now() + WAIT;
    while !run.is_finished() {
        assert!(Instant::now() < until, "run did not return");
        std::thread::sleep(Duration::from_millis(20));
    }
    run.join().expect("run returned");
    let ended = link.join().expect("the link's thread");
    assert!(matches!(ended, Ok(Err(_))), "the link ended: {ended:?}");
    let until = Instant::now() + Duration::from_secs(5);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(pid), "the agent ended with the host");

    // The home is free again, and the same configuration binds again in this process.
    drop(lock);
    let lock = home.lock("a test").expect("a lock given up is free");
    let again = Serving::bind(
        &config,
        home.identity(Some(support::method())).expect("the same"),
        home.store().expect("a store"),
        &ServeOptions::new(host_binary()),
    )
    .expect("it binds again");
    again.stopper().stop();
    again.run();
    drop(lock);
}

#[test]
fn a_fixed_address_is_free_once_stop_returns_from_run() {
    let scratch = Scratch::new();
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = probe.local_addr().expect("its address").port();
    drop(probe);
    let text = format!("{}listen = \"127.0.0.1:{port}\"\n", agent_entry());
    let config = Config::from_str_in(&text, &scratch.0).expect("parses");
    let home = Home::at(scratch.0.join("home"));
    for _ in 0..3 {
        let serving = Serving::bind(
            &config,
            home.identity(Some(support::method())).expect("the same"),
            home.store().expect("a store"),
            &ServeOptions::new(host_binary()),
        )
        .expect("the address is free");
        let stopper = serving.stopper();
        let run = std::thread::spawn(move || serving.run());
        stopper.stop();
        run.join().expect("run returned");
    }
}
