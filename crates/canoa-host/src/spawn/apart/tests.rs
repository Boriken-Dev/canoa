//! The starter and the host's side of its link, over a socket pair in this process: what a child is given, what the host hears of
//! it, and when a signal reaches its group.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{BufRead as _, BufReader};
use std::net::Shutdown;
use std::time::{Duration, Instant};

use super::*;

/// A link to a starter that runs on a thread of this test.
fn started() -> (Arc<Link>, UnixStream) {
    let (host, starter) = UnixStream::pair().expect("a socket pair");
    let end = starter.try_clone().expect("a second handle");
    thread::spawn(move || run(&starter));
    (Link::over(host, None).expect("a link"), end)
}

fn shell(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);
    command
}

fn alive(pid: u32) -> bool {
    let pid =
        rustix::process::Pid::from_raw(i32::try_from(pid).expect("a process id")).expect("not 0");
    rustix::process::test_kill_process(pid).is_ok()
}

fn gone_soon(pid: u32) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    !alive(pid)
}

/// The first line a child's standard output has, as a process id.
fn first_id(child: &mut Child) -> u32 {
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().expect("piped"))
        .read_line(&mut line)
        .expect("a line");
    line.trim().parse().expect("a process id")
}

#[test]
fn a_child_has_the_environment_directory_and_streams_it_was_asked_for() {
    let (link, _end) = started();
    let dir = std::env::temp_dir().canonicalize().expect("a directory");
    let mut command = shell(
        "echo \"$CHILD_VALUE ${HOME-unset} $PWD\"; read word; echo \"got $word\" >&2; exit 3",
    );
    command
        .env("CHILD_VALUE", "set")
        .env_remove("HOME")
        .current_dir(&dir)
        .stdin(Stdio::Piped)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped);
    let mut child = link.spawn(&command).expect("it starts");
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(b"word\n")
        .expect("written");
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("piped")
        .read_to_string(&mut out)
        .expect("it ends the pipe");
    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("piped")
        .read_to_string(&mut err)
        .expect("it ends the pipe");
    assert_eq!(out, format!("set unset {}\n", dir.display()));
    assert_eq!(err, "got word\n");
    let status = child.wait().expect("it ends");
    assert_eq!(status.code(), Some(3));
}

#[test]
fn a_stream_set_to_nothing_is_nothing() {
    let (link, _end) = started();
    let mut command = shell("cat; echo said");
    command
        .stdin(Stdio::Null)
        .stdout(Stdio::Null)
        .stderr(Stdio::Null);
    let mut child = link.spawn(&command).expect("it starts");
    assert!(child.stdin.is_none() && child.stdout.is_none() && child.stderr.is_none());
    assert!(
        child.wait().expect("it ends").success(),
        "cat found its input at its end"
    );
}

#[test]
fn a_program_that_is_not_there_fails_as_the_system_says() {
    let (link, _end) = started();
    let failed = link
        .spawn(&Command::new("/no/such/program"))
        .expect_err("it cannot start");
    assert_eq!(failed.kind(), io::ErrorKind::NotFound, "{failed}");
}

#[test]
fn a_child_is_seen_to_end_and_how_before_it_is_collected() {
    let (link, _end) = started();
    let mut child = link.spawn(&shell("exit 7")).expect("it starts");
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.peek().expect("it can be seen") {
            break status;
        }
        assert!(Instant::now() < until, "it ended");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(7));
    // Looking does not collect it: it can be asked again, and then collected.
    assert_eq!(child.peek().expect("again").and_then(|s| s.code()), Some(7));
    assert_eq!(
        child.try_wait().expect("collected").and_then(|s| s.code()),
        Some(7)
    );
}

#[test]
fn killing_a_child_ends_it_by_the_signal() {
    let (link, _end) = started();
    let mut child = link.spawn(&shell("sleep 300")).expect("it starts");
    assert!(child.try_wait().expect("running").is_none());
    child.kill().expect("asked");
    assert_eq!(child.wait().expect("it ends").signal(), Some(9));
    // Collected, it is no process to signal: not an error, as std's is not.
    child.kill().expect("nothing to do");
}

