//! `options/choices` on the control channel: what an entry lists before a session starts, for a client with no host of its own
//! to run `canoa options` on, asked as an account when one is named.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use canoa_host::Link;
use linked::{Hosted, next, send};
use sealed::{App, Typed};
use serde_json::{Value, json};
use support::{example, quoted};

fn host() -> Hosted {
    let claude = example("harness_standin", false);
    let agent = example("stdio_agent", false);
    Hosted::start_with_setup(
        |dir| {
            let program = dir.join(format!("claude{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&claude, &program).expect("the stand-in is copied");
            format!(
                "[host]\naccount_store = \"file\"\n\n[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n\n[host.agents.Probe]\nprogram = {}\nargs = [\"--scripted\"]\n",
                quoted(&program.to_string_lossy()),
                quoted(&agent.to_string_lossy()),
            )
        },
        &[],
    )
}

fn call(link: &Link, id: u64, method: &str, params: &Value) -> Result<Value, Value> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    );
    loop {
        let message = next(link).expect("an answer");
        if message["id"] == id && message.get("method").is_none() {
            return match message.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(message["result"].clone()),
            };
        }
    }
}

fn model_ids(answer: &Value) -> Vec<&str> {
    answer["models"]
        .as_array()
        .expect("models")
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect()
}

#[test]
fn a_harness_entry_and_a_probed_program_list_their_models_over_the_link() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let claude = call(&link, 1, "options/choices", &json!({"entry": "Claude"})).expect("asked");
    assert_eq!(claude["known"], true, "{claude}");
    assert_eq!(claude["models"][0]["id"], "standin-small", "{claude}");
    let probe = call(&link, 2, "options/choices", &json!({"entry": "Probe"})).expect("asked");
    assert_eq!(probe["known"], true, "{probe}");
    assert_eq!(
        model_ids(&probe),
        ["default", "fast", "refuse-me"],
        "{probe}"
    );
    let missing = call(&link, 3, "options/choices", &json!({"entry": "Nobody"})).expect_err("none");
    assert_eq!(missing["code"], -32003, "{missing}");
    let bare = call(&link, 4, "options/choices", &json!({})).expect_err("no entry");
    assert_eq!(bare["code"], -32602, "{bare}");
    host.stop();
}

#[test]
fn an_entry_is_asked_as_an_account_it_offers_and_not_as_one_it_does_not_have() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let secret = host.local.link_secret().expect("the host minted a secret");
    let app = App::new(&secret[..]);
    let params = json!({"name": "work", "harness": "claude", "method": "api-key"});
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "accounts/add", "params": params});
    send(&link, &line);
    let typed = Typed::Fills(&[("api_key", "sk-ant-test-0123456789")]);
    loop {
        let message = next(&link).expect("progress");
        if message["method"] == "_coqui_acp/secret_request" {
            let result = app.answer(&message["params"], &typed).expect("typed");
            send(
                &link,
                &json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
            );
        } else if message["id"] == 1 && message.get("method").is_none() {
            assert!(message.get("error").is_none(), "{message}");
            break;
        }
    }
    call(
        &link,
        2,
        "accounts/offer",
        &json!({"entry": "Claude", "accounts": ["work"]}),
    )
    .expect("offered");
    let as_work = call(
        &link,
        3,
        "options/choices",
        &json!({"entry": "Claude", "account": "work"}),
    )
    .expect("asked as work");
    assert_eq!(as_work["known"], true, "{as_work}");
    assert!(as_work.get("signed_out").is_none(), "{as_work}");
    let other = call(
        &link,
        4,
        "options/choices",
        &json!({"entry": "Claude", "account": "nobody"}),
    )
    .expect("answered");
    assert_eq!(other["known"], false, "{other}");
    assert_eq!(other["unknown_kind"], "failed", "{other}");
    host.stop();
}

#[test]
fn an_account_with_no_session_is_read_on_demand_and_a_name_the_host_lacks_is_refused() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let secret = host.local.link_secret().expect("the host minted a secret");
    let app = App::new(&secret[..]);
    send(
        &link,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "accounts/add",
            "params": {"name": "work", "harness": "claude", "method": "api-key"}}),
    );
    let typed = Typed::Fills(&[("api_key", "sk-ant-test-0123456789")]);
    loop {
        let message = next(&link).expect("progress");
        if message["method"] == "_coqui_acp/secret_request" {
            let result = app.answer(&message["params"], &typed).expect("typed");
            send(
                &link,
                &json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
            );
        } else if message["id"] == 1 && message.get("method").is_none() {
            break;
        }
    }
    let listed = call(&link, 2, "accounts/list", &json!({})).expect("listed");
    let id = listed["accounts"][0]["id"].clone();
    // The key is read from the address its account names, so a loopback stand-in answers in place of the provider.
    let provider = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = provider.local_addr().expect("its address");
    std::thread::spawn(move || {
        for stream in provider.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = [0_u8; 2048];
            let _ = std::io::Read::read(&mut stream, &mut head);
            let _ = std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
        }
    });
    let file = host.scratch.0.join("canoa.cfg");
    let text = std::fs::read_to_string(&file).expect("canoa.cfg");
    let at = text.find("[host.accounts.work]").expect("the account");
    let end = at + text[at..].find('\n').expect("a line") + 1;
    let mut text = text;
    text.insert_str(
        end,
        &format!("settings = {{ base_url = \"http://{address}\" }}\n"),
    );
    std::fs::write(&file, text).expect("written");
    let read = call(&link, 3, "usage/read", &json!({"account": "work"})).expect("read");
    assert_eq!(read["account"]["account"], id, "{read}");
    assert!(read["account"]["supported"].is_boolean(), "{read}");
    let missing = call(&link, 4, "usage/read", &json!({"account": "nobody"})).expect_err("none");
    assert_eq!(missing["code"], -32003, "{missing}");
    host.stop();
}
