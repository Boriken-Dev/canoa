use std::io::Cursor;
use std::net::TcpListener;
use std::path::PathBuf;

use super::*;

/// A directory of the test's own.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("canoa-forward-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A host's end with one session enrolled, and the channel file its forwarder reads.
fn opened(dir: &Dir, refuse: &[&'static str]) -> (Forward, PathBuf) {
    let forward = Forward::open().expect("a listener");
    let fail = refuse.iter().map(|kind| (*kind, true)).collect();
    let file = forward.enroll("s1", &fail).expect("enrolled");
    let path = dir.0.join(CHANNEL_FILE);
    std::fs::write(&path, file).expect("written");
    (forward, path)
}

/// Runs the forwarder for `kind` in a thread, with `payload` as what the harness sends it.
fn forwarding(kind: &'static str, path: &Path, payload: &str) -> JoinHandle<Result<bool, String>> {
    let (path, payload) = (path.to_owned(), payload.to_owned());
    std::thread::spawn(move || forward(kind, &path, Cursor::new(payload.into_bytes())))
}

/// Everything reported, once at least `count` reports are there.
fn reported(forward: &Forward, count: usize) -> Vec<Forwarded> {
    let until = Instant::now() + Duration::from_secs(10);
    let mut all = Vec::new();
    while all.len() < count && Instant::now() < until {
        all.extend(forward.take());
        std::thread::sleep(Duration::from_millis(10));
    }
    all
}

fn secret_of(path: &Path) -> String {
    let file: Value = serde_json::from_slice(&std::fs::read(path).expect("read")).expect("JSON");
    file["secret"].as_str().expect("a secret").to_owned()
}

fn addr_of(path: &Path) -> SocketAddr {
    let file: Value = serde_json::from_slice(&std::fs::read(path).expect("read")).expect("JSON");
    file["addr"]
        .as_str()
        .expect("an address")
        .parse()
        .expect("an address")
}

/// One raw request to the host's port: what an outside process that knows the address could send.
fn raw(addr: SocketAddr, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(600)))
        .ok()?;
    let _ = stream.write_all(bytes);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    (!answer.is_empty()).then_some(answer)
}

#[test]
fn a_report_is_queued_answered_with_its_nonce_and_carries_only_the_fields_the_host_names() {
    let dir = Dir::new("report");
    let (host, path) = opened(&dir, &[]);
    let payload = json!({
        "cwd": "/work/sub", "trigger": "manual", "source": "compact",
        "tool_input": {"command": "cat ~/.ssh/id"}, "transcript_path": "/x", "prompt": "secret words",
    })
    .to_string();
    let run = forwarding("pre-compact", &path, &payload);
    let mut taken = reported(&host, 1);
    assert_eq!(taken.len(), 1, "one report");
    let report = taken.remove(0);
    assert_eq!(report.session, "s1");
    assert_eq!(report.kind, Kind::PreCompact);
    assert_eq!(report.cwd.as_deref(), Some("/work/sub"));
    assert!(report.source.is_some());
    report.answer(Verdict { block: true });
    assert_eq!(
        run.join().expect("ran"),
        Ok(true),
        "the verdict is what the forwarder returns"
    );
}

#[test]
fn a_request_with_a_secret_that_is_not_held_is_dropped_with_no_answer_and_queues_nothing() {
    let dir = Dir::new("secret");
    let (host, path) = opened(&dir, &[]);
    let addr = addr_of(&path);
    let wrong = "0".repeat(secret_of(&path).len());
    let line = format!(
        "{{\"secret\":\"{wrong}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"pre-compact\",\"fields\":{{}}}}\n"
    );
    assert_eq!(raw(addr, line.as_bytes()), None);
    assert!(host.take().is_empty());
    // The right one on the same port is taken: the port itself is no secret.
    let right = secret_of(&path);
    let line = format!(
        "{{\"secret\":\"{right}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{}}}}\n"
    );
    let answered = std::thread::spawn(move || raw(addr, line.as_bytes()));
    let mut taken = reported(&host, 1);
    assert_eq!(taken.len(), 1);
    taken.remove(0).answer(Verdict::default());
    let answer = answered.join().expect("ran").expect("an answer");
    let answer: Value = serde_json::from_slice(&answer).expect("JSON");
    assert_eq!(answer["nonce"], "abcdefgh12345678");
}

