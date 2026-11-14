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
fn opened(dir: &Dir) -> (Forward, PathBuf) {
    let forward = Forward::open().expect("a listener");
    let (_, file) = forward.enroll("s1").expect("enrolled");
    let path = dir.0.join(CHANNEL_FILE);
    std::fs::write(&path, file).expect("written");
    (forward, path)
}

/// Runs the forwarder for `kind` in a thread, with `payload` as what the harness sends it.
fn forwarding(kind: &'static str, path: &Path, payload: &str) -> JoinHandle<Result<bool, String>> {
    let (path, payload) = (path.to_owned(), payload.to_owned());
    std::thread::spawn(move || forward(kind, &path, false, Cursor::new(payload.into_bytes())))
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
    raw_for(addr, bytes, Duration::from_millis(600))
}

/// As [`raw`], waiting up to `wait` for the answer.
fn raw_for(addr: SocketAddr, bytes: &[u8], wait: Duration) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(wait)).ok()?;
    let _ = stream.write_all(bytes);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    (!answer.is_empty()).then_some(answer)
}

#[test]
fn a_report_is_queued_answered_with_its_nonce_and_carries_only_the_fields_the_host_names() {
    let dir = Dir::new("report");
    let (host, path) = opened(&dir);
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
    let (host, path) = opened(&dir);
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
    let (host, path) = opened(&dir);
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
    let (host, path) = opened(&dir);
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
    let got = answered.join().expect("ran");
    assert!(got.is_some(), "no answer");
}

#[test]
fn a_forwarder_that_is_not_answered_fails_as_its_fail_mode_says() {
    let dir = Dir::new("fail");
    // A listener that takes each connection and closes it without a word.
    let silent = TcpListener::bind("127.0.0.1:0").expect("bound");
    let addr = silent.local_addr().expect("address");
    let hold = std::thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((stream, _)) = silent.accept() {
                drop(stream);
            }
        }
    });
    let path = dir.0.join("channel.json");
    std::fs::write(
        &path,
        json!({"addr": addr.to_string(), "secret": "s"}).to_string(),
    )
    .expect("written");
    let a = forward("pre-compact", &path, true, Cursor::new(b"{}".to_vec()));
    let b = forward("pre-compact", &path, false, Cursor::new(b"{}".to_vec()));
    assert!(
        a.is_err(),
        "a guard that cannot be asked stops what it guards: {a:?}"
    );
    assert_eq!(
        b,
        Ok(false),
        "any other hook that cannot be asked lets the harness go on"
    );
    let _ = hold.join();
    // A host that has gone: its port is closed.
    let gone = TcpListener::bind("127.0.0.1:0").expect("bound");
    let port = gone.local_addr().expect("address");
    drop(gone);
    std::fs::write(
        &path,
        json!({"addr": port.to_string(), "secret": "s"}).to_string(),
    )
    .expect("written");
    assert!(forward("pre-compact", &path, true, Cursor::new(b"{}".to_vec())).is_err());
    assert_eq!(
        forward("start", &path, false, Cursor::new(b"{}".to_vec())),
        Ok(false)
    );
}

#[test]
fn a_channel_that_is_missing_or_unreadable_fails_closed_for_a_guard_and_open_for_the_rest() {
    let dir = Dir::new("gone");
    let missing = dir.0.join("none.json");
    assert!(forward("pre-compact", &missing, true, Cursor::new(Vec::new())).is_err());
    assert_eq!(
        forward("pre-compact", &missing, false, Cursor::new(Vec::new())),
        Ok(false)
    );
    let garbled = dir.0.join("garbled.json");
    std::fs::write(&garbled, b"not json").expect("written");
    assert!(forward("pre-compact", &garbled, true, Cursor::new(Vec::new())).is_err());
    // A file that names an address off this computer is not dialled.
    let path = dir.0.join("channel.json");
    std::fs::write(
        &path,
        json!({"addr": "10.1.2.3:9", "secret": "s"}).to_string(),
    )
    .expect("written");
    let started = Instant::now();
    assert!(forward("pre-compact", &path, true, Cursor::new(Vec::new())).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "nothing was dialled"
    );
}

/// A stand-in for the host that answers one request with `answer(nonce)`.
fn answering(answer: impl FnOnce(&str) -> String + Send + 'static) -> (SocketAddr, JoinHandle<()>) {
    let host = TcpListener::bind("127.0.0.1:0").expect("bound");
    let addr = host.local_addr().expect("address");
    let serving = std::thread::spawn(move || {
        let (mut stream, _) = host.accept().expect("a request");
        let mut line = Vec::new();
        std::io::BufReader::new(&stream)
            .read_until(b'\n', &mut line)
            .expect("read");
        let request: Value = serde_json::from_slice(&line).expect("JSON");
        let nonce = request["nonce"].as_str().expect("a nonce").to_owned();
        stream
            .write_all((answer(&nonce) + "\n").as_bytes())
            .expect("answered");
    });
    (addr, serving)
}

fn channel_to(dir: &Dir, addr: SocketAddr) -> PathBuf {
    let path = dir.0.join("channel.json");
    std::fs::write(
        &path,
        json!({"addr": addr.to_string(), "secret": "the-secret"}).to_string(),
    )
    .expect("written");
    path
}

#[test]
fn an_answer_that_does_not_echo_the_nonce_is_no_answer() {
    let dir = Dir::new("nonce");
    let (addr, serving) =
        answering(|_| json!({"nonce": "not-the-one", "block": false}).to_string());
    let path = channel_to(&dir, addr);
    assert!(
        forward("pre-compact", &path, true, Cursor::new(Vec::new())).is_err(),
        "an answer for another request does not pass for this one's"
    );
    serving.join().expect("ran");
    let (addr, serving) = answering(|nonce| json!({"nonce": nonce, "block": true}).to_string());
    let path = channel_to(&dir, addr);
    assert_eq!(
        forward("pre-compact", &path, true, Cursor::new(Vec::new())),
        Ok(true)
    );
    serving.join().expect("ran");
}

