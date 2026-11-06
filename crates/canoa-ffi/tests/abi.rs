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
    ABI_VERSION, STATUS_BAD_ARGUMENT, STATUS_OK, STATUS_UNKNOWN_JOB, canoa_ffi_abi,
    canoa_ffi_job_free, canoa_ffi_job_read, canoa_ffi_job_start, canoa_ffi_job_stop,
    canoa_ffi_text_free,
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

/// `provider`, `tool` and `launch` change the process they run in, its directory, its standard input and its end, so in an
/// application's process they are refused: they run only as a process of their own.
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