#[test]
fn a_secret_ends_with_its_session_and_with_the_connection() {
    let dir = Dir::new("revoke");
    let (host, path) = opened(&dir, &[]);
    let secret = secret_of(&path);
    host.revoke("s1");
    let line = format!(
        "{{\"secret\":\"{secret}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{}}}}\n"
    );
    assert_eq!(raw(addr_of(&path), line.as_bytes()), None);
    assert!(host.take().is_empty());
    let addr = addr_of(&path);
    drop(host);
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err(),
        "the listener went with the connection"
    );
}

#[test]
fn a_request_past_the_size_bound_or_not_one_request_is_dropped() {
    let dir = Dir::new("size");
    let (host, path) = opened(&dir, &[]);
    let addr = addr_of(&path);
    let secret = secret_of(&path);
    let big = "x".repeat(MAX_REQUEST + 100);
    let line = format!(
        "{{\"secret\":\"{secret}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{\"cwd\":\"{big}\"}}}}\n"
    );
    assert_eq!(raw(addr, line.as_bytes()), None);
    assert_eq!(raw(addr, b"not json\n"), None);
    assert_eq!(raw(addr, b"{\"secret\":1}\n"), None);
    // A field past its own bound is left out, not cut: the report has no `cwd`.
    let long = "y".repeat(MAX_FIELD + 1);
    let line = format!(
        "{{\"secret\":\"{secret}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{\"cwd\":\"{long}\"}}}}\n"
    );
    let answered = std::thread::spawn(move || raw(addr, line.as_bytes()));
    let mut taken = reported(&host, 1);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].cwd, None);
    taken.remove(0).answer(Verdict::default());
    assert!(answered.join().expect("ran").is_some());
}

