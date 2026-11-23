//! The WebView2 control, for real: made in a hidden window of the test's own from the runtime Windows ships, with its data in a
//! scratch folder, showing a page served by a loopback listener of the test. No Office application is involved.

#![cfg(windows)]
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_office::webview::{Browser, runtime_dir};
use windows::Win32::Foundation::RECT;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
    TranslateMessage, WINDOW_EX_STYLE, WS_POPUP,
};
use windows::core::w;

/// Runs the thread's message loop until `done` or `wait` is over.
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

#[test]
fn the_pane_shows_a_page_from_a_loopback_address_in_the_runtimes_control() {
    assert!(
        runtime_dir().is_some(),
        "the WebView2 runtime is installed on a machine this runs on"
    );
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };

    // A page of the test's own, and what asked for it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                let mut buffer = [0_u8; 4096];
                let Ok(read) = stream.read(&mut buffer) else {
                    return;
                };
                let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if let Some(line) = text.lines().next() {
                    seen.lock().unwrap().push(line.to_owned());
                }
                let body = "<!doctype html><title>canoa test page</title><p>chat</p>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });

    let data = std::env::temp_dir().join(format!("canoa-office-webview-{}", std::process::id()));
    // SAFETY: the STATIC class is the system's; the window is destroyed below.
    let parent = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            500,
            500,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let browser = Browser::new();
    let failure = Rc::new(RefCell::new(None::<String>));
    let why = Rc::clone(&failure);
    let url = format!("http://localhost:{port}/office/app/?mode=chat&client=word");
    browser.open(
        parent,
        &data,
        RECT {
            left: 0,
            top: 0,
            right: 400,
            bottom: 500,
        },
        &url,
        move |reason| *why.borrow_mut() = Some(reason),
        || {},
    );
    let loaded = pump(Duration::from_secs(60), || {
        failure.borrow().is_some()
            || requests
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("/office/app/"))
    });
    assert!(
        failure.borrow().is_none(),
        "the control was not made: {:?}",
        failure.borrow()
    );
    assert!(
        loaded,
        "the page was never asked for; asked for: {:?}",
        requests.lock().unwrap()
    );
    let line = requests
        .lock()
        .unwrap()
        .iter()
        .find(|line| line.contains("/office/app/"))
        .cloned()
        .unwrap();
    assert!(
        line.starts_with("GET /office/app/?mode=chat&client=word "),
        "{line}"
    );
    assert!(
        pump(Duration::from_secs(20), || browser
            .source()
            .is_some_and(|source| source.contains("/office/app/"))),
        "the control's source: {:?}",
        browser.source()
    );
    // Moving and hiding do not fail, and closing ends it.
    browser.resize(RECT {
        left: 0,
        top: 0,
        right: 300,
        bottom: 300,
    });
    browser.set_visible(false);
    browser.close();
    assert!(!browser.is_open());
    // SAFETY: the window made above.
    unsafe {
        let _ = DestroyWindow(parent);
    }
    pump(Duration::from_secs(3), || false);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_control_closed_before_it_exists_is_closed_when_it_arrives() {
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
    let data = std::env::temp_dir().join(format!(
        "canoa-office-webview-closed-{}",
        std::process::id()
    ));
    // SAFETY: the STATIC class is the system's; the window is destroyed below.
    let parent = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            500,
            500,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let browser = Browser::new();
    browser.open(
        parent,
        &data,
        RECT {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        },
        "about:blank",
        |_| {},
        || {},
    );
    browser.close();
    pump(Duration::from_secs(8), || false);
    assert!(
        !browser.is_open(),
        "a control that arrived after close is not kept"
    );
    // SAFETY: the window made above.
    unsafe {
        let _ = DestroyWindow(parent);
    }
    let _ = std::fs::remove_dir_all(&data);
}
