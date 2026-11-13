//! The stdio bridge's checks that open no socket: what it refuses to start on and what it answers by itself. What it relays to a
//! server is checked from a process of its own, since an agent's usage reading looks at the sockets of the process it runs in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and the clock are std's"
)]

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::delegate::bridge::{Refused, Target, serve};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// An output the test can read while the bridge holds it.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("not poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("not poisoned").clone()).expect("UTF-8")
    }

    /// At least `count` lines, waiting for them; fewer if they do not come.
    fn lines(&self, count: usize) -> Vec<Value> {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let lines: Vec<Value> = self
                .text()
                .lines()
                .map(|line| serde_json::from_str(line).expect("a JSON line"))
                .collect();
            if lines.len() >= count || Instant::now() > until {
                return lines;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn target(url: &str) -> Target {
    Target::new(Some(url), Some(TOKEN)).expect("a loopback address")
}

fn refusal(url: Option<&str>, token: Option<&str>) -> Option<Refused> {
    Target::new(url, token).err()
}

#[test]
fn it_refuses_a_missing_address_a_missing_token_and_any_address_that_is_not_loopback() {
    let good = Some("http://127.0.0.1:9/mcp");
    assert_eq!(refusal(None, Some(TOKEN)), Some(Refused::NoUrl));
    assert_eq!(refusal(Some(""), Some(TOKEN)), Some(Refused::NoUrl));
    assert_eq!(refusal(good, None), Some(Refused::NoToken));
    assert_eq!(refusal(good, Some("")), Some(Refused::NoToken));
    assert_eq!(refusal(good, Some("a b")), Some(Refused::BadToken));
    assert_eq!(refusal(good, Some("a\r\nHost: x")), Some(Refused::BadToken));
    for url in [
        "http://10.0.0.5:9/mcp",
        "http://192.168.1.2:9/mcp",
        "http://example.com:9/mcp",
        "http://localhost.example.com:9/mcp",
        "http://127.0.0.1.example.com:9/mcp",
        "http://[2001:db8::1]:9/mcp",
        "http://0.0.0.0:9/mcp",
        "http://[::ffff:127.0.0.1]:9/mcp",
    ] {
        assert_eq!(
            refusal(Some(url), Some(TOKEN)),
            Some(Refused::NotLoopback),
            "{url}"
        );
    }
    for url in [
        "https://127.0.0.1:9/mcp",
        "ftp://127.0.0.1:9/mcp",
        "127.0.0.1:9/mcp",
        "http://user@127.0.0.1:9/mcp",
        "http://127.0.0.1:99999/mcp",
        "http://127.0.0.1:9/m cp",
    ] {
        assert_eq!(
            refusal(Some(url), Some(TOKEN)),
            Some(Refused::BadUrl),
            "{url}"
        );
    }
    for url in [
        "http://127.0.0.1:9/mcp",
        "http://127.3.4.5:9/mcp",
        "http://[::1]:9/mcp",
        "http://localhost:9/mcp",
        "HTTP://127.0.0.1:9/mcp",
    ] {
        assert!(Target::new(Some(url), Some(TOKEN)).is_ok(), "{url}");
    }
    // Nothing a refusal or a debug print says holds the bearer.
    assert!(!format!("{:?}", target("http://127.0.0.1:9/mcp")).contains(TOKEN));
    for refused in [
        Refused::NoUrl,
        Refused::NoToken,
        Refused::BadToken,
        Refused::BadUrl,
        Refused::NotLoopback,
    ] {
        assert!(!refused.to_string().contains(TOKEN));
    }
}

#[test]
fn a_line_that_is_not_json_is_answered_with_a_parse_error_and_the_bridge_goes_on() {
    let sink = Sink::default();
    let input = "not json\n\n";
    serve(
        target("http://127.0.0.1:9/mcp"),
        Cursor::new(input),
        sink.clone(),
    )
    .expect("served");
    let lines = sink.lines(1);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0]["id"].is_null());
    assert_eq!(lines[0]["error"]["code"], -32700);
}

#[test]
fn it_ends_when_its_input_does_even_with_nothing_said() {
    let started = Instant::now();
    serve(
        target("http://127.0.0.1:9/mcp"),
        Cursor::new(Vec::new()),
        Sink::default(),
    )
    .expect("served");
    assert!(started.elapsed() < Duration::from_secs(5));
}
