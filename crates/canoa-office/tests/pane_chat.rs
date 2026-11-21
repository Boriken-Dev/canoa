//! The pane end to end, without Office: a control activated in a window of the test's own finds a stand-in host through a door in a
//! scratch home (no address anywhere), starts the real WebView2 control and asks a loopback web server of the test for the chat
//! of the open document, then follows the document when it changes. With no host it says so in its window.

#![cfg(windows)]
#![allow(unsafe_code, non_snake_case)]

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_office::context::Context;
use canoa_office::host::{door_name, pipe_path};
use canoa_office::relation::{chat_url, document_relation};
use canoa_office::surface::{pane_state, set_next_context};
use canoa_office::{PaneControl, host};
use serde_json::json;
use windows::Win32::Foundation::{E_NOTIMPL, HANDLE, HWND, RECT};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IMoniker};
use windows::Win32::System::Ole::{
    IOleClientSite, IOleClientSite_Impl, IOleContainer, IOleObject, OLEGETMONIKER,
    OLEIVERB_INPLACEACTIVATE, OLEWHICHMK,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
    TranslateMessage, WINDOW_EX_STYLE, WS_POPUP,
};
use windows::core::{BOOL, HSTRING, Interface, Result, implement, w};

#[implement(IOleClientSite)]
struct Site;

impl IOleClientSite_Impl for Site_Impl {
    fn SaveObject(&self) -> Result<()> {
        Ok(())
    }
    fn GetMoniker(&self, _: &OLEGETMONIKER, _: &OLEWHICHMK) -> Result<IMoniker> {
        Err(E_NOTIMPL.into())
    }
    fn GetContainer(&self) -> Result<IOleContainer> {
        Err(E_NOTIMPL.into())
    }
    fn ShowObject(&self) -> Result<()> {
        Ok(())
    }
    fn OnShowWindow(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn RequestNewObjectLayout(&self) -> Result<()> {
        Ok(())
    }
}

fn pump(wait: Duration, done: impl Fn() -> bool) -> bool {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if done() {
            return true;
        }
        let mut message = MSG::default();
        // SAFETY: the thread's own queue and a live local.
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

/// A door that answers `@hello` with `reply` to as many connections as come.
fn stand_in_door(home: &std::path::Path, reply: serde_json::Value) {
    let path = pipe_path(&door_name(home));
    // SAFETY: a pipe with a name of this test's own, which its thread owns.
    let handle = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(path),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            4,
            64 * 1024,
            64 * 1024,
            0,
            None,
        )
    };
    assert!(!handle.is_invalid());
    let raw = handle.0 as usize;
    std::thread::spawn(move || {
        let first = HANDLE(raw as *mut core::ffi::c_void);
        let mut handle = first;
        loop {
            // SAFETY: a pipe instance this thread owns.
            unsafe {
                let _ = ConnectNamedPipe(handle, None);
            }
            // SAFETY: the same instance, owned by the `File` until it is dropped.
            let mut file = unsafe { File::from_raw_handle(handle.0) };
            let mut length = [0_u8; 4];
            if file.read_exact(&mut length).is_ok() {
                let mut ask = vec![0; u32::from_le_bytes(length) as usize];
                let _ = file.read_exact(&mut ask);
                let body = reply.to_string();
                let mut frame = (body.len() as u32).to_le_bytes().to_vec();
                frame.extend_from_slice(body.as_bytes());
                let _ = file.write_all(&frame);
                let _ = file.sync_all();
            }
            // SAFETY: still this thread's instance.
            unsafe {
                let _ = DisconnectNamedPipe(HANDLE(
                    std::os::windows::io::AsRawHandle::as_raw_handle(&file),
                ));
            }
            // A fresh instance for the next connection.
            drop(file);
            // SAFETY: as above.
            handle = unsafe {
                CreateNamedPipeW(
                    &HSTRING::from(pipe_path(&door_name_of_env())),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    4,
                    64 * 1024,
                    64 * 1024,
                    0,
                    None,
                )
            };
            if handle.is_invalid() {
                return;
            }
        }
    });
}

