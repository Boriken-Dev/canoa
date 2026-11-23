//! A stand-in for a LiteLLM proxy on loopback: the sign-in a browser would do (discovery, registration, the authorize page, the
//! redirect, the token and its refresh), `/key/info`, `/v1/models`, `/v1/messages` and `/v1/responses`. It checks what a real proxy
//! checks that matters here (the PKCE verifier against the challenge, a code used once, a rotated refresh token) and counts every
//! request a key makes against that key's spend, so two programs on one key are seen to share a budget.
//!
//! Shared by this crate's tests and the host's, which include it by path.
#![allow(
    dead_code,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "each test uses part of it, and a server is std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// A key the proxy was given before anyone signed in, which a person may be handed.
pub const VIRTUAL_KEY: &str = "sk-virtual-0001";

/// What one request to the proxy was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub line: String,
    /// The credential it carried: the bearer, or the `x-api-key`.
    pub credential: String,
}

#[derive(Default)]
struct State {
    sso: bool,
    clients: BTreeMap<String, String>,
    codes: BTreeMap<String, (String, String, String)>,
    keys: BTreeSet<String>,
    expired: BTreeSet<String>,
    refresh: Option<String>,
    issued: u32,
    spend: BTreeMap<String, u64>,
    max_budget: Option<f64>,
    seen: Vec<Seen>,
}

/// A running stand-in proxy.
pub struct Proxy {
    /// `http://127.0.0.1:PORT`.
    pub address: String,
    state: Arc<Mutex<State>>,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A request's line, headers (names in lower case) and body.
fn read_request(stream: &mut TcpStream) -> Option<(String, BTreeMap<String, String>, String)> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.lines();
    let line = lines.next()?.to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|each| each.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;
    Some((line, headers, String::from_utf8_lossy(&body).into_owned()))
}

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'%' && at + 3 <= bytes.len())
            .then(|| u8::from_str_radix(&String::from_utf8_lossy(&bytes[at + 1..at + 3]), 16).ok())
            .flatten();
        if let Some(byte) = escaped {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query of `url` (or a query), decoded, by name.
pub fn query_of(url: &str) -> BTreeMap<String, String> {
    pairs(url.split_once('?').map_or(url, |(_, query)| query))
}

fn pairs(text: &str) -> BTreeMap<String, String> {
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (decode(name), decode(value)))
        .collect()
}

