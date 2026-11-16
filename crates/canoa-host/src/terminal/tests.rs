use alloc::sync::Arc;
use core::sync::atomic::AtomicBool;
use core::time::Duration;
use std::time::Instant;

use super::*;
use crate::control::{Caller, Control, Grant, MAX_REQUEST, serve};

const WAIT: Duration = Duration::from_secs(20);

/// The shell the tests start: one every machine has.
fn shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned())
    } else {
        "/bin/sh".to_owned()
    }
}

fn terminals() -> Terminals {
    Terminals::new(Some(shell()), Some(std::env::temp_dir()))
}

fn control(terminals: Terminals) -> Control {
    Control::open(None, 100, 4096, Vec::new())
        .expect("opens")
        .with_terminals(terminals)
}

fn peer(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

/// The page's end of a link: what it asked, the answers, and the shell's output as it arrived.
struct Client<'a> {
    link: &'a crate::local::Link,
    next: u64,
    output: Vec<u8>,
    exits: Vec<(u64, i64)>,
}

impl<'a> Client<'a> {
    fn new(link: &'a crate::local::Link) -> Self {
        Self {
            link,
            next: 1,
            output: Vec::new(),
            exits: Vec::new(),
        }
    }

    fn take(&mut self, message: &[u8]) -> Option<Value> {
        let message: Value = serde_json::from_slice(message).expect("JSON");
        match message.get("method").and_then(Value::as_str) {
            Some("terminal/output") => {
                let data = message["params"]["data"].as_str().expect("data");
                self.output
                    .extend(BASE64.decode(data.as_bytes()).expect("base64"));
                None
            }
            Some("terminal/exit") => {
                self.exits.push((
                    message["params"]["id"].as_u64().expect("id"),
                    message["params"]["code"].as_i64().expect("code"),
                ));
                None
            }
            _ => Some(message),
        }
    }

    fn ask(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.link
            .send(
                &serde_json::to_vec(
                    &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
                )
                .expect("encodes"),
            )
            .expect("sent");
        let until = Instant::now() + WAIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let message = self
                .link
                .receive(Some(left))
                .expect("the host answered in time")
                .expect("a message");
            if let Some(answer) = self.take(&message)
                && answer["id"] == id
            {
                return answer;
            }
        }
    }

    fn open(&mut self, params: &Value) -> u64 {
        let answer = self.ask("terminal/open", params);
        answer["result"]["id"]
            .as_u64()
            .unwrap_or_else(|| panic!("a terminal opened: {answer}"))
    }

    fn type_in(&mut self, id: u64, text: &str) -> Value {
        self.ask(
            "terminal/write",
            &json!({"id": id, "data": BASE64.encode(text.as_bytes())}),
        )
    }

    /// Reads until `wanted` has come out of the shell.
    fn until_output(&mut self, wanted: &str) {
        let until = Instant::now() + WAIT;
        while !String::from_utf8_lossy(&self.output).contains(wanted) {
            let left = until.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "{wanted:?} never came out of the shell; it printed {:?}",
                String::from_utf8_lossy(&self.output)
            );
            if let Ok(Some(message)) = self.link.receive(Some(left)) {
                let _ = self.take(&message);
            }
        }
    }

    fn until_exit(&mut self) -> (u64, i64) {
        let until = Instant::now() + WAIT;
        while self.exits.is_empty() {
            let left = until.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the shell never reported its end");
            if let Ok(Some(message)) = self.link.receive(Some(left)) {
                let _ = self.take(&message);
            }
        }
        self.exits[0]
    }
}

/// Serves `caller` on a link and hands the page's end to `page`; the link is closed after, and what `serve` made on it has
/// ended by the time this returns.
fn on_a_link<R>(
    control: &Control,
    caller: &Caller,
    page: impl FnOnce(&mut Client<'_>) -> R,
) -> (R, Duration) {
    let stopped = Arc::new(AtomicBool::new(false));
    let (client, host) = crate::local::pair(MAX_REQUEST, stopped);
    std::thread::scope(|scope| {
        let served = scope.spawn(|| serve(control, caller, &host));
        // A page that panics still closes the link, or the scope would wait for it for ever.
        struct Close<'a>(&'a crate::local::Link);
        impl Drop for Close<'_> {
            fn drop(&mut self) {
                self.0.close();
            }
        }
        let closing = Close(&client);
        let result = page(&mut Client::new(&client));
        let closed = Instant::now();
        drop(closing);
        served.join().expect("the link ended");
        (result, closed.elapsed())
    })
}

fn newline() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

