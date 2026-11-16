//! A stand-in MCP server over HTTP, for the tests of a session that is given one. It answers `tools/call` with the text `the
//! marker` and `prompts/get` with the text `the skill`, and appends one JSON line per request to the file `--log=` names:
//! `{"authorization": ..., "method": ..., "params": ...}`. By default it serves `https` on 127.0.0.1 with a certificate it makes
//! for the run and writes as PEM to `--ca=`; `--plain` serves `http`. It prints `{"ready":true,"port":N}` when it listens and
//! serves until it is killed.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use rustls::pki_types::{PrivateKeyDer, pem::PemObject as _};
use serde_json::{Value, json};

fn option_of(name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    std::env::args().find_map(|arg| arg.strip_prefix(prefix.as_str()).map(str::to_owned))
}

fn reply(body: &Value) -> String {
    let text = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )
}

/// Reads one request and answers it; the request is noted in the log first.
fn serve_one(stream: &mut (impl Read + Write), log: &str) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let (head, mut body) = loop {
        let Ok(count) = stream.read(&mut chunk) else {
            return;
        };
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break (
                String::from_utf8_lossy(&bytes[..at]).into_owned(),
                bytes[at + 4..].to_vec(),
            );
        }
    };
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0);
    while body.len() < length {
        let Ok(count) = stream.read(&mut chunk) else {
            return;
        };
        if count == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..count]);
    }
    let authorization = head
        .lines()
        .find_map(|line| line.strip_prefix("Authorization: "))
        .unwrap_or("");
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let method = request["method"].as_str().unwrap_or("");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let _ = writeln!(
            file,
            "{}",
            json!({"authorization": authorization, "method": method, "params": request["params"]})
        );
    }
    let result = match method {
        "tools/call" => json!({"content": [{"type": "text", "text": "the marker"}]}),
        "prompts/get" => {
            json!({"messages": [{"role": "user", "content": {"type": "text", "text": "the skill"}}]})
        }
        _ => json!({}),
    };
    let answer = reply(&json!({"jsonrpc": "2.0", "id": request["id"], "result": result}));
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
}

fn main() {
    let log = option_of("log").expect("--log=FILE");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    if std::env::args().any(|arg| arg == "--plain") {
        println!("{}", json!({"ready": true, "port": port}));
        for mut stream in listener.incoming().flatten() {
            serve_one(&mut stream, &log);
        }
        return;
    }
    let made =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("a certificate");
    std::fs::write(option_of("ca").expect("--ca=FILE"), made.cert.pem()).expect("the certificate");
    let key =
        PrivateKeyDer::from_pem_slice(made.signing_key.serialize_pem().as_bytes()).expect("a key");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(vec![made.cert.der().clone()], key)
            .expect("a server"),
    );
    println!("{}", json!({"ready": true, "port": port}));
    for tcp in listener.incoming().flatten() {
        let Ok(connection) = rustls::ServerConnection::new(Arc::clone(&config)) else {
            continue;
        };
        let mut stream = rustls::StreamOwned::new(connection, tcp);
        serve_one(&mut stream, &log);
        stream.conn.send_close_notify();
        let _ = stream.flush();
    }
}