fn respond(stream: &mut TcpStream, status: &str, body: &Value) {
    let body = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// The answer of a grant: a new key and the refresh token that renews it, which the proxy remembers as the only good one.
fn tokens(state: &mut State, client: &str) -> Value {
    state.issued += 1;
    let access = format!("sk-issued-{}", state.issued);
    let refresh = format!("refresh-{}", state.issued);
    state.keys.insert(access.clone());
    state.refresh = Some(refresh.clone());
    json!({"access_token": access, "token_type": "Bearer", "expires_in": 86_400,
        "refresh_token": refresh, "user_id": "jo@example.com", "team_id": null, "client": client})
}

fn handle(state: &Mutex<State>, address: &str, mut stream: TcpStream) {
    let Some((line, headers, body)) = read_request(&mut stream) else {
        return;
    };
    let mut parts = line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let credential = headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-api-key").map(String::as_str))
        .unwrap_or("")
        .to_owned();
    let mut state = lock(state);
    state.seen.push(Seen {
        line: format!("{method} {path}"),
        credential: credential.clone(),
    });
    let valid = state.keys.contains(&credential) && !state.expired.contains(&credential);
    match (method, path) {
        ("GET", "/.well-known/litellm-cli-auth") if state.sso => respond(
            &mut stream,
            "200 OK",
            &json!({"contract_version": 1, "issuer": address, "resource": address}),
        ),
        ("POST", "/register") if state.sso => {
            let asked: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let redirect = asked["redirect_uris"][0].as_str().unwrap_or("").to_owned();
            if asked["token_endpoint_auth_method"] != "none"
                || !redirect.starts_with("http://127.0.0.1:")
            {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"error": "invalid_client_metadata"}),
                );
            }
            let client = format!("client-{}", state.clients.len() + 1);
            state.clients.insert(client.clone(), redirect);
            respond(&mut stream, "201 Created", &json!({"client_id": client}));
        }
        ("GET", "/authorize") if state.sso => {
            let asked = pairs(query);
            let client = asked.get("client_id").cloned().unwrap_or_default();
            let registered = state.clients.get(&client).cloned();
            if asked.get("response_type").map(String::as_str) != Some("code")
                || asked.get("code_challenge_method").map(String::as_str) != Some("S256")
                || registered.as_deref() != asked.get("redirect_uri").map(String::as_str)
            {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"error": "invalid_request"}),
                );
            }
            let code = format!("code-{}", state.codes.len() + 1);
            let challenge = asked.get("code_challenge").cloned().unwrap_or_default();
            let redirect = registered.unwrap_or_default();
            state
                .codes
                .insert(code.clone(), (challenge, client, redirect.clone()));
            let there = format!(
                "{redirect}?code={code}&state={}",
                asked.get("state").cloned().unwrap_or_default()
            );
            respond(&mut stream, "200 OK", &json!({"redirect": there}));
        }
        ("POST", "/token") if state.sso => {
            let form = pairs(&body);
            let client = form.get("client_id").cloned().unwrap_or_default();
            match form.get("grant_type").map(String::as_str) {
                Some("authorization_code") => {
                    let code = form.get("code").cloned().unwrap_or_default();
                    // A code is good once.
                    let Some((challenge, owner, redirect)) = state.codes.remove(&code) else {
                        return respond(
                            &mut stream,
                            "400 Bad Request",
                            &json!({"error": "invalid_grant"}),
                        );
                    };
                    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                    if canoa_litellm::sso::challenge(&verifier) != challenge
                        || owner != client
                        || form.get("redirect_uri") != Some(&redirect)
                    {
                        return respond(
                            &mut stream,
                            "400 Bad Request",
                            &json!({"error": "invalid_grant"}),
                        );
                    }
                    let granted = tokens(&mut state, &client);
                    respond(&mut stream, "200 OK", &granted);
                }
                Some("refresh_token") => {
                    let used = form.get("refresh_token").cloned();
                    if used != state.refresh || !state.clients.contains_key(&client) {
                        return respond(
                            &mut stream,
                            "400 Bad Request",
                            &json!({"error": "invalid_grant"}),
                        );
                    }
                    let granted = tokens(&mut state, &client);
                    respond(&mut stream, "200 OK", &granted);
                }
                _ => respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"error": "unsupported_grant_type"}),
                ),
            }
        }
        ("GET", "/key/info") => {
            if !valid {
                return respond(
                    &mut stream,
                    "401 Unauthorized",
                    &json!({"error": "bad key"}),
                );
            }
            let spend = state.spend.get(&credential).copied().unwrap_or(0);
            let max_budget = state.max_budget;
            respond(
                &mut stream,
                "200 OK",
                &json!({"key": "hashed", "info": {
                    "spend": spend as f64 / 1_000_000.0, "max_budget": max_budget,
                    "budget_duration": "30d", "budget_reset_at": "2099-01-01T00:00:00Z",
                    "rpm_limit": 100, "tpm_limit": null, "models": ["claude-y", "gpt-x"]}}),
            );
        }
        ("GET", "/v1/models") => {
            if !valid {
                return respond(
                    &mut stream,
                    "401 Unauthorized",
                    &json!({"error": "bad key"}),
                );
            }
            respond(
                &mut stream,
                "200 OK",
                &json!({"object": "list", "data": [{"id": "claude-y"}, {"id": "gpt-x"}]}),
            );
        }
        ("POST", "/v1/messages" | "/v1/responses") => {
            if !valid {
                return respond(
                    &mut stream,
                    "401 Unauthorized",
                    &json!({"error": "bad key"}),
                );
            }
            // A request costs a cent, on the key whichever API it came in by.
            *state.spend.entry(credential).or_insert(0) += 10_000;
            respond(
                &mut stream,
                "200 OK",
                &json!({"id": "x", "type": "message", "content": []}),
            );
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            &json!({"error": "no such route"}),
        ),
    }
}