/// A command that prints `canoa_<word>` and whose own text, as the terminal echoes it, does not contain that.
fn say(word: &str) -> String {
    if cfg!(windows) {
        format!("echo canoa^_{word}{}", newline())
    } else {
        format!("echo canoa\\_{word}{}", newline())
    }
}

#[test]
fn what_is_typed_reaches_the_shell_and_what_it_prints_comes_back() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({"cols": 100, "rows": 30}));
        page.type_in(id, &say("round_trip"));
        page.until_output("canoa_round_trip");
        let kept = page.ask("terminal/scrollback", &json!({"id": id}));
        let bytes = BASE64
            .decode(kept["result"]["data"].as_str().expect("data").as_bytes())
            .expect("base64");
        assert!(String::from_utf8_lossy(&bytes).contains("canoa_round_trip"));
        assert_eq!(
            kept["result"]["end"].as_u64(),
            Some(page.output.len() as u64)
        );
        let closed = page.ask("terminal/close", &json!({"id": id}));
        assert_eq!(closed["result"]["closed"], true, "{closed}");
        let gone = page.ask("terminal/write", &json!({"id": id, "data": "AA=="}));
        assert_eq!(gone["error"]["code"], -32003, "{gone}");
    });
}

#[test]
fn the_shell_ending_is_told_with_its_code() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({}));
        page.type_in(id, &format!("exit 3{}", newline()));
        assert_eq!(page.until_exit(), (id, 3));
        let after = page.type_in(id, "x");
        assert_eq!(after["error"]["code"], -32002, "{after}");
        // An ended terminal stays until it is closed, so its last output can still be read.
        let kept = page.ask("terminal/scrollback", &json!({"id": id}));
        assert!(kept["result"]["data"].is_string(), "{kept}");
    });
}

#[cfg(unix)]
#[test]
fn a_resize_reaches_the_shell() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({"cols": 100, "rows": 30}));
        page.type_in(id, "stty size\n");
        page.until_output("30 100");
        let resized = page.ask(
            "terminal/resize",
            &json!({"id": id, "cols": 120, "rows": 40}),
        );
        assert!(resized.get("result").is_some(), "{resized}");
        page.type_in(id, "stty size\n");
        page.until_output("40 120");
    });
}

#[cfg(windows)]
#[test]
fn a_resize_is_taken() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({"cols": 100, "rows": 30}));
        let resized = page.ask(
            "terminal/resize",
            &json!({"id": id, "cols": 120, "rows": 40}),
        );
        assert!(resized.get("result").is_some(), "{resized}");
    });
}

/// The command lines of every process on this computer.
fn command_lines() -> String {
    let out = if cfg!(windows) {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }",
            ])
            .output()
    } else {
        std::process::Command::new("ps")
            .args(["-eo", "args"])
            .output()
    }
    .expect("lists processes");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn running(marker: &str) -> bool {
    command_lines().lines().any(|line| line.contains(marker))
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let until = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn closing_a_terminal_ends_what_its_shell_started() {
    let marker = if cfg!(windows) {
        "-n 3117 127.0.0.1"
    } else {
        "sleep 3117"
    };
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({}));
        let start = if cfg!(windows) {
            format!("start /b ping -n 3117 127.0.0.1 > nul{}", newline())
        } else {
            format!("sleep 3117 &{}", newline())
        };
        page.type_in(id, &start);
        wait_until("the command the shell started never ran", || {
            running(marker)
        });
        page.ask("terminal/close", &json!({"id": id}));
        wait_until("what the shell started kept running after close", || {
            !running(marker)
        });
    });
}

#[test]
fn the_end_of_the_link_ends_every_shell_and_what_it_started() {
    let marker = if cfg!(windows) {
        "-n 3118 127.0.0.1"
    } else {
        "sleep 3118"
    };
    let control = control(terminals());
    let ((), ended) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({}));
        let start = if cfg!(windows) {
            format!("start /b ping -n 3118 127.0.0.1 > nul{}", newline())
        } else {
            format!("sleep 3118 &{}", newline())
        };
        page.type_in(id, &start);
        page.open(&json!({}));
        wait_until("the command the shell started never ran", || {
            running(marker)
        });
    });
    assert!(
        ended < Duration::from_secs(10),
        "the link took {ended:?} to end"
    );
    wait_until("what the shell started outlived its link", || {
        !running(marker)
    });
    assert_eq!(control.terminals().running(), 0);
}

