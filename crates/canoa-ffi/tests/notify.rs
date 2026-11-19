//! A link that was given a Dart port posts to it when there is something to read, so the application need not poll. The table of
//! Dart's native API is a stand-in here that records each post.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use core::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use canoa_ffi::exports::{
    STATUS_BAD_ARGUMENT, STATUS_OK, canoa_ffi_dart_api, canoa_ffi_dart_forget, canoa_ffi_job_read,
    canoa_ffi_job_start, canoa_ffi_job_stop, canoa_ffi_link_close, canoa_ffi_link_open,
    canoa_ffi_link_read, canoa_ffi_link_send, canoa_ffi_text_free,
};
use serde_json::{Value, json};

mod common;

const WAIT: Duration = Duration::from_secs(30);
const PORT: i64 = 4242;

#[repr(C)]
struct Api {
    major: i32,
    minor: i32,
    functions: *const Entry,
}

#[repr(C)]
struct Entry {
    name: *const c_char,
    function: *const c_void,
}

/// Every `(port, integer)` posted, and a signal that one came.
static POSTED: Mutex<Vec<(i64, i64)>> = Mutex::new(Vec::new());
static POSTS: Condvar = Condvar::new();

unsafe extern "C" fn fake_post(port: i64, value: i64) -> bool {
    POSTED.lock().expect("a lock").push((port, value));
    POSTS.notify_all();
    true
}

/// A table as the virtual machine hands it over, never freed.
fn table(major: i32, with_post: bool) -> *const c_void {
    let other = Entry {
        name: c"Dart_NewNativePort".as_ptr(),
        function: core::ptr::dangling(),
    };
    let post = Entry {
        name: c"Dart_PostInteger".as_ptr(),
        function: fake_post as *const c_void,
    };
    let end = Entry {
        name: core::ptr::null(),
        function: core::ptr::null(),
    };
    let entries: Vec<Entry> = if with_post {
        vec![other, post, end]
    } else {
        vec![other, end]
    };
    let entries = Box::leak(entries.into_boxed_slice());
    let api = Box::new(Api {
        major,
        minor: 0,
        functions: entries.as_ptr(),
    });
    core::ptr::from_ref(Box::leak(api)).cast()
}

/// Waits for the next post to `PORT` past the `seen` ones, and answers its value.
fn next_post(seen: usize) -> i64 {
    let until = Instant::now() + WAIT;
    let mut posted = POSTED.lock().expect("a lock");
    loop {
        if let Some((port, value)) = posted.get(seen) {
            assert_eq!(*port, PORT);
            return *value;
        }
        let left = until.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "nothing was posted");
        posted = POSTS.wait_timeout(posted, left).expect("a lock").0;
    }
}

fn example(name: &str) -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    profile
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn text_of(call: impl FnOnce(&mut *mut u8, &mut usize) -> i32) -> Value {
    let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
    assert_eq!(call(&mut text, &mut len), STATUS_OK);
    // SAFETY: the library just gave this text of this length.
    let value = serde_json::from_slice(unsafe { core::slice::from_raw_parts(text, len) })
        .expect("the text is JSON");
    // SAFETY: as it was given, once.
    unsafe { canoa_ffi_text_free(text, len) };
    value
}

fn send_on(link: u64, message: &Value) {
    let text = message.to_string();
    // SAFETY: the text lives across the call.
    let status = unsafe { canoa_ffi_link_send(link, text.as_ptr(), text.len()) };
    assert_eq!(status, STATUS_OK);
}

fn read_link(link: u64) -> Value {
    // SAFETY: both point at locals that live across the call.
    text_of(|text, len| unsafe { canoa_ffi_link_read(link, text, len) })
}

