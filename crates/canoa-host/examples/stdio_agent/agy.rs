//! `stdio_agent --agy FIXTURE`: an ACP agent that plays Antigravity's usage surface. After `session/new` it serves the
//! recorded `GetUserStatus` answer FIXTURE holds on a loopback HTTP port of its own process, chunked as agy does, and
//! lists the fixture's models in the answer. It never reads a prompt's text and keeps no account of its own.
//!
//! `CANOA_TEST_AGY_LOG=FILE` appends one line per event, led by its process id (`start`, `cwd PATH`, `new`, `prompt`, `end`), so a test can tell
//! what the host started and ended. `_fixture/reads` answers how many status requests were served.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

pub fn run(args: &[String]) {
    let path = args
        .iter()
        .position(|arg| arg == "--agy")
        .and_then(|at| args.get(at + 1))
        .expect("--agy takes the fixture's path");
    let body = std::fs::read(path).expect("the fixture is readable");
    let status: Value = serde_json::from_slice(&body).expect("the fixture is JSON");
    let models: Vec<Value> = status["userStatus"]["cascadeModelConfigData"]["clientModelConfigs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            Some(json!({"modelId": model["modelId"].as_str()?, "name": model["label"].as_str()?}))
        })
        .collect();
    log("start");
    let reads = Arc::new(AtomicU64::new(0));
    let body = Arc::new(body);
    let mut serving = false;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (Some(method), Some(id)) = (
            message.get("method").and_then(Value::as_str),
            message.get("id").cloned(),
        ) else {
            continue;
        };
        let result = match method {
            "initialize" => json!({"protocolVersion": 1, "agentCapabilities": {}}),
            "session/new" => {
                log(&format!(
                    "cwd {}",
                    message["params"]["cwd"].as_str().unwrap_or("")
                ));
                if !serving {
                    serving = true;
                    serve(&body, &reads);
                }
                log("new");
                json!({"sessionId": "agy-session", "models": {"availableModels": models}})
            }
            "session/prompt" => {
                log("prompt");
                json!({"stopReason": "end_turn"})
            }
            "_fixture/reads" => json!({"reads": reads.load(Ordering::SeqCst)}),
            "_fixture/pid" => json!({"pid": std::process::id()}),
            _ => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})
                );
                continue;
            }
        };
        let _ = writeln!(
            stdout,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        );
        let _ = stdout.flush();
    }
    log("end");
}

/// Serves the status on a loopback port until the process ends. The port is bound before `session/new` is answered, so a
/// reading that follows the answer finds it.
fn serve(body: &Arc<Vec<u8>>, reads: &Arc<AtomicU64>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let (body, reads) = (Arc::clone(body), Arc::clone(reads));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (body, reads) = (Arc::clone(&body), Arc::clone(&reads));
            std::thread::spawn(move || answer(stream, &body, &reads));
        }
    });
}

fn answer(mut stream: TcpStream, body: &[u8], reads: &AtomicU64) {
    let mut reader = BufReader::new(stream.try_clone().expect("a second handle"));
    let mut length = 0_usize;
    let mut first = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if first.is_empty() {
            first.clone_from(&line);
        }
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut request = vec![0; length];
    let _ = reader.read_exact(&mut request);
    if !first.contains("GetUserStatus") {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    reads.fetch_add(1, Ordering::SeqCst);
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for chunk in body.chunks(4096) {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    let _ = stream.write_all(&response);
}

/// Appends `line` to the file `CANOA_TEST_AGY_LOG` names.
fn log(line: &str) {
    let Some(path) = std::env::var_os("CANOA_TEST_AGY_LOG") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{} {line}", std::process::id());
    }
}
