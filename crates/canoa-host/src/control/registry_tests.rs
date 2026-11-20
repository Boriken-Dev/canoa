use std::net::TcpListener;
use std::sync::Mutex;

use super::*;
use crate::control::overlays_tests::ask;
use crate::control::{Control, Grant};
use crate::store::tools::Reply;

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

fn peer(id: &str, grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn canned(status: u16, body: &[u8]) -> Arc<Canned> {
    Arc::new(Canned {
        status,
        body: body.to_vec(),
        asked: Mutex::new(Vec::new()),
    })
}

fn control_with(transport: Arc<Canned>, cache: Option<PathBuf>) -> Control {
    Control::open(None, 10, 1024, Vec::new())
        .expect("opens")
        .with_registry(Registry::new(cache).with_transport(transport))
}

const LIST: &str = "https://registry.example/v0.1/servers?limit=30&search=files";

#[test]
fn the_person_reads_any_registrys_list_and_the_body_comes_back() {
    let transport = canned(200, br#"{"servers":[]}"#);
    let control = control_with(transport.clone(), None);
    let answer = ask(
        &control,
        &Caller::Owner,
        "registry/fetch",
        json!({"url": LIST}),
    );
    assert_eq!(answer["result"]["body"], r#"{"servers":[]}"#, "{answer}");
    assert_eq!(*transport.asked.lock().expect("lock"), [LIST]);
}

#[test]
fn only_a_registrys_list_address_is_read() {
    let transport = canned(200, b"{}");
    let control = control_with(transport.clone(), None);
    for url in [
        "https://registry.example/other",
        "https://registry.example/v0.1/servers/x",
        "https://registry.example/v0.1/servers?token=1",
        "https://u:p@registry.example/v0.1/servers",
        "https://registry.example/v0.1/servers#frag",
        "ftp://registry.example/v0.1/servers",
        "file:///etc/passwd",
        "https://169.254.169.254/latest/meta-data",
    ] {
        let answer = ask(
            &control,
            &Caller::Owner,
            "registry/fetch",
            json!({"url": url}),
        );
        assert_eq!(answer["error"]["code"], -32002, "{url}: {answer}");
    }
    assert!(transport.asked.lock().expect("lock").is_empty());
    let loopback = ask(
        &control,
        &Caller::Owner,
        "registry/fetch",
        json!({"url": "http://127.0.0.1:9/v0.1/servers"}),
    );
    assert!(loopback.get("result").is_some(), "{loopback}");
    for plain in [
        "http://registry.example/v0.1/servers",
        "http://192.0.2.11:8080/v0.1/servers?limit=3",
    ] {
        let answer = ask(
            &control,
            &Caller::Owner,
            "registry/fetch",
            json!({"url": plain}),
        );
        assert!(answer.get("result").is_some(), "{plain}: {answer}");
        assert!(answer["result"]["warning"].is_string(), "{plain}: {answer}");
    }
    assert!(loopback["result"].get("warning").is_none(), "{loopback}");
}

#[test]
fn a_registrys_refusal_oversize_answer_and_redirect_are_said_in_words() {
    let owner = Caller::Owner;
    let redirect = control_with(canned(302, b""), None);
    let said = ask(&redirect, &owner, "registry/fetch", json!({"url": LIST}));
    assert!(
        said["error"]["message"]
            .as_str()
            .expect("text")
            .contains("a redirect"),
        "{said}"
    );
    let gone = control_with(canned(404, b""), None);
    let said = ask(&gone, &owner, "registry/fetch", json!({"url": LIST}));
    assert_eq!(said["error"]["message"], "The registry answered 404");
    let big = control_with(
        canned(
            200,
            &vec![b'x'; usize::try_from(MAX_REGISTRY_BYTES).expect("fits") + 1],
        ),
        None,
    );
    let said = ask(&big, &owner, "registry/fetch", json!({"url": LIST}));
    assert!(
        said["error"]["message"]
            .as_str()
            .expect("text")
            .contains("larger"),
        "{said}"
    );
}

#[test]
fn a_key_needs_the_grant_and_reads_only_listed_registries() {
    let transport = canned(200, b"{}");
    let control = control_with(transport.clone(), None);
    let none = peer("k", &[Grant::OverlayRead]);
    let denied = ask(&control, &none, "registry/fetch", json!({"url": LIST}));
    assert_eq!(denied["error"]["code"], -32001, "{denied}");
    // With the grant but no [store.mcps.catalogs] (this control has no canoa.cfg), only the official registry is read.
    let key = peer("k", &[Grant::OverlayWrite]);
    let other = ask(&control, &key, "registry/fetch", json!({"url": LIST}));
    assert_eq!(other["error"]["code"], -32002, "{other}");
    assert!(transport.asked.lock().expect("lock").is_empty());
    let official = ask(
        &control,
        &key,
        "registry/fetch",
        json!({"url": "https://registry.modelcontextprotocol.io/v0.1/servers?limit=30"}),
    );
    assert!(official.get("result").is_some(), "{official}");
}

#[test]
fn the_bases_a_key_may_read_are_the_official_one_and_the_http_ones_listed() {
    let store = json!({"mcps": {"catalogs": {"a": "https://a.example/", "git": "https://git.example/r.git@main#list.json", "b": "https://b.example/x"}}});
    let bases = listed_bases(&store);
    assert_eq!(bases.len(), 3, "{bases:?}");
    assert!(same_base(
        &bases[1],
        &list_endpoint_base("https://A.example/v0.1/servers").expect("base")
    ));
    assert!(same_base(
        &bases[2],
        &list_endpoint_base("https://b.example/x/v0.1/servers?limit=1").expect("base")
    ));
}

#[test]
fn a_real_registry_on_this_computer_is_read_with_no_redirect_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let port = listener.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || {
        for (index, stream) in listener.incoming().take(2).enumerate() {
            let mut stream = stream.expect("conn");
            let mut buffer = [0_u8; 2048];
            let _ = stream.read(&mut buffer).expect("request");
            let reply = if index == 0 {
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 12\r\nconnection: close\r\n\r\n{\"servers\":}".to_owned()
            } else {
                "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:1/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
            };
            stream.write_all(reply.as_bytes()).expect("reply");
        }
    });
    let control = Control::open(None, 10, 1024, Vec::new()).expect("opens");
    let url = format!("http://127.0.0.1:{port}/v0.1/servers?limit=3");
    let first = ask(
        &control,
        &Caller::Owner,
        "registry/fetch",
        json!({"url": url}),
    );
    assert_eq!(first["result"]["body"], "{\"servers\":}", "{first}");
    let second = ask(
        &control,
        &Caller::Owner,
        "registry/fetch",
        json!({"url": url}),
    );
    assert!(
        second["error"]["message"]
            .as_str()
            .expect("text")
            .contains("redirect"),
        "{second}"
    );
    server.join().expect("server ends");
}

#[test]
fn which_finds_a_runnable_file_in_an_absolute_directory_of_path_only() {
    let root = crate::control::overlays_tests::dir("which");
    let name = if cfg!(windows) {
        "tool-x.cmd"
    } else {
        "tool-x"
    };
    let file = root.join(name);
    fs::write(&file, "echo").expect("file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).expect("mode");
        fs::write(root.join("plain"), "x").expect("file");
    }
    let path = std::env::join_paths([PathBuf::from("relative"), root.clone()]).expect("joins");
    assert_eq!(which_in(&path, "", "tool-x"), Some(file));
    assert_eq!(which_in(&path, "", "missing"), None);
    #[cfg(unix)]
    assert_eq!(which_in(&path, "", "plain"), None, "not executable");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn which_looks_in_the_folders_of_installed_runtimes_after_path() {
    let root = crate::control::overlays_tests::dir("which-runtime");
    let name = if cfg!(windows) {
        "runtime-tool-x.cmd"
    } else {
        "runtime-tool-x"
    };
    let file = root.join(name);
    fs::write(&file, "echo").expect("file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).expect("mode");
    }
    let asked = json!({"names": ["runtime-tool-x"]});
    let without = which(&asked, &[]).expect("answers");
    assert_eq!(without["found"]["runtime-tool-x"], Value::Null);
    let with = which(&asked, std::slice::from_ref(&root)).expect("answers");
    assert_eq!(
        with["found"]["runtime-tool-x"],
        Value::String(file.display().to_string())
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn which_answers_names_it_is_given_and_refuses_paths() {
    let control = Control::open(None, 10, 1024, Vec::new()).expect("opens");
    let key = peer("k", &[Grant::StoreRead]);
    let said = ask(
        &control,
        &key,
        "host/which",
        json!({"names": ["definitely-not-a-program-here"]}),
    );
    assert_eq!(
        said["result"]["found"]["definitely-not-a-program-here"],
        Value::Null,
        "{said}"
    );
    for bad_name in ["../x", "a/b", "", "a b", "C:\\x"] {
        let said = ask(&control, &key, "host/which", json!({"names": [bad_name]}));
        assert_eq!(said["error"]["code"], -32602, "{bad_name}: {said}");
    }
    let denied = ask(
        &control,
        &peer("k", &[]),
        "host/which",
        json!({"names": ["git"]}),
    );
    assert_eq!(denied["error"]["code"], -32001);
}

#[test]
fn a_cached_document_is_kept_per_key_and_must_be_a_json_object() {
    let root = crate::control::overlays_tests::dir("contents");
    let control = control_with(canned(200, b"{}"), Some(root.clone()));
    let a = peer("sha256:aaa", &[Grant::OverlayRead]);
    let b = peer("sha256:bbb", &[Grant::OverlayRead]);
    let none = ask(&control, &a, "store/contents", json!({"label": "main"}));
    assert_eq!(none["result"]["document"], Value::Null, "{none}");
    let put = ask(
        &control,
        &a,
        "store/contents_put",
        json!({"label": "main", "document": "{\"version\":1}"}),
    );
    assert!(put.get("result").is_some(), "{put}");
    let got = ask(&control, &a, "store/contents", json!({"label": "main"}));
    assert_eq!(got["result"]["document"], "{\"version\":1}");
    let other = ask(&control, &b, "store/contents", json!({"label": "main"}));
    assert_eq!(
        other["result"]["document"],
        Value::Null,
        "another key reads nothing of it"
    );
    let owner = ask(
        &control,
        &Caller::Owner,
        "store/contents",
        json!({"label": "main"}),
    );
    assert_eq!(owner["result"]["document"], Value::Null);
    for document in ["not json", "[1]", "3"] {
        let said = ask(
            &control,
            &a,
            "store/contents_put",
            json!({"label": "main", "document": document}),
        );
        assert_eq!(said["error"]["code"], -32602, "{document}: {said}");
    }
    let huge = "x".repeat(MAX_DOCUMENT_BYTES + 1);
    let said = ask(
        &control,
        &a,
        "store/contents_put",
        json!({"label": "main", "document": huge}),
    );
    assert_eq!(said["error"]["code"], -32002);
    // A label cannot name a place outside the key's folder.
    let said = ask(
        &control,
        &a,
        "store/contents_put",
        json!({"label": "../../escape", "document": "{}"}),
    );
    assert!(said.get("result").is_some(), "{said}");
    assert!(!root.join("escape.json").exists());
    assert!(!root.parent().expect("parent").join("escape.json").exists());
    let denied = ask(
        &control,
        &peer("c", &[]),
        "store/contents",
        json!({"label": "main"}),
    );
    assert_eq!(denied["error"]["code"], -32001);
    let _ = fs::remove_dir_all(&root);
}
