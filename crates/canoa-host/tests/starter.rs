//! Every process the host starts for itself, once the host's own process starter is installed, is a child of that starter and never
//! of the application, and the application collecting every child of its own process leaves each status readable and each
//! group kill live. A thread here collects every child of this process that ends, as Dart's process handling does while a
//! Dart `Process` runs. One test, since that thread takes the children of any other.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::ffi::OsString;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use canoa_host::cli::Program;
use canoa_host::spawn::{self, Command};
use coqui_acp_bridge::agent::Agent;
use coqui_acp_harness::{Launch, PlainSpawn, Spawn as _};

fn collect_every_child(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let _ = rustix::process::wait(rustix::process::WaitOptions::NOHANG);
        thread::sleep(Duration::from_millis(1));
    }
}

fn alive(pid: u32) -> bool {
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid).expect("an id")).expect("not 0");
    rustix::process::test_kill_process(pid).is_ok()
}

/// `(parent id, its command line)` of process `pid`, as the system's own `ps` says.
fn parent_of(pid: u32) -> (u32, String) {
    let ps = |field: &str, of: u32| {
        // Through the host's own way: this test's collector takes a child of the process that is its own.
        let listed = Command::new("ps")
            .args(["-o", field, "-p", &of.to_string()])
            .output()
            .expect("ps runs");
        String::from_utf8_lossy(&listed.stdout).trim().to_owned()
    };
    let parent: u32 = ps("ppid=", pid).parse().expect("a parent id");
    (parent, ps("args=", parent))
}

#[test]
fn every_process_the_host_starts_is_the_starters_child_and_reads_true_while_the_application_collects_its_own()
 {
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| collect_every_child(&stop));
        let outcome = std::panic::catch_unwind(run);
        stop.store(true, Ordering::SeqCst);
        if let Err(failed) = outcome {
            std::panic::resume_unwind(failed);
        }
    });
}

fn run() {
    let me = std::process::id();
    spawn::install(Program::new(
        PathBuf::from(env!("CARGO_BIN_EXE_canoa")),
        Vec::<OsString>::new(),
    ));

    // The way the host starts everything.
    let mut plain = Command::new("sleep").arg("300").spawn().expect("it starts");
    let starter = parent_of(plain.id());
    assert_ne!(
        starter.0, me,
        "a process of the host's is the application's child"
    );
    assert!(
        starter.1.contains("spawner"),
        "its parent is not the starter: {}",
        starter.1
    );
    assert_eq!(
        parent_of(starter.0).0,
        me,
        "the starter is the application's own child"
    );

    // An agent, as a session starts one.
    let agent = Agent::spawn_with(&spawn::Agents, "sleep".as_ref(), &["300".into()], 1024)
        .expect("it starts");
    assert_eq!(
        parent_of(agent.pid()).0,
        starter.0,
        "an agent is not the starter's child"
    );

    // What `coqui-acp-harness` starts for itself: an account's sign-in, an installer, a harness's models.
    let sleeper = PlainSpawn
        .spawn(&Launch::new("/bin/sh").args(["-c", "echo $$; exec sleep 300"]))
        .expect("it starts");
    let mut line = String::new();
    let mut said = sleeper.stdout;
    let mut byte = [0_u8; 1];
    while said.read(&mut byte).expect("read") == 1 && byte[0] != b'\n' {
        line.push(char::from(byte[0]));
    }
    let held: u32 = line.trim().parse().expect("a process id");
    assert_eq!(
        parent_of(held).0,
        starter.0,
        "a process of the harness crate's is not the starter's child"
    );

    // Its status is read though the application collects every child of its own, and killing it ends its group.
    let done = PlainSpawn
        .spawn(&Launch::new("/bin/sh").args(["-c", "sleep 0.2; exit 5"]))
        .expect("it starts");
    let until = Instant::now() + Duration::from_secs(20);
    let exit = loop {
        match done.process.try_wait().expect("its status is readable") {
            Some(exit) => break exit,
            None => assert!(Instant::now() < until, "it ended"),
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(exit.code, Some(5), "{exit}");
    sleeper.process.kill().expect("ended");
    let until = Instant::now() + Duration::from_secs(10);
    while alive(held) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !alive(held),
        "the process the harness crate started outlived its kill"
    );

    // Control: a process this test starts itself is its own child, and is the kind the collector takes.
    #[allow(
        clippy::zombie_processes,
        reason = "the collecting thread of this test is what collects it"
    )]
    let mut own = StdCommand::new("sleep")
        .arg("300")
        .spawn()
        .expect("it starts");
    assert_eq!(parent_of(own.id()).0, me);
    let _ = own.kill();

    plain.kill().expect("ended");
    plain.wait().expect("its status is readable");
    agent.stop().expect("ended");
}
