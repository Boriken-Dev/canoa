//! `skill/search` against a canned transport and against a directory on this computer that answers the way skills.sh does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;
use crate::control::overlays_tests::{ask, dir, key};
use crate::control::registry::Registry;
use crate::control::{
    Caller, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, Files, Grant, UploadLimits,
};
use crate::store::tools::{Reply, ToolsLimits, Transport};

#[derive(Debug)]
struct Canned {
    status: u16,
    body: Vec<u8>,
    asked: Mutex<Vec<String>>,
}

impl Transport for Canned {
    fn get(&self, url: &str, _limits: &ToolsLimits) -> Result<Reply, String> {
        self.asked.lock().expect("lock").push(url.to_owned());
        Ok(Reply {
            status: self.status,
            location: None,
            body: Box::new(std::io::Cursor::new(self.body.clone())),
        })
    }
}

fn canned(status: u16, body: &[u8]) -> Arc<Canned> {
    Arc::new(Canned {
        status,
        body: body.to_vec(),
        asked: Mutex::new(Vec::new()),
    })
}

/// A control whose canoa.cfg holds `config` (none for no file) and whose registry reads through `transport`.
fn control_with(config: Option<&str>, transport: Option<Arc<Canned>>) -> (Control, PathBuf) {
    let tmp = dir("skill-search");
    let file = tmp.join("canoa.cfg");
    if let Some(text) = config {
        std::fs::write(&file, text).expect("config");
    }
    let registry = Registry::new(None);
    let registry = match transport {
        Some(transport) => registry.with_transport(transport),
        None => registry,
    };
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        None,
        Vec::new(),
        Some(file.clone()),
        None,
        UploadLimits::default(),
    ))
    .with_registry(registry);
    (control, file)
}

fn search(control: &Control, params: Value) -> Value {
    ask(control, &Caller::Owner, "skill/search", params)
}

const ANSWER: &str = r#"{"query":"pdf","searchType":"fuzzy","skills":[
    {"id":"anthropics/skills/pdf","source":"anthropics/skills","skillId":"pdf","name":"pdf","installs":231770,"extra":"left out"},
    {"id":"a/b/c","source":"a/b","skillId":"../etc","name":"x","installs":1},
    {"source":"o/r","skillId":"nice-one","installs":"many"},
    {"id":"o/r/","source":"","skillId":"gone","name":"gone","installs":5},
    {"id":"o/r/ctl","source":"o/r","skillId":"ctl","name":"bad\nname","installs":5},
    {"id":"x","source":"/etc","skillId":"passwd","name":"p","installs":9},
    {"id":"x","source":"https://evil.example/o/r","skillId":"s","name":"p","installs":9},
    {"id":"x","source":"git@evil.example:o/r","skillId":"s","name":"p","installs":9},
    {"id":"x","source":"o/r/deeper","skillId":"s","name":"p","installs":9},
    {"id":"x","source":"../r","skillId":"s","name":"p","installs":9}
],"count":10}"#;

#[test]
fn the_search_is_off_until_the_person_turns_it_on() {
    let transport = canned(200, ANSWER.as_bytes());
    for config in [
        None,
        Some("[store]\n"),
        Some("[store]\nskills_search = false\n"),
    ] {
        let (control, _) = control_with(config, Some(transport.clone()));
        let answer = search(&control, json!({"query": "pdf"}));
        assert_eq!(answer["error"]["code"], -32002, "{config:?}: {answer}");
        assert!(
            answer["error"]["message"]
                .as_str()
                .expect("text")
                .contains("off"),
            "{answer}"
        );
    }
    assert!(
        transport.asked.lock().expect("lock").is_empty(),
        "nothing is asked of the directory"
    );
}