fn door_name_of_env() -> String {
    door_name(&host::home_dir().expect("a home"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-office-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Serves a page on a loopback port and records each request line.
fn web_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let record = Arc::clone(&record);
            std::thread::spawn(move || {
                let mut buffer = [0_u8; 8192];
                let Ok(read) = stream.read(&mut buffer) else {
                    return;
                };
                let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if let Some(line) = text.lines().next() {
                    record.lock().unwrap().push(line.to_owned());
                }
                let body = "<!doctype html><title>chat</title>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    (port, seen)
}

fn parent_window() -> HWND {
    // SAFETY: the STATIC class is the system's; the caller destroys the window.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            500,
            600,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    }
}

fn activate(control: &IOleObject, parent: HWND) {
    let site: IOleClientSite = Site.into();
    let area = RECT {
        left: 0,
        top: 0,
        right: 420,
        bottom: 560,
    };
    // SAFETY: live interfaces and a rectangle that outlives the call.
    unsafe {
        control.SetClientSite(&site).unwrap();
        control
            .DoVerb(
                OLEIVERB_INPLACEACTIVATE.0,
                std::ptr::null(),
                &site,
                0,
                parent,
                &area,
            )
            .unwrap();
    }
}

#[test]
fn the_pane_finds_the_host_with_no_address_and_shows_the_chat_of_the_open_document_and_follows_it()
{
    // SAFETY: first COM call on this thread; the variables are set before any pane exists, and this is the only test that does.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let home = temp("home");
        std::fs::create_dir_all(home.join("office")).unwrap();
        std::fs::write(home.join("office").join("secret"), "s3cret\n").unwrap();
        std::env::set_var("COQUI_AGENTIC_PEER_HOME", &home);
        std::env::set_var("CANOA_OFFICE_DATA", temp("data"));
        std::env::set_var("CANOA_OFFICE_LOG", temp("log").join("log.txt"));
    }
    let (port, requests) = web_server();
    let origin = format!("http://localhost:{port}");
    stand_in_door(
        &host::home_dir().unwrap(),
        json!({ "name": "Canoa", "entries": [], "web": { "url": origin, "office": true, "office_local": true } }),
    );

    let first = document_relation("word", r"D:\Papers\Plan.docx").unwrap();
    set_next_context(
        7,
        Context {
            client: "word",
            relation: Some(first.clone()),
        },
    );
    let control: IOleObject = PaneControl::new().into();
    let parent = parent_window();
    activate(&control, parent);

    let wanted = chat_url(&origin, Some(&first), "word", Some("s3cret")).unwrap();
    let path = wanted.strip_prefix(&origin).unwrap().to_owned();
    let asked = |path: &str| {
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with(&format!("GET {path} ")))
    };
    assert!(
        pump(Duration::from_secs(60), || asked(&path)),
        "the chat of the document was never asked for; asked for: {:?}",
        requests.lock().unwrap()
    );

    // The person opens another document and shows the pane again: the chat follows it, in the same control.
    let second = document_relation("word", r"D:\Papers\Budget.docx").unwrap();
    pane_state(7).expect("the pane's state").refresh(Context {
        client: "word",
        relation: Some(second.clone()),
    });
    let next = chat_url(&origin, Some(&second), "word", Some("s3cret"))
        .unwrap()
        .strip_prefix(&origin)
        .unwrap()
        .to_owned();
    assert!(
        pump(Duration::from_secs(30), || asked(&next)),
        "asked for: {:?}",
        requests.lock().unwrap()
    );
    assert!(
        pane_state(7)
            .unwrap()
            .chat_address()
            .is_some_and(|address| address.contains("Budget"))
    );

    // Closing the control ends the browser.
    // SAFETY: live interfaces and the window made above.
    unsafe {
        control
            .cast::<windows::Win32::System::Ole::IOleInPlaceObject>()
            .unwrap()
            .InPlaceDeactivate()
            .unwrap();
        control.SetClientSite(None::<&IOleClientSite>).unwrap();
        let _ = DestroyWindow(parent);
    }
    assert!(pane_state(7).unwrap().chat_address().is_none());
}

#[test]
fn with_no_host_the_pane_says_why_in_its_window() {
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
    // The first test sets the home for the process; this one needs one with no door, so it reads the failure from the locator.
    set_next_context(
        8,
        Context {
            client: "excel",
            relation: None,
        },
    );
    canoa_office::surface::set_next_locator(|| Err(host::Failure::WebAppNotForOffice));
    let control: IOleObject = PaneControl::new().into();
    let parent = parent_window();
    activate(&control, parent);
    let state = pane_state(8).expect("the pane's state");
    assert!(
        pump(Duration::from_secs(20), || state
            .status()
            .contains("office = true")),
        "the window says: {}",
        state.status()
    );
    assert!(state.chat_address().is_none());
    // SAFETY: the window made above.
    unsafe {
        let _ = DestroyWindow(parent);
    }
}
