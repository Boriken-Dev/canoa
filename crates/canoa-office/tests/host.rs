//! Finding or starting the host: against a stand-in door on a named pipe of a scratch home. The stand-in speaks the door's framing
//! (a little-endian length, then JSON) and answers `@hello`; nothing here starts a real Canoa or reads the person's home.

#![cfg(windows)]
#![allow(unsafe_code)]

use std::cell::Cell;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use canoa_office::host::{
    Failure, Launch, Web, door_name, hello, home_dir_from, is_local, locate_with, pipe_path,
    secret, web_of,
};
use serde_json::{Value, json};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};
use windows::core::HSTRING;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A home folder of the test's own, removed when it goes.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("canoa-office-host-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A door that answers `@hello` with `reply` to one connection, then goes.
fn stand_in_door(home: &Path, reply: Value) -> JoinHandle<Value> {
    let path = pipe_path(&door_name(home));
    // SAFETY: a fresh pipe with a name of this test's own; the handle becomes a `File`, which closes it.
    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(path),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            64 * 1024,
            64 * 1024,
            0,
            None,
        )
    };
    assert!(!handle.is_invalid(), "the stand-in pipe is created");
    let raw = handle.0 as usize;
    std::thread::spawn(move || {
        let handle = HANDLE(raw as *mut core::ffi::c_void);
        // SAFETY: the handle is the pipe made above, used by this thread alone.
        unsafe {
            let _ = ConnectNamedPipe(handle, None);
        }
        // SAFETY: as above; the `File` owns it from here.
        let mut file = unsafe { File::from_raw_handle(handle.0) };
        let mut length = [0_u8; 4];
        file.read_exact(&mut length).unwrap();
        let mut ask = vec![0; u32::from_le_bytes(length) as usize];
        file.read_exact(&mut ask).unwrap();
        let ask: Value = serde_json::from_slice(&ask).unwrap();
        let body = reply.to_string();
        let mut frame = (body.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(body.as_bytes());
        file.write_all(&frame).unwrap();
        // Waits until the client has read it: a pipe disconnected first loses what it holds.
        file.sync_all().unwrap();
        // SAFETY: the pipe is still this thread's.
        unsafe {
            let _ = DisconnectNamedPipe(HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(
                &file,
            )));
        }
        ask
    })
}

fn hello_with(web: Value) -> Value {
    let mut hello = json!({ "name": "Canoa", "version": "0", "peer": "p", "method": "noise", "entries": [], "control": true });
    if !web.is_null() {
        hello["web"] = web;
    }
    hello
}

fn never_start(_: &Launch) -> std::io::Result<u32> {
    panic!("a host that answers is not started");
}

#[test]
fn the_door_name_is_the_one_the_host_makes() {
    // The same vector is in canoa-host's tests: the name is made there and read here, so the two must agree.
    assert_eq!(
        door_name(Path::new(r"C:\canoa-test-home\x")),
        "canoa-native-ce53970373d8428b"
    );
    // Case does not matter on this system, as the host lower-cases the path.
    assert_eq!(
        door_name(Path::new(r"C:\Canoa-Test-Home\X")),
        "canoa-native-ce53970373d8428b"
    );
    assert_eq!(pipe_path("canoa-native-ab"), r"\\.\pipe\canoa-native-ab");
}

#[test]
fn the_home_is_the_variable_else_dot_canoa_in_the_users_folder() {
    let user = Some(PathBuf::from(r"D:\People\someone"));
    assert_eq!(
        home_dir_from(Some(PathBuf::from(r"D:\homes\x")), user.clone()),
        Some(PathBuf::from(r"D:\homes\x"))
    );
    assert_eq!(
        home_dir_from(None, user),
        Some(PathBuf::from(r"D:\People\someone\.canoa"))
    );
    assert_eq!(home_dir_from(None, None), None);
}

#[test]
fn a_running_host_is_found_through_its_door_with_no_address_given_and_its_secret_is_read() {
    let home = Home::new();
    std::fs::create_dir_all(home.0.join("office")).unwrap();
    std::fs::write(home.0.join("office").join("secret"), "abc123\n").unwrap();
    let door = stand_in_door(
        &home.0,
        hello_with(
            json!({ "url": "https://localhost:7800", "office": true, "office_local": true }),
        ),
    );
    let found = locate_with(&home.0, Duration::from_secs(1), &never_start).expect("found");
    assert_eq!(
        found.web,
        Web {
            url: "https://localhost:7800".to_owned(),
            office: true,
            office_local: true
        }
    );
    assert_eq!(found.secret.as_deref(), Some("abc123"));
    let ask = door.join().unwrap();
    assert_eq!(ask["open"], "@hello", "the add-in only asks hello: {ask}");
    assert_eq!(secret(&home.0).as_deref(), Some("abc123"));
}

