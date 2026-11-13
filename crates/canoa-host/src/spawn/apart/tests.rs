//! The starter and the host's side of its link, over a socket pair in this process: what a child is given, what the host hears of
//! it, and when a signal reaches its group.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::Shutdown;
use std::time::{Duration, Instant};

use std::mem::MaybeUninit;

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

/// A starter on a thread of this test and the host's raw end of its socket, which a test writes frames of its own to.
fn raw_starter() -> (UnixStream, thread::JoinHandle<io::Result<()>>) {
    let (host, starter) = UnixStream::pair().expect("a socket pair");
    host.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a bound");
    (host, thread::spawn(move || run(&starter)))
}

/// What a starter does with a frame that is not one: it ends, and the host is told by the socket ending, not by a wait.
fn ends_on(send_it: impl FnOnce(&UnixStream)) {
    let (host, starter) = raw_starter();
    send_it(&host);
    let mut byte = [0_u8; 1];
    let mut reading = &host;
    let read = std::io::Read::read(&mut reading, &mut byte);
    assert!(
        matches!(&read, Ok(0))
            || read
                .as_ref()
                .is_err_and(|e| e.kind() == io::ErrorKind::ConnectionReset),
        "{read:?}"
    );
    assert!(starter.join().expect("it ended").is_err());
}

fn header(kind: u8, count: u8, length: u32) -> [u8; HEADER] {
    let mut header = [kind, count, 0, 0, 0, 0, 0, 0];
    header[4..].copy_from_slice(&length.to_le_bytes());
    header
}

#[test]
fn a_frame_that_claims_descriptors_it_did_not_bring_ends_the_starter() {
    ends_on(|host| send_raw(host, header(SPAWN, 2, 0), &[], &[]).expect("sent"));
}

#[test]
fn a_frame_with_more_descriptors_than_a_frame_carries_ends_the_starter() {
    ends_on(|host| {
        let file = File::open("/dev/null").expect("a descriptor");
        let fds = [file.as_fd(), file.as_fd(), file.as_fd(), file.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        assert!(control.push(SendAncillaryMessage::ScmRights(&fds)));
        let message = header(SPAWN, 4, 0);
        sendmsg(
            host,
            &[IoSlice::new(&message)],
            &mut control,
            SendFlags::empty(),
        )
        .expect("sent");
    });
}

#[test]
fn a_frame_whose_body_stops_short_ends_the_starter() {
    ends_on(|host| {
        let mut writing = host;
        std::io::Write::write_all(&mut writing, &header(KILL, 0, 100)).expect("sent");
        std::io::Write::write_all(&mut writing, b"abc").expect("sent");
        host.shutdown(Shutdown::Write)
            .expect("the host's end of the frame ended");
    });
}

#[test]
fn a_frame_over_the_cap_ends_the_starter() {
    ends_on(|host| {
        let too_long = u32::try_from(MAX_BODY + 1).expect("a length");
        send_raw(host, header(SPAWN, 0, too_long), &[], &[]).expect("sent");
    });
}

#[test]
fn a_request_of_a_kind_unknown_ends_the_starter() {
    ends_on(|host| send_raw(host, header(99, 0, 0), &[], &[]).expect("sent"));
}

#[test]
fn a_signal_for_a_process_the_starter_never_started_reaches_nobody() {
    let (link, _end) = started();
    let mut bystander = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("a process of its own");
    // The starter never started it, and 0 and 1 name this group and every process: none is the starter's to signal.
    for pid in [bystander.id(), 0, 1] {
        for kind in [KILL_GROUP, KILL, COLLECT] {
            link.ask(kind, pid).expect("asked");
        }
    }
    // Still serving, and in order: a child started now is answered after the requests above were read.
    let mut after = link.spawn(&shell("exit 4")).expect("it starts");
    assert_eq!(after.wait().expect("it ends").code(), Some(4));
    assert!(
        alive(bystander.id()),
        "a process the starter did not start is left alone"
    );
    bystander.kill().expect("ended");
    bystander.wait().expect("collected");
}

#[cfg(target_os = "linux")]
#[test]
fn a_child_has_no_descriptor_past_its_three_streams_and_none_of_them_is_the_starters_socket() {
    let (link, _end) = started();
    let mut command = shell(
        "for n in 3 4 5 6 7 8 9 10 11 12 13 14 15; do [ -e /dev/fd/$n ] && echo open $n; done; \
         for n in 0 1 2; do [ -S /dev/fd/$n ] && echo socket $n; done; exit 0",
    );
    command
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Null);
    let mut child = link.spawn(&command).expect("it starts");
    let mut out = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().expect("piped"), &mut out).expect("read");
    child.wait().expect("it ends");
    // The test process itself passes on what its own parent left open (a build tool's descriptors): a plain child of it gets the
    // same, and the starter and its socket add nothing to it.
    let plain = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(
            "for n in 3 4 5 6 7 8 9 10 11 12 13 14 15; do [ -e /dev/fd/$n ] && echo open $n; done;              for n in 0 1 2; do [ -S /dev/fd/$n ] && echo socket $n; done; exit 0",
        )
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .expect("it runs");
    assert_eq!(
        out,
        String::from_utf8_lossy(&plain.stdout),
        "what the child holds besides its streams"
    );
}

#[test]
fn a_stream_closed_on_exec_is_a_closed_one_and_the_child_gets_nothing_behind_it() {
    // Opened by std, so closed on exec: what a child would inherit of it is nothing. Its other end is gone, so a copy of it
    // would refuse every write; the stand-in takes them all.
    let (read, write) = io::pipe().expect("a pipe");
    drop(read);
    let stand_in = inherit(OwnedFd::from(write).as_fd()).expect("a stand-in");
    File::from(stand_in)
        .write_all(b"dropped")
        .expect("a descriptor with nothing behind it takes a write");
}