#[test]
fn a_key_needs_the_terminal_grant_and_the_link_it_came_on() {
    let control = control(terminals());
    let without = peer(&[Grant::Folders, Grant::LedgerRead]);
    let ((), _) = on_a_link(&control, &without, |page| {
        for method in [
            "terminal/open",
            "terminal/write",
            "terminal/resize",
            "terminal/scrollback",
            "terminal/close",
        ] {
            let answer = page.ask(method, &json!({"id": 1}));
            assert_eq!(answer["error"]["code"], -32001, "{method}: {answer}");
        }
    });
    let with = peer(&[Grant::Terminal]);
    let ((), _) = on_a_link(&control, &with, |page| {
        let id = page.open(&json!({}));
        page.ask("terminal/close", &json!({"id": id}));
    });
    // Without a link to push output on, a request is refused rather than answered with a shell nobody can hear.
    let line = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "terminal/open", "params": {}}),
    )
    .expect("encodes");
    let answered: Value =
        serde_json::from_slice(&control.handle(&with, &line).expect("answered")).expect("JSON");
    assert_eq!(answered["error"]["code"], -32002, "{answered}");
    let denied: Value =
        serde_json::from_slice(&control.handle(&without, &line).expect("answered")).expect("JSON");
    assert_eq!(denied["error"]["code"], -32001, "{denied}");
    assert_eq!(control.terminals().running(), 0);
}

#[test]
fn a_terminal_is_its_links_alone() {
    let control = control(terminals());
    let holder = peer(&[Grant::Terminal]);
    let ((), _) = on_a_link(&control, &holder, |first| {
        let id = first.open(&json!({}));
        // Another link of the same key has its own terminals, numbered from the same start: the first's is not there.
        let ((), _) = on_a_link(&control, &holder, |second| {
            let other = second.open(&json!({}));
            assert_eq!(other, id);
            let gone = second.ask("terminal/write", &json!({"id": other + 1, "data": "AA=="}));
            assert_eq!(gone["error"]["code"], -32003, "{gone}");
        });
        // Its own is still there.
        let kept = first.ask("terminal/scrollback", &json!({"id": id}));
        assert!(kept.get("result").is_some(), "{kept}");
    });
}

#[test]
fn a_request_that_is_not_well_formed_is_refused_with_its_reason() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let cases = [
            json!({"cwd": "relative/folder"}),
            json!({"cwd": std::env::temp_dir().join("canoa-no-such-folder-3117")}),
            json!({"cols": 0}),
            json!({"rows": 1001}),
            json!({"cols": "wide"}),
            json!({"cwd": 5}),
        ];
        for params in cases {
            let answer = page.ask("terminal/open", &params);
            assert_eq!(answer["error"]["code"], -32602, "{params}: {answer}");
        }
        let id = page.open(&json!({}));
        for data in [
            json!("not base64!"),
            json!(5),
            json!(BASE64.encode(&vec![b'a'; MAX_WRITE + 1])),
        ] {
            let answer = page.ask("terminal/write", &json!({"id": id, "data": data}));
            assert_eq!(answer["error"]["code"], -32602, "{answer}");
        }
        let none = page.ask("terminal/write", &json!({"data": "AA=="}));
        assert_eq!(none["error"]["code"], -32602, "{none}");
        let size = page.ask("terminal/resize", &json!({"id": id, "cols": 0}));
        assert_eq!(size["error"]["code"], -32602, "{size}");
    });
}

#[test]
fn a_link_has_a_limit_of_terminals_and_a_refusal_says_so() {
    let control = control(terminals());
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        for _ in 0..MAX_PER_LINK {
            page.open(&json!({}));
        }
        let more = page.ask("terminal/open", &json!({}));
        assert_eq!(more["error"]["code"], -32002, "{more}");
        assert!(
            more["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("at most")),
            "{more}"
        );
    });
    assert_eq!(control.terminals().running(), 0);
}

#[test]
fn a_shell_that_cannot_start_is_refused_with_its_name() {
    let control = control(Terminals::new(
        Some("canoa-no-such-shell-3117".to_owned()),
        Some(std::env::temp_dir()),
    ));
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let answer = page.ask("terminal/open", &json!({}));
        assert_eq!(answer["error"]["code"], -32002, "{answer}");
        assert!(
            answer["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("canoa-no-such-shell-3117")),
            "{answer}"
        );
    });
    assert_eq!(control.terminals().running(), 0);
}

#[test]
fn the_scrollback_keeps_the_latest_and_counts_all() {
    let mut scroll = Scroll::default();
    let chunk = vec![b'a'; SCROLLBACK / 2 + 1];
    scroll.keep(&chunk);
    let end = scroll.keep(&chunk);
    assert_eq!(end, 2 * chunk.len() as u64);
    assert_eq!(scroll.bytes.len(), SCROLLBACK);
    scroll.keep(b"tail");
    assert_eq!(scroll.bytes.len(), SCROLLBACK);
    assert!(scroll.bytes.iter().rev().take(4).copied().eq(*b"liat"));
}