#[test]
fn turning_it_on_asks_skills_sh_and_turning_it_off_stops_asking() {
    let transport = canned(200, ANSWER.as_bytes());
    let (control, file) = control_with(
        Some("[store]\nskills_search = true\n"),
        Some(transport.clone()),
    );
    let answer = search(&control, json!({"query": " pdf tools "}));
    assert_eq!(answer["result"]["from"], "https://skills.sh", "{answer}");
    assert_eq!(
        *transport.asked.lock().expect("lock"),
        ["https://skills.sh/api/search?q=pdf%20tools&limit=20"]
    );
    std::fs::write(&file, "[store]\nskills_search = false\n").expect("config");
    let off = search(&control, json!({"query": "pdf"}));
    assert_eq!(off["error"]["code"], -32002, "{off}");
    assert_eq!(transport.asked.lock().expect("lock").len(), 1);
}

#[test]
fn a_result_keeps_only_what_the_page_reads_and_a_bad_one_is_left_out() {
    let (control, _) = control_with(
        Some("[store]\nskills_search = true\n"),
        Some(canned(200, ANSWER.as_bytes())),
    );
    let answer = search(&control, json!({"query": "pdf"}));
    assert_eq!(
        answer["result"]["skills"],
        json!([
            {"id": "anthropics/skills/pdf", "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 231_770},
            {"id": "o/r/nice-one", "source": "o/r", "skillId": "nice-one", "name": "nice-one", "installs": 0},
            {"id": "o/r/ctl", "source": "o/r", "skillId": "ctl", "name": "ctl", "installs": 5},
        ]),
        "{answer}"
    );
    let one = search(&control, json!({"query": "pdf", "limit": 1}));
    assert_eq!(one["result"]["skills"].as_array().expect("list").len(), 1);
}

#[test]
fn a_query_and_a_limit_are_checked_before_anything_is_asked() {
    let transport = canned(200, ANSWER.as_bytes());
    let (control, _) = control_with(
        Some("[store]\nskills_search = true\n"),
        Some(transport.clone()),
    );
    for params in [
        json!({}),
        json!({"query": "a"}),
        json!({"query": "  a  "}),
        json!({"query": "pd\nf"}),
        json!({"query": "x".repeat(129)}),
        json!({"query": 7}),
        json!({"query": "pdf", "limit": 0}),
        json!({"query": "pdf", "limit": 51}),
        json!({"query": "pdf", "limit": "5"}),
    ] {
        let answer = search(&control, params.clone());
        assert!(answer.get("error").is_some(), "{params}: {answer}");
    }
    assert!(transport.asked.lock().expect("lock").is_empty());
    let longest = search(&control, json!({"query": "x".repeat(128), "limit": 50}));
    assert!(longest.get("result").is_some(), "{longest}");
}

#[test]
fn another_directory_is_named_by_its_address() {
    let cases = [
        (
            "\"https://find.example/skills/\"",
            Some("https://find.example/skills/api/search?q=pdf&limit=20"),
        ),
        (
            "\"http://127.0.0.1:9\"",
            Some("http://127.0.0.1:9/api/search?q=pdf&limit=20"),
        ),
        (
            "\"http://localhost:9/x\"",
            Some("http://localhost:9/x/api/search?q=pdf&limit=20"),
        ),
        ("\"http://find.example\"", None),
        ("\"https://u:p@find.example\"", None),
        ("\"https://find.example/?token=1\"", None),
        ("\"https://find.example/#x\"", None),
        ("\"ftp://find.example\"", None),
        ("\"not an address\"", None),
        ("7", None),
    ];
    for (value, asked) in cases {
        let transport = canned(200, ANSWER.as_bytes());
        let (control, _) = control_with(
            Some(&format!("[store]\nskills_search = {value}\n")),
            Some(transport.clone()),
        );
        let answer = search(&control, json!({"query": "pdf"}));
        match asked {
            Some(url) => {
                assert!(answer.get("result").is_some(), "{value}: {answer}");
                assert_eq!(*transport.asked.lock().expect("lock"), [url], "{value}");
            }
            None => {
                assert_eq!(answer["error"]["code"], -32002, "{value}: {answer}");
                assert!(transport.asked.lock().expect("lock").is_empty(), "{value}");
            }
        }
    }
}

#[test]
fn what_goes_wrong_is_said_in_words() {
    let on = Some("[store]\nskills_search = true\n");
    let message = |status: u16, body: &[u8]| {
        let (control, _) = control_with(on, Some(canned(status, body)));
        let answer = search(&control, json!({"query": "pdf"}));
        answer["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("a refusal: {answer}"))
            .to_owned()
    };
    assert_eq!(message(429, b""), "The skills search answered 429");
    assert!(message(302, b"").contains("redirect"));
    assert!(message(200, b"<html>").contains("cannot read"));
    assert!(message(200, br#"{"error":"nope"}"#).contains("cannot read"));
    assert!(message(200, br#"{"skills":{}}"#).contains("cannot read"));
    let big = vec![b' '; usize::try_from(MAX_SEARCH_BYTES).expect("fits") + 1];
    assert!(message(200, &big).contains("larger"));
    let empty = {
        let (control, _) = control_with(on, Some(canned(200, br#"{"skills":[]}"#)));
        search(&control, json!({"query": "zzzz"}))
    };
    assert_eq!(empty["result"]["skills"], json!([]), "{empty}");
}

#[test]
fn a_key_needs_the_overlay_write_grant() {
    let transport = canned(200, ANSWER.as_bytes());
    let (control, _) = control_with(
        Some("[store]\nskills_search = true\n"),
        Some(transport.clone()),
    );
    let read_only = key("k", &[Grant::OverlayRead]);
    let denied = ask(
        &control,
        &read_only,
        "skill/search",
        json!({"query": "pdf"}),
    );
    assert_eq!(denied["error"]["code"], -32001, "{denied}");
    assert!(transport.asked.lock().expect("lock").is_empty());
    let writer = key("k", &[Grant::OverlayWrite]);
    let allowed = ask(&control, &writer, "skill/search", json!({"query": "pdf"}));
    assert!(allowed.get("result").is_some(), "{allowed}");
    // The same off switch holds for a key.
    let (off, _) = control_with(None, Some(transport));
    let refused = ask(&off, &writer, "skill/search", json!({"query": "pdf"}));
    assert_eq!(refused["error"]["code"], -32002, "{refused}");
}

#[test]
fn a_directory_on_this_computer_is_asked_over_http_and_no_redirect_is_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let port = listener.local_addr().expect("addr").port();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let taken = heard.clone();
    let server = std::thread::spawn(move || {
        for (index, stream) in listener.incoming().take(2).enumerate() {
            let mut stream = stream.expect("conn");
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).expect("request");
            let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
            taken
                .lock()
                .expect("lock")
                .push(text.lines().next().unwrap_or_default().to_owned());
            let reply = if index == 0 {
                let body = r#"{"query":"pdf","searchType":"fuzzy","skills":[{"id":"anthropics/skills/pdf","source":"anthropics/skills","skillId":"pdf","name":"pdf","installs":231770}],"count":1}"#;
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://127.0.0.1:1/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
            };
            stream.write_all(reply.as_bytes()).expect("reply");
        }
    });
    let (control, _) = control_with(
        Some(&format!(
            "[store]\nskills_search = \"http://127.0.0.1:{port}\"\n"
        )),
        None,
    );
    let first = search(&control, json!({"query": "pdf", "limit": 5}));
    assert_eq!(
        first["result"]["skills"],
        json!([{"id": "anthropics/skills/pdf", "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 231_770}]),
        "{first}"
    );
    let second = search(&control, json!({"query": "pdf"}));
    assert!(
        second["error"]["message"]
            .as_str()
            .expect("text")
            .contains("redirect"),
        "{second}"
    );
    server.join().expect("server ends");
    let heard = heard.lock().expect("lock");
    assert!(
        heard[0].starts_with("GET /api/search?q=pdf&limit=5 HTTP/1.1"),
        "{heard:?}"
    );
}
