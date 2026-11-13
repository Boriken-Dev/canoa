//! What the host learns of its children when the application around it collects every child of the process that ends, as Dart's
//! process handling does on Linux and macOS while a `Process` of the application's own runs. A thread here does the same, so
//! this is the application's behaviour without its runtime: a child of this process is collected before the host can ask how it
//! ended, and a child of the host's own process starter is not.
//!
//! One test, since the collecting thread takes every child of this process, another test's included.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{BufRead as _, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use canoa_host::spawn::{Command, Stdio};
use serde_json::json;

fn child_entry() -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile.join("examples").join("child_entry")
}

/// The collection an application's exit handler does: any child of this process that has ended, wherever it was started.
fn collect_every_child(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let mut status = 0;
        // SAFETY: `waitpid` writes a status into a local and takes no pointer it keeps.
        unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
        thread::sleep(Duration::from_millis(1));
    }
}

fn alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).expect("a process id");
    // SAFETY: signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn gone_soon(pid: u32) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    !alive(pid)
}

#[test]
fn the_host_reads_how_its_children_ended_while_the_application_collects_every_child_of_its_own() {
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| collect_every_child(&stop));
        let outcome = std::panic::catch_unwind(|| {
            // The application starts the host: from now on its children are the process starter's.
            let request = json!({
                "args": ["--version"],
                "program": child_entry(),
                "program_args": [canoa_ffi::jobs::CHILD_FLAG],
            })
            .to_string();
            let mut job = 0_u64;
            // SAFETY: both point at what lives across the call.
            let status = unsafe {
                canoa_ffi::exports::canoa_ffi_job_start(
                    request.as_ptr(),
                    request.len(),
                    &raw mut job,
                )
            };
            assert_eq!(status, canoa_ffi::exports::STATUS_OK);

            // Control: a child of this process, as std starts it, is collected before it is asked about.
            let mut own = StdCommand::new("sh")
                .args(["-c", "sleep 0.2; exit 5"])
                .spawn()
                .expect("it starts");
            // Asked a little after it ended, as the host does while it waits on one.
            let until = Instant::now() + Duration::from_secs(10);
            let lost = loop {
                match own.try_wait() {
                    Err(lost) => break lost,
                    Ok(Some(status)) => {
                        panic!("not collected by the application's handler: {status}")
                    }
                    Ok(None) => assert!(Instant::now() < until, "it ended"),
                }
                thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(lost.raw_os_error(), Some(libc::ECHILD), "{lost}");

            // Through the host: how it ended is read, and the output is read to its end.
            let mut child = Command::new("sh")
                .args(["-c", "echo said; sleep 0.2; exit 5"])
                .stdout(Stdio::Piped)
                .spawn()
                .expect("it starts");
            let mut said = String::new();
            child
                .stdout
                .take()
                .expect("piped")
                .read_to_string(&mut said)
                .expect("read");
            assert_eq!(said, "said\n");
            let status = child.wait().expect("its status is the host's to read");
            assert_eq!(status.code(), Some(5));
            let output = Command::new("sh")
                .args(["-c", "echo out; echo err >&2; exit 0"])
                .stdin(Stdio::Null)
                .output()
                .expect("it runs");
            assert!(output.status.success());
            assert_eq!(
                (output.stdout.as_slice(), output.stderr.as_slice()),
                (&b"out\n"[..], &b"err\n"[..])
            );

            // An agent's stop reaches its own group while it lives, and no process of the application's own.
            let mut bystander = StdCommand::new("sleep")
                .arg("300")
                .spawn()
                .expect("it starts");
            let mut agent = Command::new("sh")
                .args(["-c", "sleep 300 & echo $!; wait"])
                .stdout(Stdio::Piped)
                .own_group()
                .spawn()
                .expect("it starts");
            let mut line = String::new();
            BufReader::new(agent.stdout.as_mut().expect("piped"))
                .read_line(&mut line)
                .expect("a line");
            let grandchild: u32 = line.trim().parse().expect("a process id");
            assert!(agent.peek().expect("seen").is_none(), "the agent runs");
            agent.end_group();
            let ended = agent.wait().expect("its status is the host's to read");
            assert_ne!(ended.code(), Some(0), "{ended:?}");
            assert!(
                gone_soon(grandchild),
                "what the agent started ended with its group"
            );
            assert!(
                alive(bystander.id()),
                "the application's own process is none of the host's to end"
            );
            // The application's own child, ended here, is collected by the application's own handler.
            let _ = bystander.kill();
        });
        stop.store(true, Ordering::SeqCst);
        if let Err(failed) = outcome {
            std::panic::resume_unwind(failed);
        }
    });
}