impl Proxy {
    /// A proxy that offers the CLI sign-in and one virtual key, [`VIRTUAL_KEY`], with a budget of ten dollars.
    pub fn start() -> Self {
        Self::start_with(true)
    }

    /// As [`Proxy::start`]; without `sso` it has no sign-in (an older proxy).
    pub fn start_with(sso: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        let address = format!("http://{}", listener.local_addr().expect("an address"));
        let state = Arc::new(Mutex::new(State {
            sso,
            max_budget: Some(10.0),
            keys: BTreeSet::from([VIRTUAL_KEY.to_owned()]),
            ..State::default()
        }));
        let (served, at) = (Arc::clone(&state), address.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (state, address) = (Arc::clone(&served), at.clone());
                std::thread::spawn(move || handle(&state, &address, stream));
            }
        });
        Self { address, state }
    }

    /// Every request so far, in order.
    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.state).seen.clone()
    }

    /// The requests made to `path`.
    pub fn to(&self, path: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|each| each.line.ends_with(&format!(" {path}")))
            .collect()
    }

    /// Sets what the proxy says the key's budget is, in dollars.
    pub fn budget(&self, dollars: Option<f64>) {
        lock(&self.state).max_budget = dollars;
    }

    /// Makes the key `key` spend `dollars` more, as the programs on it would.
    pub fn spend(&self, key: &str, dollars: f64) {
        let micro = (dollars * 1_000_000.0).round() as u64;
        *lock(&self.state).spend.entry(key.to_owned()).or_insert(0) += micro;
    }

    /// Makes the proxy refuse `key` from now on, as an expired key is.
    pub fn expire(&self, key: &str) {
        lock(&self.state).expired.insert(key.to_owned());
    }

    /// The proxy's own record of what the refresh token is, which a sign-in made and a renewal rotates.
    pub fn refresh_token(&self) -> Option<String> {
        lock(&self.state).refresh.clone()
    }

    /// What the person's browser does with the page a sign-in asked to open: the proxy signs them in at its `/authorize`, and the
    /// browser goes to the address that page redirects to. Nothing here is a real browser.
    pub fn browse(&self, page: &str) -> u16 {
        assert!(
            page.starts_with(&format!("{}/authorize?", self.address)),
            "{page}"
        );
        let (status, body) = get(page);
        assert_eq!(status, 200, "{body}");
        let redirect = serde_json::from_str::<Value>(&body).expect("JSON")["redirect"]
            .as_str()
            .expect("a redirect")
            .to_owned();
        get(&redirect).0
    }
}

/// `GET url` with nothing else, as a browser would: the status and the body.
pub fn get(url: &str) -> (u16, String) {
    let rest = url.strip_prefix("http://").expect("http");
    let (host, path) = rest
        .split_once('/')
        .map_or((rest, String::new()), |(host, path)| {
            (host, path.to_owned())
        });
    let mut stream = TcpStream::connect(host).expect("connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .expect("writes");
    let mut said = String::new();
    let _ = stream.read_to_string(&mut said);
    let status = said
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = said
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

/// `POST url` with `body` as JSON and `key` as the `x-api-key` or bearer, as a program does: the status.
pub fn post(url: &str, key: &str, bearer: bool) -> u16 {
    let rest = url.strip_prefix("http://").expect("http");
    let (host, path) = rest.split_once('/').expect("a path");
    let mut stream = TcpStream::connect(host).expect("connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let header = if bearer {
        format!("Authorization: Bearer {key}")
    } else {
        format!("x-api-key: {key}")
    };
    write!(
        stream,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\n{header}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )
    .expect("writes");
    let mut said = String::new();
    let _ = stream.read_to_string(&mut said);
    said.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}
