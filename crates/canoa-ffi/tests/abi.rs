//! The C ABI as a foreign caller uses it: every pointer checked, every job a number, and `serve` holding its home while it
//! runs and giving it back when stopped.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use canoa_ffi::exports::{
    ABI_VERSION, STATUS_BAD_ARGUMENT, STATUS_ENDED, STATUS_NO_ENTRY, STATUS_OK, STATUS_UNKNOWN_JOB,
    STATUS_UNKNOWN_LINK, canoa_ffi_abi, canoa_ffi_job_free, canoa_ffi_job_read,
    canoa_ffi_job_start, canoa_ffi_job_stop, canoa_ffi_link_close, canoa_ffi_link_open,
    canoa_ffi_link_read, canoa_ffi_link_send, canoa_ffi_text_free,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-ffi-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }

    /// An empty canoa.cfg, for a host with nothing to serve.
    fn config(&self) -> PathBuf {
        let path = self.0.join("canoa.cfg");
        std::fs::write(&path, "").expect("written");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start(request: &Value) -> u64 {
    let text = request.to_string();
    let mut job = u64::MAX;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(text.as_ptr(), text.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK, "{request}");
    assert_ne!(job, 0);
    job
}

fn read(job: u64) -> Value {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_job_read(job, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    value
}

/// What a job said, up to the end of whatever `done` is waiting for.
fn until(job: u64, what: &str, done: impl Fn(&[String], &[String], Option<i64>) -> bool) -> Said {
    let mut said = Said::default();
    let deadline = Instant::now() + WAIT;
    loop {
        let value = read(job);
        for (key, into) in [("out", &mut said.out), ("err", &mut said.err)] {
            into.extend(
                value[key]
                    .as_array()
                    .expect("a list")
                    .iter()
                    .map(|line| line.as_str().expect("text").to_owned()),
            );
        }
        said.exit = value["exit"].as_i64();
        if done(&said.out, &said.err, said.exit) {
            return said;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {said:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Debug, Default)]
struct Said {
    out: Vec<String>,
    err: Vec<String>,
    exit: Option<i64>,
}

fn to_end(job: u64) -> Said {
    until(job, "its end", |_, _, exit| exit.is_some())
}

fn serve(config: &Path, home: &Path, holder: &str) -> u64 {
    start(&json!({
        "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", config],
        "home": home,
        "holder": holder,
    }))
}

#[test]
fn it_says_its_version() {
    assert_eq!(canoa_ffi_abi(), ABI_VERSION);
}

#[test]
fn a_null_or_aliased_pointer_is_refused_and_the_out_parameter_is_written_first() {
    let mut job = 9_u64;
    // SAFETY: a null request is what is being refused; `job` lives across the call.
    let status = unsafe { canoa_ffi_job_start(core::ptr::null(), 0, &raw mut job) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);
    assert_eq!(job, 0, "written first");

    // SAFETY: a null out-parameter is what is being refused.
    let status = unsafe { canoa_ffi_job_start(b"{}".as_ptr(), 2, core::ptr::null_mut()) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);

    let mut len = 9_usize;
    // SAFETY: a null out-parameter is what is being refused; `len` lives across the call.
    let status = unsafe { canoa_ffi_job_read(1, core::ptr::null_mut(), &raw mut len) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);

    let mut both = [0_usize; 1];
    let alias = both.as_mut_ptr();
    // SAFETY: the two name one word, which is what is being refused.
    let status = unsafe { canoa_ffi_job_read(1, alias.cast(), alias) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);

    let misaligned = both
        .as_mut_ptr()
        .cast::<u8>()
        .wrapping_add(1)
        .cast::<usize>();
    let mut text = core::ptr::null_mut();
    // SAFETY: a misaligned pointer is what is being refused; it is never written through.
    let status = unsafe { canoa_ffi_job_read(1, &raw mut text, misaligned) };
    assert_eq!(status, STATUS_BAD_ARGUMENT);

    // SAFETY: null frees nothing.
    unsafe { canoa_ffi_text_free(core::ptr::null_mut(), 0) };
}

#[test]
fn a_request_it_cannot_read_is_refused() {
    for request in [
        &b"not json"[..],
        b"{}",
        br#"{"args":"serve"}"#,
        br#"{"args":[],"unknown":1}"#,
        br#"{"args":[],"program_args":["x"]}"#,
        b"\xff\xfe",
    ] {
        let mut job = 9_u64;
        // SAFETY: both point at what lives across the call.
        let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
        assert_eq!(status, STATUS_BAD_ARGUMENT, "{request:?}");
        assert_eq!(job, 0);
    }
}

#[test]
fn a_number_that_names_no_job_is_said_so() {
    let (mut text, mut len) = (core::ptr::null_mut(), 9_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_job_read(u64::MAX, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_UNKNOWN_JOB);
    assert!(text.is_null());
    assert_eq!(len, 0);
    assert_eq!(canoa_ffi_job_stop(u64::MAX), STATUS_UNKNOWN_JOB);
    assert_eq!(canoa_ffi_job_free(u64::MAX), STATUS_UNKNOWN_JOB);
    assert_eq!(canoa_ffi_job_free(0), STATUS_UNKNOWN_JOB);
}

#[test]
fn a_job_freed_is_gone_for_good() {
    let scratch = Scratch::new();
    let job = start(&json!({"args": ["peers", "--format", "json"], "home": scratch.0}));
    assert_eq!(to_end(job).exit, Some(0));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
    assert_eq!(canoa_ffi_job_free(job), STATUS_UNKNOWN_JOB);
    assert_eq!(canoa_ffi_job_stop(job), STATUS_UNKNOWN_JOB);
}

/// The job that asks a source's choices needs no session and no program: an entry that runs a program on standard streams
/// says it lists its models only once it has started.
#[test]
fn a_job_asks_a_sources_choices_with_no_session() {
    let scratch = Scratch::new();
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(
        &config,
        "[host.agents.Plain]
program = \"agy\"
args = [\"acp\"]
",
    )
    .expect("written");
    let job = start(&json!({
        "args": ["options", "Plain", "--entry", "--config", config, "--format", "json"],
        "home": scratch.0,
    }));
    let said = to_end(job);
    assert_eq!(said.exit, Some(0), "{said:?}");
    let choices: Value = serde_json::from_str(&said.out[0]).expect("one JSON object");
    assert_eq!(choices["known"], false);
    assert_eq!(choices["models"], json!([]));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}

#[test]
fn a_wrong_command_line_ends_with_status_2_and_says_why() {
    let job = start(&json!({"args": ["no-such-command"]}));
    let said = to_end(job);
    assert_eq!(said.exit, Some(2));
    assert!(said.err.iter().any(|line| line.contains("no-such-command")));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}

#[test]
fn serving_holds_the_home_and_a_second_host_says_who_until_the_first_stops() {
    let scratch = Scratch::new();
    let (config, home) = (scratch.config(), scratch.0.join("home"));
    let first = serve(&config, &home, "CanoaUI");
    let said = until(first, "it to be up", |out, _, _| !out.is_empty());
    let ready: Value = serde_json::from_str(&said.out[0]).expect("one JSON object");
    assert_eq!(ready["ready"], true);
    assert!(ready["peer"].is_string());

    let second = serve(&config, &home, "another");
    let refused = to_end(second);
    assert_eq!(refused.exit, Some(3), "{refused:?}");
    let last = refused
        .err
        .iter()
        .find(|line| line.starts_with("canoa: "))
        .expect("it says why");
    assert!(last.contains("CanoaUI"), "{last}");
    assert!(last.contains(&std::process::id().to_string()), "{last}");
    assert_eq!(canoa_ffi_job_free(second), STATUS_OK);

    assert_eq!(canoa_ffi_job_stop(first), STATUS_OK);
    assert_eq!(to_end(first).exit, Some(0));
    assert_eq!(
        canoa_ffi_job_stop(first),
        STATUS_OK,
        "stopping twice is fine"
    );
    assert_eq!(canoa_ffi_job_free(first), STATUS_OK);

    // Its end says everything is given up, so a next one starts at once.
    let next = serve(&config, &home, "CanoaUI");
    until(next, "the next to be up", |out, _, _| !out.is_empty());
    assert_eq!(canoa_ffi_job_free(next), STATUS_OK, "freeing stops it");
    let deadline = Instant::now() + WAIT;
    loop {
        let again = serve(&config, &home, "last");
        let said = until(again, "it to start or refuse", |out, _, exit| {
            !out.is_empty() || exit.is_some()
        });
        let up = !said.out.is_empty();
        assert_eq!(canoa_ffi_job_free(again), STATUS_OK);
        if up {
            break;
        }
        assert!(Instant::now() < deadline, "the home was not given up");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn stopping_before_the_host_is_up_still_stops_it() {
    let scratch = Scratch::new();
    let (config, home) = (scratch.config(), scratch.0.join("home"));
    let job = serve(&config, &home, "CanoaUI");
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    assert_eq!(to_end(job).exit, Some(0));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}

/// `provider`, `tool`, `agent` and `launch` change the process they run in, its directory, its standard input and its end,
/// so in an application's process they are refused: they run only as a process of their own.
#[test]
fn a_hosts_own_command_is_refused_as_a_job() {
    let scratch = Scratch::new();
    let before = std::env::current_dir().expect("a directory");
    for args in [
        json!([
            "provider",
            "--library",
            "none",
            "--provider",
            "p",
            "--directory",
            scratch.0
        ]),
        json!(["launch", "--", "none"]),
        json!(["tool", "--name", "t", "--spec", "{}"]),
        json!(["agent", "claude", "--executable", "none"]),
    ] {
        let job = start(&json!({"args": args, "home": scratch.0}));
        let said = to_end(job);
        assert_eq!(said.exit, Some(2), "{args}: {said:?}");
        assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
    }
    assert_eq!(std::env::current_dir().expect("a directory"), before);
}

/// An application that leaves `SIGPIPE` as the system gives it would end the first time the host writes to an agent that has
/// died: hosting sets it aside.
#[cfg(unix)]
#[test]
fn hosting_sets_broken_pipes_aside() {
    let scratch = Scratch::new();
    let probe = child_entry().with_file_name("signal_probe");
    let said = Command::new(probe)
        .arg(&scratch.0)
        .output()
        .expect("it runs");
    assert!(said.status.success(), "{said:?}");
    assert_eq!(String::from_utf8_lossy(&said.stdout).trim(), "ignored=true");
}

fn child_entry() -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile
        .join("examples")
        .join(format!("child_entry{}", std::env::consts::EXE_SUFFIX))
}

#[test]
fn as_a_hosts_own_command_it_runs_what_a_host_starts_and_nothing_else() {
    let refused = Command::new(child_entry())
        .args([canoa_ffi::jobs::CHILD_FLAG, "peers"])
        .output()
        .expect("it runs");
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");

    let wrong = Command::new(child_entry())
        .args([
            canoa_ffi::jobs::CHILD_FLAG,
            "tool",
            "--name",
            "t",
            "--spec",
            "{}",
        ])
        .output()
        .expect("it runs");
    assert_eq!(wrong.status.code(), Some(2), "{wrong:?}");

    let mut launch = Command::new(child_entry())
        .args([canoa_ffi::jobs::CHILD_FLAG, "launch", "--"])
        .arg(child_entry())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("it starts");
    launch
        .stdin
        .take()
        .expect("its input")
        .write_all(b"{\"canoa_host_launch\":1,\"env\":[[\"CANOA_FFI_TEST_VALUE\",\"given\"]]}\n")
        .expect("written");
    let ended = launch.wait_with_output().expect("it ends");
    assert_eq!(ended.status.code(), Some(7), "{ended:?}");
    assert_eq!(String::from_utf8_lossy(&ended.stdout).trim(), "given");
}

/// The list of files allowed to hold `unsafe`: the review surface stays as narrow as this says.
#[test]
fn unsafe_code_stays_in_the_files_named_here() {
    let allowed = ["exports.rs", "platform.rs"];
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&source).expect("the source") {
        let path = entry.expect("an entry").path();
        let text = std::fs::read_to_string(&path).expect("read");
        let name = path.file_name().expect("a name").to_string_lossy();
        // A word in a comment or an attribute is not code: what counts is the keyword opening a block, a function or an
        // attribute's argument.
        let holds = [
            "unsafe {",
            "unsafe extern",
            "unsafe(no_mangle)",
            "unsafe(link_section",
        ]
        .iter()
        .any(|form| text.contains(form));
        assert_eq!(holds, allowed.contains(&&*name), "{name}");
    }
}

#[test]
fn accounts_are_listed_and_a_sign_in_that_needs_a_typed_secret_is_refused_as_a_job() {
    let scratch = Scratch::new();
    let (config, home) = (scratch.0.join("canoa.cfg"), scratch.0.join("home"));
    std::fs::write(
        &config,
        "[host]\naccount_store = \"file\"\n[host.agents.c]\nharness = \"claude\"\nexecutable = \"no-such-claude-for-the-test\"\n[host.accounts.work]\nharness = \"claude\"\nkind = \"harness-login\"\nid = \"0b7d\"\n",
    )
    .expect("written");
    let listed = to_end(start(&json!({
        "args": ["accounts", "list", "--format", "json", "--config", config],
        "home": home,
    })));
    assert_eq!(listed.exit, Some(0), "{listed:?}");
    let account: Value = serde_json::from_str(&listed.out[0]).expect("a JSON line");
    assert_eq!(
        (account["name"].as_str(), account["id"].as_str()),
        (Some("work"), Some("0b7d"))
    );

    // The application has no way to hand a secret in sealed yet, so a method that asks for one is refused, and no
    // credential is taken from anywhere else.
    let added = to_end(start(&json!({
        "args": ["accounts", "add", "second", "--harness", "claude", "--method", "api-key", "--format", "json",
            "--until-stdin-closes", "--config", config],
        "home": home,
    })));
    assert_ne!(added.exit, Some(0), "{added:?}");
    assert!(
        std::fs::read_to_string(&config)
            .expect("canoa.cfg")
            .contains("[host.accounts.work]")
            && !std::fs::read_to_string(&config)
                .expect("canoa.cfg")
                .contains("second"),
        "no account was added"
    );
}
fn open_link(job: u64, entry: &str) -> Result<u64, i32> {
    let mut link = u64::MAX;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_link_open(job, entry.as_ptr(), entry.len(), &raw mut link) };
    if status == STATUS_OK {
        assert_ne!(link, 0);
        Ok(link)
    } else {
        assert_eq!(link, 0, "written first");
        Err(status)
    }
}

fn send_on(link: u64, message: &Value) -> i32 {
    let text = message.to_string();
    // SAFETY: the text lives across the call.
    unsafe { canoa_ffi_link_send(link, text.as_ptr(), text.len()) }
}

fn read_link(link: u64) -> Value {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_link_read(link, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    value
}

/// The first message on `link` answering request `id`, parsed.
fn answer_on(link: u64, id: u64) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let read = read_link(link);
        for message in read["messages"].as_array().expect("a list") {
            let message: Value =
                serde_json::from_str(message.as_str().expect("text")).expect("ACP");
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
        assert!(Instant::now() < deadline, "no answer to {id}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stdio_agent() -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile
        .join("examples")
        .join(format!("stdio_agent{}", std::env::consts::EXE_SUFFIX))
}

/// Whether process `pid` is running, asked of the system's own tools.
fn process_runs(pid: u64) -> bool {
    if cfg!(windows) {
        let listed = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&listed.stdout).contains(&pid.to_string())
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[test]
fn a_link_refuses_what_it_cannot_open_and_says_which() {
    let mut link = 9_u64;
    // SAFETY: a null entry is what is being refused; `link` lives across the call.
    assert_eq!(
        unsafe { canoa_ffi_link_open(1, core::ptr::null(), 0, &raw mut link) },
        STATUS_BAD_ARGUMENT
    );
    assert_eq!(link, 0, "written first");
    // SAFETY: a null out-parameter is what is being refused.
    assert_eq!(
        unsafe { canoa_ffi_link_open(1, b"x".as_ptr(), 1, core::ptr::null_mut()) },
        STATUS_BAD_ARGUMENT
    );
    assert_eq!(open_link(u64::MAX, "x"), Err(STATUS_UNKNOWN_JOB));
    let bad = [0xff_u8, 0xfe];
    // SAFETY: both point at what lives across the call; text that is not UTF-8 is what is being refused.
    assert_eq!(
        unsafe { canoa_ffi_link_open(1, bad.as_ptr(), bad.len(), &raw mut link) },
        STATUS_BAD_ARGUMENT
    );
    assert_eq!(canoa_ffi_link_close(u64::MAX), STATUS_UNKNOWN_LINK);
    assert_eq!(send_on(u64::MAX, &json!({})), STATUS_UNKNOWN_LINK);
    // SAFETY: a null message is what is being refused.
    assert_eq!(
        unsafe { canoa_ffi_link_send(1, core::ptr::null(), 0) },
        STATUS_BAD_ARGUMENT
    );
    let (mut text, mut len) = (core::ptr::null_mut(), 9_usize);
    // SAFETY: both point at locals that live across the call.
    let status = unsafe { canoa_ffi_link_read(u64::MAX, &raw mut text, &raw mut len) };
    assert_eq!(status, STATUS_UNKNOWN_LINK);
    assert!(text.is_null());
    assert_eq!(len, 0);

    // A host that serves nothing has no entry to open.
    let scratch = Scratch::new();
    let job = serve(&scratch.config(), &scratch.0.join("home"), "a test");
    until(job, "the host to be ready", |out, _, _| {
        out.iter().any(|line| line.contains("ready"))
    });
    assert_eq!(open_link(job, "none"), Err(STATUS_NO_ENTRY));
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    assert_eq!(to_end(job).exit, Some(0));
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}

#[test]
fn a_link_runs_a_session_on_an_entry_with_no_proof_and_closing_it_ends_the_agent() {
    let agent = stdio_agent();
    assert!(
        agent.exists(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    let scratch = Scratch::new();
    let config = scratch.0.join("canoa.cfg");
    let quoted = serde_json::to_string(&agent.to_string_lossy()).expect("quoted");
    std::fs::write(
        &config,
        format!("[host.agents.fixture]\nprogram = {quoted}\nlisten = \"127.0.0.1:0\"\n"),
    )
    .expect("written");
    let job = serve(&config, &scratch.0.join("home"), "a test");
    until(job, "the host to be ready", |out, _, _| {
        out.iter().any(|line| line.contains("ready"))
    });
    let link = open_link(job, "fixture").expect("a link");

    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}}});
    assert_eq!(send_on(link, &initialize), STATUS_OK);
    assert!(answer_on(link, 1).get("result").is_some());
    let dir = scratch.0.to_string_lossy().into_owned();
    let new = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": dir, "mcpServers": []}});
    assert_eq!(send_on(link, &new), STATUS_OK);
    let session = answer_on(link, 2);
    assert!(session["result"]["sessionId"].is_string(), "{session}");
    let ask = json!({"jsonrpc": "2.0", "id": 3, "method": "_fixture/pid", "params": {}});
    assert_eq!(send_on(link, &ask), STATUS_OK);
    let pid = answer_on(link, 3)["result"]["pid"].as_u64().expect("a pid");

    assert_eq!(canoa_ffi_link_close(link), STATUS_OK);
    assert_eq!(canoa_ffi_link_close(link), STATUS_UNKNOWN_LINK);
    assert_eq!(send_on(link, &json!({})), STATUS_UNKNOWN_LINK);
    let deadline = Instant::now() + WAIT;
    while process_runs(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!process_runs(pid), "the agent outlived its link");

    // Stopping the host ends a link still open: it reads closed, and a send is told it ended.
    let second = open_link(job, "fixture").expect("another link");
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    assert_eq!(to_end(job).exit, Some(0));
    let deadline = Instant::now() + WAIT;
    while !read_link(second)["closed"].as_bool().unwrap_or(false) {
        assert!(Instant::now() < deadline, "the link never read closed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(send_on(second, &json!({})), STATUS_ENDED);
    assert_eq!(canoa_ffi_link_close(second), STATUS_OK);
    assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
}