#[test]
fn the_grant_has_a_name_and_the_owner_holds_it() {
    assert_eq!(Grant::named("terminal"), Some(Grant::Terminal));
    assert!(Grant::all().contains(&Grant::Terminal));
    let control = control(terminals());
    let (told, _) = on_a_link(&control, &peer(&[Grant::Terminal]), |page| {
        page.ask("host/grants", &json!({}))
    });
    assert_eq!(told["result"]["grants"], json!(["terminal"]));
}

/// A link whose queue is full for its first `refusals` sends, as a WebSocket's is when output outpaces it.
struct Slow {
    refusals: core::sync::atomic::AtomicUsize,
    sent: Mutex<Vec<u8>>,
}

impl Channel for Slow {
    fn send(&self, message: &[u8]) -> Result<(), coqui_transport::Failure> {
        let left = self.refusals.load(Ordering::SeqCst);
        if left > 0 {
            self.refusals.store(left - 1, Ordering::SeqCst);
            return Err(coqui_transport::Failure::NoProgress);
        }
        let message: Value = serde_json::from_slice(message).expect("JSON");
        if message["method"] == "terminal/output" {
            let data = message["params"]["data"].as_str().expect("data");
            lock(&self.sent).extend(BASE64.decode(data.as_bytes()).expect("base64"));
        }
        Ok(())
    }

    fn receive(&self, _: Option<Duration>) -> Result<Option<Vec<u8>>, coqui_transport::Failure> {
        Err(coqui_transport::Failure::Timeout)
    }

    fn close(&self) {}
}

#[test]
fn a_link_that_is_full_for_a_while_does_not_end_the_shell() {
    let host = terminals();
    let slow = Slow {
        refusals: core::sync::atomic::AtomicUsize::new(6),
        sent: Mutex::new(Vec::new()),
    };
    std::thread::scope(|scope| {
        let link = Link::new(scope, &slow, &host);
        let opened = link.call("terminal/open", &json!({})).expect("opened");
        let id = opened["id"].clone();
        let typed = BASE64.encode(say("slow_link").as_bytes());
        link.call("terminal/write", &json!({"id": id, "data": typed}))
            .expect("typed");
        wait_until("the output never reached a link that was only slow", || {
            String::from_utf8_lossy(&lock(&slow.sent)).contains("canoa_slow_link")
        });
        assert!(link.any_running(), "the shell was ended for a full queue");
    });
}

#[cfg(windows)]
#[test]
fn a_bare_shell_name_is_found_on_path_as_an_absolute_path_and_never_in_the_working_folder() {
    let found = resolve(std::ffi::OsStr::new("cmd")).expect("cmd is on PATH");
    assert!(found.is_absolute() && found.is_file(), "{found:?}");
    assert!(resolve(std::ffi::OsStr::new("canoa-no-such-shell-3117")).is_none());
    // A name with a folder in it that is not absolute is looked for nowhere.
    assert!(resolve(std::ffi::OsStr::new(r".\cmd.exe")).is_none());
    // A program in a folder that is not on PATH is not found by its bare name, whatever the working folder holds.
    let folder = std::env::temp_dir().join("canoa-resolve-3117");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(folder.join("canoa-only-here-3117.exe"), b"").expect("file");
    assert!(resolve(std::ffi::OsStr::new("canoa-only-here-3117")).is_none());
    assert!(resolve(folder.join("canoa-only-here-3117").as_os_str()).is_some());
    std::fs::remove_dir_all(&folder).expect("removed");
}

#[test]
fn a_shell_whose_path_has_spaces_starts() {
    let folder = std::env::temp_dir().join("canoa shell 3117");
    std::fs::create_dir_all(&folder).expect("folder");
    let name = if cfg!(windows) {
        "my shell.exe"
    } else {
        "my shell"
    };
    std::fs::copy(shell(), folder.join(name)).expect("copied");
    let spaced = folder.join(name);
    let control = control(Terminals::new(
        Some(spaced.to_string_lossy().into_owned()),
        Some(std::env::temp_dir()),
    ));
    let ((), _) = on_a_link(&control, &Caller::Owner, |page| {
        let id = page.open(&json!({}));
        page.type_in(id, &say("spaced"));
        page.until_output("canoa_spaced");
    });
    std::fs::remove_dir_all(&folder).expect("removed");
}