#[test]
fn a_block_stops_only_a_compaction() {
    let dir = Dir::new("kinds");
    for kind in ["start", "batch"] {
        let (addr, serving) = answering(|nonce| json!({"nonce": nonce, "block": true}).to_string());
        let path = channel_to(&dir, addr);
        assert_eq!(
            forward(kind, &path, false, Cursor::new(Vec::new())),
            Ok(false),
            "{kind}"
        );
        serving.join().expect("ran");
    }
}

#[test]
fn a_peer_that_sends_a_byte_at_a_time_does_not_keep_a_place() {
    let dir = Dir::new("trickle");
    let (host, path) = opened(&dir);
    let addr = addr_of(&path);
    let stop = Arc::new(AtomicBool::new(false));
    // Each sends a byte before the read limit is up, and again, as a peer keeping a handler would.
    let tricklers: Vec<_> = (0..MAX_HANDLERS * 2)
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
                else {
                    return;
                };
                while !stop.load(Ordering::SeqCst) {
                    if stream.write_all(b"{").is_err() {
                        return;
                    }
                    std::thread::sleep(READ_LIMIT / 3);
                }
            })
        })
        .collect();
    std::thread::sleep(READ_LIMIT * 4);
    let run = forwarding("batch", &path, "{\"cwd\":\"/x\"}");
    let mut taken = reported(&host, 1);
    assert_eq!(
        taken.len(),
        1,
        "the trickling connections gave up their places at the deadline"
    );
    taken.remove(0).answer(Verdict::default());
    assert_eq!(run.join().expect("ran"), Ok(false));
    stop.store(true, Ordering::SeqCst);
    for each in tricklers {
        let _ = each.join();
    }
}

#[test]
fn idle_connections_do_not_keep_a_report_from_being_heard() {
    let dir = Dir::new("idle");
    let (host, path) = opened(&dir);
    let addr = addr_of(&path);
    // More idle connections than there are handlers, as another user could hold open.
    let idle: Vec<TcpStream> = (0..MAX_HANDLERS * 3)
        .filter_map(|_| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok())
        .collect();
    std::thread::sleep(READ_LIMIT * 3);
    let run = forwarding("batch", &path, "{\"cwd\":\"/x\"}");
    let mut taken = reported(&host, 1);
    assert_eq!(
        taken.len(),
        1,
        "the idle connections gave up their places within the read limit"
    );
    taken.remove(0).answer(Verdict::default());
    assert_eq!(run.join().expect("ran"), Ok(false));
    drop(idle);
}

#[test]
fn a_session_has_a_limited_number_of_secrets_and_each_ends_alone() {
    let host = Forward::open().expect("a listener");
    let mut secrets = Vec::new();
    for at in 0..MAX_SECRETS {
        secrets.push(host.enroll(&format!("s{at}")).expect("enrolled").0);
    }
    assert!(host.enroll("one-more").is_err());
    host.revoke_secret(&secrets[0]);
    assert!(
        host.enroll("one-more").is_ok(),
        "an ended secret leaves room"
    );
}

#[test]
fn a_command_is_written_only_for_paths_that_quote_safely() {
    let program = if cfg!(windows) {
        "C:\\Program Files\\canoa\\canoa.exe"
    } else {
        "/opt/canoa/canoa"
    };
    let channel = Path::new("C:/tmp/canoa-profile-1/ab/channel.json");
    let command = command_for(program, &[], channel, Kind::PreCompact, false).expect("quotable");
    let guard = command_for(program, &[], channel, Kind::PreCompact, true).expect("quotable");
    assert!(
        guard.ends_with("\" --refuse pre-compact"),
        "the fail mode is on the command line: {guard}"
    );
    assert!(!command.contains("--refuse"));
    assert!(
        command.ends_with(
            "hook-forward --channel \"C:/tmp/canoa-profile-1/ab/channel.json\" pre-compact"
        ),
        "{command}"
    );
    assert!(!command.contains('\\'), "{command}");
    for bad in [
        "/opt/$HOME/canoa",
        "/opt/a\"b/canoa",
        "/opt/a`b/canoa",
        "/opt/a%b/canoa",
        "/opt/a!b/canoa",
    ] {
        assert_eq!(
            command_for(bad, &[], channel, Kind::Start, false),
            None,
            "{bad}"
        );
    }
    // A name inside double quotes is one word for bash and PowerShell, and these are no more than letters there.
    for fine in ["/opt/O'Brien/canoa", "/opt/R&D/canoa", "/opt/a;b/canoa"] {
        let each = command_for(fine, &[], channel, Kind::Start, false).expect(fine);
        assert!(
            each.starts_with(&format!("\"{fine}\" hook-forward")),
            "{each}"
        );
    }
    assert_eq!(
        command_for(program, &["--x\n".to_owned()], channel, Kind::Start, false),
        None
    );
    let hooks = hooks_json(&[(Kind::PreCompact, command.clone())]);
    assert_eq!(
        hooks["hooks"]["PreCompact"][0]["hooks"][0]["shell"], "bash",
        "the command is written for bash"
    );
    // The secret is not in what the command line shows.
    let dir = Dir::new("cmd");
    let (host, path) = opened(&dir);
    let secret = secret_of(&path);
    assert!(
        !command_for(program, &[], &path, Kind::Start, false)
            .expect("quotable")
            .contains(&secret)
    );
    drop(host);
}