#[test]
fn a_link_with_a_port_posts_its_number_when_it_has_something_to_read_and_when_it_ends() {
    common::memory_credentials();
    // Before a table, and with tables that are not Dart's, no link may name a port.
    let (agent, entry) = (example("stdio_agent"), example("child_entry"));
    assert!(
        agent.exists() && entry.exists(),
        "build the examples first: cargo build -p canoa-host -p canoa-ffi --examples"
    );
    let me = std::process::id();
    let scratch = std::env::temp_dir().join(format!("canoa-notify-{me}"));
    std::fs::create_dir_all(&scratch).expect("a directory");
    let config = scratch.join("canoa.cfg");
    std::fs::write(
        &config,
        format!(
            "[host.agents.fixture]\nprogram = {}\nlisten = \"127.0.0.1:0\"\n",
            serde_json::to_string(&agent.to_string_lossy()).expect("quoted")
        ),
    )
    .expect("written");
    let request = json!({
        "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", config],
        "home": scratch.join("home"),
        "holder": "a test",
        "program": entry,
        "program_args": [canoa_ffi::jobs::CHILD_FLAG],
    })
    .to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK);
    let until = Instant::now() + WAIT;
    loop {
        // SAFETY: both point at locals that live across the call.
        let said = text_of(|text, len| unsafe { canoa_ffi_job_read(job, text, len) });
        if said["out"].to_string().contains("ready") {
            break;
        }
        assert!(Instant::now() < until, "the host never was ready: {said}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let name = "fixture";
    let mut link = 7_u64;
    // SAFETY: both point at what lives across the call.
    let status =
        unsafe { canoa_ffi_link_open(job, name.as_ptr(), name.len(), PORT, &raw mut link) };
    assert_eq!(status, STATUS_BAD_ARGUMENT, "no table yet");
    assert_eq!(link, 0, "written first");

    // SAFETY: null is what is refused; each table lives for the whole test.
    unsafe {
        assert_eq!(canoa_ffi_dart_api(core::ptr::null()), STATUS_BAD_ARGUMENT);
        assert_eq!(canoa_ffi_dart_api(table(1, true)), STATUS_BAD_ARGUMENT);
        assert_eq!(canoa_ffi_dart_api(table(2, false)), STATUS_BAD_ARGUMENT);
    }
    // SAFETY: as above.
    let status =
        unsafe { canoa_ffi_link_open(job, name.as_ptr(), name.len(), PORT, &raw mut link) };
    assert_eq!(status, STATUS_BAD_ARGUMENT, "still no usable table");

    // SAFETY: the table lives for the whole test.
    unsafe {
        assert_eq!(canoa_ffi_dart_api(table(2, true)), STATUS_OK);
        assert_eq!(canoa_ffi_dart_api(table(2, true)), STATUS_OK, "again");
    }
    // SAFETY: both point at what lives across the call.
    let status =
        unsafe { canoa_ffi_link_open(job, name.as_ptr(), name.len(), PORT, &raw mut link) };
    assert_eq!(status, STATUS_OK);
    assert_ne!(link, 0);
    let number = i64::try_from(link).expect("a small number");

    // Each answer wakes the port, with no read in between to fetch it.
    for id in 1..=3_u64 {
        let seen = POSTED.lock().expect("a lock").len();
        send_on(
            link,
            &json!({"jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {}}}),
        );
        let mut posts = seen;
        loop {
            assert_eq!(next_post(posts), number);
            posts += 1;
            let read = read_link(link);
            assert_eq!(read["closed"], false);
            let answered = read["messages"]
                .as_array()
                .expect("a list")
                .iter()
                .any(|message| {
                    message
                        .as_str()
                        .is_some_and(|text| text.contains(&format!("\"id\":{id}")))
                });
            if answered {
                break;
            }
        }
    }

    // The host stopping ends the link, which is told too.
    let seen = POSTED.lock().expect("a lock").len();
    assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
    assert_eq!(next_post(seen), number);
    let until = Instant::now() + WAIT;
    while read_link(link)["closed"] != true {
        assert!(Instant::now() < until, "the link never read as closed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(canoa_ffi_link_close(link), STATUS_OK);

    // Once every table is given up, no link may name a port, and nothing more is posted.
    canoa_ffi_dart_forget(core::ptr::null_mut());
    canoa_ffi_dart_forget(core::ptr::null_mut());
    let posted = POSTED.lock().expect("a lock").len();
    let mut other = 7_u64;
    // SAFETY: both point at what lives across the call.
    let status =
        unsafe { canoa_ffi_link_open(job, name.as_ptr(), name.len(), PORT, &raw mut other) };
    assert_eq!(status, STATUS_BAD_ARGUMENT, "disarmed");
    assert_eq!(POSTED.lock().expect("a lock").len(), posted);
    let _ = std::fs::remove_dir_all(&scratch);
}
