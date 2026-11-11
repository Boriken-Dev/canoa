//! `PipeChannel` on a real pipe or Unix socket: whole messages, the bounds on either side, and how an end is told from a
//! failure.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use canoa_host::pipe::{Listener, PipeChannel, PipeName, Stream};
use coqui_web_channel::{Channel as _, Failure};

/// The directory a socket needs on Unix, removed when the test ends.
struct Directory(Option<std::path::PathBuf>);

impl Drop for Directory {
    fn drop(&mut self) {
        if let Some(directory) = &self.0 {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

struct Pair {
    channel: PipeChannel,
    /// The other end as the bytes of a stream, to write what a peer should never write.
    raw: Stream,
    _directory: Directory,
}

fn pair(max_message: usize) -> Pair {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let number = NEXT.fetch_add(1, Ordering::SeqCst);
    let (name, directory) = if cfg!(windows) {
        let name = format!("pipe:canoa-channel-test-{}-{number}", std::process::id());
        (name, None)
    } else {
        let directory = std::env::temp_dir().join(format!(
            "canoa-channel-test-{}-{number}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        (
            format!("unix:{}", directory.join("c.sock").display()),
            Some(directory),
        )
    };
    let name: PipeName = name.parse().expect("a pipe name");
    let listener = Listener::bind(&name).expect("the pipe binds");
    let accepting = thread::spawn(move || listener.accept().expect("a client"));
    let raw = Stream::connect(&name).expect("the client connects");
    let served = accepting.join().expect("accepted");
    Pair {
        channel: PipeChannel::new(served, max_message),
        raw,
        _directory: Directory(directory),
    }
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut framed = u32::try_from(body.len())
        .expect("a small body")
        .to_le_bytes()
        .to_vec();
    framed.extend_from_slice(body);
    framed
}

#[test]
fn a_message_comes_whole_however_the_peer_cut_it_up() {
    let pair = pair(1024);
    let bytes = [frame(b"first"), frame(b"second one")].concat();
    let mut writer = &pair.raw;
    for chunk in bytes.chunks(1) {
        writer.write_all(chunk).expect("written");
    }
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"first".to_vec()))
    );
    writer.write_all(&frame(b"third")).expect("written");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"second one".to_vec()))
    );
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"third".to_vec()))
    );
}

#[test]
fn a_message_over_the_bound_or_of_no_length_ends_the_channel() {
    for (name, bytes) in [
        ("over the bound", frame(&[7; 33])),
        ("of no length", vec![0, 0, 0, 0]),
        ("claiming four gigabytes", vec![0xff, 0xff, 0xff, 0xff]),
    ] {
        let pair = pair(32);
        (&pair.raw).write_all(&bytes).expect("written");
        assert_eq!(
            pair.channel.receive(Some(Duration::from_secs(5))),
            Err(Failure::Protocol),
            "a message {name}"
        );
        // It does not read on past a frame it refused.
        assert!(
            !matches!(
                pair.channel.receive(Some(Duration::from_millis(200))),
                Ok(Some(_))
            ),
            "{name}"
        );
    }
}

#[test]
fn the_bound_the_host_raises_after_the_proof_applies_to_the_next_message() {
    let pair = pair(8);
    pair.channel.set_max_message(64);
    (&pair.raw).write_all(&frame(&[1; 40])).expect("written");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(vec![1; 40]))
    );
    pair.channel.set_max_receive(4);
    (&pair.raw).write_all(&frame(&[1; 5])).expect("written");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Err(Failure::Protocol)
    );
}

#[test]
fn a_receive_that_times_out_loses_nothing_of_a_message_begun() {
    let pair = pair(1024);
    let framed = frame(b"in two parts");
    let mut writer = &pair.raw;
    writer.write_all(&framed[..2]).expect("written");
    let started = Instant::now();
    assert_eq!(
        pair.channel.receive(Some(Duration::from_millis(150))),
        Err(Failure::Timeout)
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    writer.write_all(&framed[2..7]).expect("written");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_millis(150))),
        Err(Failure::Timeout)
    );
    writer.write_all(&framed[7..]).expect("written");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"in two parts".to_vec()))
    );
}

#[test]
fn the_peer_leaving_between_messages_is_an_end_and_in_the_middle_of_one_a_failure() {
    let pair = pair(1024);
    (&pair.raw).write_all(&frame(b"last")).expect("written");
    pair.raw.shutdown().expect("the peer leaves");
    let channel = &pair.channel;
    assert_eq!(
        channel.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"last".to_vec())),
        "what was sent before the end is read first"
    );
    assert_eq!(channel.receive(Some(Duration::from_secs(5))), Ok(None));

    let pair = self::pair(1024);
    let framed = frame(b"cut short");
    (&pair.raw).write_all(&framed[..6]).expect("written");
    pair.raw.shutdown().expect("the peer leaves");
    assert_eq!(
        pair.channel.receive(Some(Duration::from_secs(5))),
        Err(Failure::PathLost)
    );
}

#[test]
fn closing_wakes_a_receive_that_is_waiting() {
    let pair = pair(1024);
    let channel = std::sync::Arc::new(pair.channel);
    let waiting = {
        let channel = std::sync::Arc::clone(&channel);
        thread::spawn(move || channel.receive(None))
    };
    thread::sleep(Duration::from_millis(200));
    channel.close();
    let answer = waiting.join().expect("the receive returned");
    assert!(
        matches!(answer, Ok(None) | Err(Failure::PathLost)),
        "{answer:?}"
    );
}

#[test]
fn what_one_end_sends_the_other_receives_and_a_message_out_of_bounds_is_refused_to_send() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let number = NEXT.fetch_add(1, Ordering::SeqCst) + 1000;
    let (name, directory) = if cfg!(windows) {
        (
            format!("pipe:canoa-channel-test-{}-{number}", std::process::id()),
            None,
        )
    } else {
        let directory = std::env::temp_dir().join(format!(
            "canoa-channel-test-{}-{number}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        (
            format!("unix:{}", directory.join("c.sock").display()),
            Some(directory),
        )
    };
    let name: PipeName = name.parse().expect("a pipe name");
    let listener = Listener::bind(&name).expect("the pipe binds");
    let accepting = thread::spawn(move || listener.accept().expect("a client"));
    let client = PipeChannel::connect(&name, 64).expect("the client connects");
    let server = PipeChannel::new(accepting.join().expect("accepted"), 64);

    client.send(b"to the host").expect("sent");
    assert_eq!(
        server.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"to the host".to_vec()))
    );
    server.send(&[9; 64]).expect("sent");
    assert_eq!(
        client.receive(Some(Duration::from_secs(5))),
        Ok(Some(vec![9; 64]))
    );
    assert_eq!(client.send(&[]), Err(Failure::Protocol), "no empty message");
    assert_eq!(
        client.send(&[0; 65]),
        Err(Failure::Protocol),
        "over the bound"
    );
    client
        .send(b"still usable")
        .expect("a refused send leaves the channel");
    assert_eq!(
        server.receive(Some(Duration::from_secs(5))),
        Ok(Some(b"still usable".to_vec()))
    );
    if let Some(directory) = directory {
        let _ = std::fs::remove_dir_all(directory);
    }
}