#[test]
fn a_session_may_report_only_so_often() {
    let dir = Dir::new("rate");
    let (host, path) = opened(&dir, &[]);
    let addr = addr_of(&path);
    let secret = secret_of(&path);
    let line = format!("{{\"secret\":\"{secret}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{}}}}
");
    let mut taken = 0;
    for _ in 0..RATE + 10 {
        let sent = line.clone();
        let asked = std::thread::spawn(move || raw(addr, sent.as_bytes()));
        let until = Instant::now() + Duration::from_millis(400);
        let mut got = Vec::new();
        while got.is_empty() && Instant::now() < until {
            got = host.take();
            std::thread::sleep(Duration::from_millis(5));
        }
        taken += got.len();
        for each in got {
            each.answer(Verdict::default());
        }
        let _ = asked.join();
    }
    assert_eq!(
        taken, RATE,
        "{RATE} reports a window are taken and the rest are dropped"
    );
}

#[test]
fn many_wrong_secrets_in_a_row_are_slowed_and_the_right_one_is_still_heard() {
    let dir = Dir::new("guess");
    let (host, path) = opened(&dir, &[]);
    let addr = addr_of(&path);
    let started = Instant::now();
    for guess in 0..BAD_FREE + 3 {
        let line = format!("{{\"secret\":\"{guess:032}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{}}}}
");
        assert_eq!(raw(addr, line.as_bytes()), None);
    }
    assert!(host.take().is_empty());
    assert!(
        started.elapsed() >= BAD_WAIT,
        "the guesses past the bound waited"
    );
    let right = secret_of(&path);
    let line = format!("{{\"secret\":\"{right}\",\"nonce\":\"abcdefgh12345678\",\"kind\":\"batch\",\"fields\":{{}}}}
");
    let answered = std::thread::spawn(move || raw(addr, line.as_bytes()));
    let mut taken = reported(&host, 1);
    assert_eq!(
        taken.len(),
        1,
        "the secret that is held is heard after the guesses"
    );
    taken.remove(0).answer(Verdict::default());
    assert!(answered.join().expect("ran").is_some());
}

#[test]
fn a_forwarder_that_is_not_answered_fails_as_its_fail_mode_says() {
    let dir = Dir::new("fail");
    // A listener that takes each connection and closes it without a word.
    let silent = TcpListener::bind("127.0.0.1:0").expect("bound");
    let addr = silent.local_addr().expect("address");
    let hold = std::thread::spawn(move || {
        for _ in 0..3 {
            if let Ok((stream, _)) = silent.accept() {
                drop(stream);
            }
        }
    });
    let write = |refuse: bool| {
        let path = dir.0.join(format!("channel-{refuse}.json"));
        std::fs::write(
            &path,
            json!({"addr": addr.to_string(), "secret": "s", "refuse": if refuse { vec!["pre-compact"] } else { Vec::new() }}).to_string(),
        )
        .expect("written");
        path
    };
    let (refusing, passing) = (write(true), write(false));
    let a = forward("pre-compact", &refusing, Cursor::new(b"{}".to_vec()));
    let b = forward("pre-compact", &passing, Cursor::new(b"{}".to_vec()));
    assert!(
        a.is_err(),
        "a guard that cannot be asked stops what it guards: {a:?}"
    );
    assert_eq!(
        b,
        Ok(false),
        "any other hook that cannot be asked lets the harness go on"
    );
    // Another kind is not stopped by a mode that names pre-compact.
    assert_eq!(
        forward("batch", &refusing, Cursor::new(b"{}".to_vec())),
        Ok(false)
    );
    let _ = hold.join();
    // A host that has gone: its port is closed.
    let gone = TcpListener::bind("127.0.0.1:0").expect("bound");
    let port = gone.local_addr().expect("address");
    drop(gone);
    let path = dir.0.join("channel-gone.json");
    std::fs::write(
        &path,
        json!({"addr": port.to_string(), "secret": "s", "refuse": ["pre-compact"]}).to_string(),
    )
    .expect("written");
    assert!(forward("pre-compact", &path, Cursor::new(b"{}".to_vec())).is_err());
    assert_eq!(
        forward("start", &path, Cursor::new(b"{}".to_vec())),
        Ok(false)
    );
}

#[test]
fn a_channel_that_is_gone_or_not_on_loopback_stops_nothing_and_asks_nobody() {
    let dir = Dir::new("gone");
    let missing = dir.0.join("none.json");
    assert_eq!(
        forward("pre-compact", &missing, Cursor::new(Vec::new())),
        Ok(false)
    );
    // A file that names an address off this computer is not dialled.
    let path = dir.0.join("channel.json");
    std::fs::write(
        &path,
        json!({"addr": "10.1.2.3:9", "secret": "s", "refuse": ["pre-compact"]}).to_string(),
    )
    .expect("written");
    let started = Instant::now();
    assert!(forward("pre-compact", &path, Cursor::new(Vec::new())).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "nothing was dialled"
    );
}

#[test]
fn an_answer_that_does_not_echo_the_nonce_is_no_answer() {
    let dir = Dir::new("nonce");
    let liar = TcpListener::bind("127.0.0.1:0").expect("bound");
    let addr = liar.local_addr().expect("address");
    let serving = std::thread::spawn(move || {
        let (mut stream, _) = liar.accept().expect("a request");
        let mut line = Vec::new();
        std::io::BufReader::new(&stream)
            .read_until(b'\n', &mut line)
            .expect("read");
        stream
            .write_all(b"{\"nonce\":\"not-the-one\",\"block\":false}\n")
            .expect("answered");
    });
    let path = dir.0.join("channel.json");
    std::fs::write(
        &path,
        json!({"addr": addr.to_string(), "secret": "s", "refuse": ["pre-compact"]}).to_string(),
    )
    .expect("written");
    assert!(
        forward("pre-compact", &path, Cursor::new(Vec::new())).is_err(),
        "a stranger's answer does not pass for the host's"
    );
    serving.join().expect("ran");
}

#[test]
fn a_command_is_written_only_for_paths_that_quote_safely() {
    let program = if cfg!(windows) {
        "C:\\Program Files\\canoa\\canoa.exe"
    } else {
        "/opt/canoa/canoa"
    };
    let channel = Path::new("C:/tmp/canoa-profile-1/ab/channel.json");
    let command = command_for(program, &[], channel, Kind::PreCompact).expect("quotable");
    assert!(
        command.ends_with(
            "hook-forward --channel \"C:/tmp/canoa-profile-1/ab/channel.json\" pre-compact"
        ),
        "{command}"
    );
    assert!(!command.contains('\\'), "{command}");
    for bad in [
        "/opt/it's/canoa",
        "/opt/$HOME/canoa",
        "/opt/a\"b/canoa",
        "/opt/a`b/canoa",
        "/opt/a;b/canoa",
        "/opt/a%b/canoa",
    ] {
        assert_eq!(command_for(bad, &[], channel, Kind::Start), None, "{bad}");
    }
    assert_eq!(
        command_for(program, &["--x\n".to_owned()], channel, Kind::Start),
        None
    );
    // The secret is not in what the command line shows.
    let dir = Dir::new("cmd");
    let (host, path) = opened(&dir, &[]);
    let secret = secret_of(&path);
    assert!(
        !command_for(program, &[], &path, Kind::Start)
            .expect("quotable")
            .contains(&secret)
    );
    drop(host);
}