#[test]
fn ending_a_group_reaches_what_its_leader_started_and_nothing_else() {
    let (link, _end) = started();
    let mut bystander = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("a process of another group");
    let mut command = shell("sleep 300 & echo $!; wait");
    command.stdout(Stdio::Piped).own_group();
    let mut child = link.spawn(&command).expect("it starts");
    let grandchild = first_id(&mut child);
    assert!(alive(grandchild));
    child.end_group();
    assert!(child.wait().expect("it ends").signal().is_some());
    assert!(
        gone_soon(grandchild),
        "what the leader started ended with its group"
    );
    assert!(alive(bystander.id()), "no other process was signalled");
    bystander.kill().expect("ended");
    bystander.wait().expect("collected");
}

#[test]
fn a_group_is_ended_while_its_leader_is_uncollected_and_left_alone_after() {
    let (link, _end) = started();
    // The leader ends at once and leaves a process of its group behind.
    let leave = "sleep 300 & echo $!";
    let mut before = shell(leave);
    before.stdout(Stdio::Piped).own_group();
    let mut before = link.spawn(&before).expect("it starts");
    let first = first_id(&mut before);
    let until = Instant::now() + Duration::from_secs(10);
    while before.peek().expect("seen").is_none() {
        assert!(Instant::now() < until, "the leader ended");
        thread::sleep(Duration::from_millis(10));
    }
    // Ended and not collected: its id is still its own, and so is its group's.
    before.end_group();
    assert!(
        gone_soon(first),
        "the group was ended while its leader was uncollected"
    );
    before.wait().expect("collected");

    let mut after = shell(leave);
    after.stdout(Stdio::Piped).own_group();
    let mut after = link.spawn(&after).expect("it starts");
    let second = first_id(&mut after);
    after.wait().expect("the leader is collected");
    after.end_group();
    thread::sleep(Duration::from_millis(300));
    assert!(
        alive(second),
        "a group whose leader is collected is not signalled: its id may be anyone's"
    );
    let leftover =
        rustix::process::Pid::from_raw(i32::try_from(second).expect("an id")).expect("not 0");
    rustix::process::kill_process(leftover, rustix::process::Signal::KILL)
        .expect("ended by the test");
}

#[test]
fn many_children_started_at_once_each_get_their_own_answer() {
    let (link, _end) = started();
    thread::scope(|scope| {
        let workers: Vec<_> = (0..12_u8)
            .map(|number| {
                let link = &link;
                scope.spawn(move || {
                    let mut command = shell(&format!("echo {number}; exit {number}"));
                    command.stdout(Stdio::Piped);
                    let mut child = link.spawn(&command).expect("it starts");
                    let mut out = String::new();
                    child
                        .stdout
                        .take()
                        .expect("piped")
                        .read_to_string(&mut out)
                        .expect("read");
                    assert_eq!(out.trim(), number.to_string());
                    assert_eq!(child.wait().expect("ends").code(), Some(i32::from(number)));
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("a worker that did not fail");
        }
    });
}

#[test]
fn when_the_starter_ends_what_it_held_is_lost_and_nothing_new_starts() {
    let (link, end) = started();
    let mut child = link.spawn(&shell("sleep 300")).expect("it starts");
    let pid = child.id();
    end.shutdown(Shutdown::Both)
        .expect("the starter's socket ended");
    let until = Instant::now() + Duration::from_secs(10);
    let error = loop {
        match child.try_wait() {
            Err(error) => break error,
            Ok(_) => {
                assert!(Instant::now() < until, "the loss is heard");
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert!(error.to_string().contains("starter"), "{error}");
    assert!(link.has_ended());
    assert!(link.spawn(&shell("true")).is_err());
    // The child itself is nobody's now; ended here so the test leaves nothing.
    let pid = rustix::process::Pid::from_raw(i32::try_from(pid).expect("an id")).expect("not 0");
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
}

#[test]
fn a_frame_round_trips_its_descriptors_and_body() {
    let (one, two) = UnixStream::pair().expect("a pair");
    let file = File::open("/dev/null").expect("a descriptor");
    send(&one, SPAWN, b"body", &[file.as_fd(), file.as_fd()]).expect("sent");
    let frame = receive(&two).expect("received").expect("a frame");
    assert_eq!(
        (frame.kind, frame.body.as_slice(), frame.fds.len()),
        (SPAWN, &b"body"[..], 2)
    );
    drop(one);
    assert!(receive(&two).expect("an end").is_none());
}