#[test]
fn with_no_host_the_recorded_program_is_started_and_the_door_waited_for() {
    let home = Home::new();
    std::fs::create_dir_all(home.0.join("native")).unwrap();
    std::fs::write(
        home.0.join("native").join("launch.json"),
        json!({ "program": r"C:\somewhere\canoa.exe", "args": ["serve", "--until-stdin-closes"] })
            .to_string(),
    )
    .unwrap();
    let started = Cell::new(0);
    let door: Cell<Option<JoinHandle<Value>>> = Cell::new(None);
    let starter = |launch: &Launch| {
        started.set(started.get() + 1);
        assert_eq!(launch.program, Path::new(r"C:\somewhere\canoa.exe"));
        assert_eq!(launch.args, ["serve", "--until-stdin-closes"]);
        // The host takes a moment to open its door.
        let path = home.0.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            stand_in_door(
                &path,
                hello_with(
                    json!({ "url": "http://localhost:8123", "office": true, "office_local": true }),
                ),
            )
            .join()
            .unwrap()
        });
        door.set(Some(handle));
        Ok(4242)
    };
    let found =
        locate_with(&home.0, Duration::from_secs(10), &starter).expect("found after the start");
    assert_eq!(started.get(), 1, "started once");
    assert_eq!(found.web.url, "http://localhost:8123");
    assert_eq!(found.secret, None, "no secret file, no secret");
    door.take().unwrap().join().unwrap();
}

#[test]
fn with_no_host_and_no_recorded_program_it_says_so_and_starts_nothing() {
    let home = Home::new();
    assert_eq!(
        locate_with(&home.0, Duration::from_millis(100), &never_start),
        Err(Failure::NotRunning)
    );
}

#[test]
fn a_host_that_cannot_be_started_is_said_not_to_have_started() {
    let home = Home::new();
    std::fs::create_dir_all(home.0.join("native")).unwrap();
    std::fs::write(
        home.0.join("native").join("launch.json"),
        json!({ "program": "x" }).to_string(),
    )
    .unwrap();
    let refused = |_: &Launch| Err(std::io::Error::other("no such program"));
    match locate_with(&home.0, Duration::from_millis(100), &refused) {
        Err(Failure::NotStarted(why)) => assert!(why.contains("no such program"), "{why}"),
        other => panic!("{other:?}"),
    }
    // One that starts and never opens a door.
    let silent = |_: &Launch| Ok(1);
    match locate_with(&home.0, Duration::from_millis(600), &silent) {
        Err(Failure::NotStarted(why)) => assert!(why.contains("did not open its door"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn what_the_door_says_of_the_web_app_decides_whether_the_pane_can_show_it() {
    let ok = hello_with(
        json!({ "url": "https://localhost:7800", "office": true, "office_local": true }),
    );
    assert!(web_of(&ok).is_ok());
    assert_eq!(web_of(&hello_with(Value::Null)), Err(Failure::NoWebApp));
    for web in [
        json!({ "url": "https://localhost:7800", "office": false, "office_local": true }),
        json!({ "url": "https://localhost:7800", "office": true, "office_local": false }),
        json!({ "url": "https://localhost:7800" }),
    ] {
        assert_eq!(web_of(&hello_with(web)), Err(Failure::WebAppNotForOffice));
    }
    // A host's claim of an address that is not this computer's is not opened.
    for url in [
        "https://example.com",
        "https://192.168.1.5:7800",
        "ftp://localhost",
        "javascript:alert(1)",
        "not a url",
    ] {
        let web = json!({ "url": url, "office": true, "office_local": true });
        assert!(
            matches!(web_of(&hello_with(web)), Err(Failure::NotLocal(_))),
            "{url}"
        );
    }
    assert!(matches!(
        web_of(&hello_with(json!({ "url": 7 }))),
        Err(Failure::Unreadable(_))
    ));
    for local in [
        "http://127.0.0.1:7800",
        "https://localhost",
        "https://LOCALHOST:1",
        "http://[::1]:80",
    ] {
        assert!(is_local(local), "{local}");
    }
}

#[test]
fn a_pipe_that_is_not_there_is_not_found() {
    let home = Home::new();
    let error = hello(&pipe_path(&door_name(&home.0))).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn the_recorded_program_is_started_detached_with_its_arguments() {
    let home = Home::new();
    let marker = home.0.join("started.txt");
    let shell = std::env::var("COMSPEC").expect("a shell");
    let launch = Launch {
        program: PathBuf::from(shell),
        args: vec![
            "/c".to_owned(),
            "echo".to_owned(),
            "started".to_owned(),
            ">".to_owned(),
            marker.display().to_string(),
        ],
    };
    let id = launch.start().expect("it starts");
    assert!(id > 0);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists(), "the program ran");
    assert_eq!(Launch::read(&home.0), None, "no record, no launch");
    std::fs::create_dir_all(home.0.join("native")).unwrap();
    std::fs::write(home.0.join("native").join("launch.json"), "not json").unwrap();
    assert_eq!(Launch::read(&home.0), None);
}
