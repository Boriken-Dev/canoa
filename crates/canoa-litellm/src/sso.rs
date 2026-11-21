//! The proxy's own sign-in, as LiteLLM's CLI does it with `--pkce`: the person's browser signs in at the proxy (its SSO), and the proxy
//! hands the key back to an address on this computer.
//!
//! The steps are the proxy's documented browser flow (OAuth 2.0 authorization code with PKCE and no client secret): the proxy's
//! discovery document, a client registered with a loopback redirect, the proxy's `/authorize` page opened for the person, the code it
//! sends to the loopback address, and `/token`; a refresh grant renews the key. Every request goes through the person's network and
//! follows no redirect. What a failure says holds no code, token or key.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a loopback listener and a clock are std's"
)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use canoa::api_gateway::http::{self, Reply};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{LoginPrompter, OpenUrl};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// Where the proxy says it serves the sign-in (LiteLLM's documented path).
pub const DISCOVERY: &str = "/.well-known/litellm-cli-auth";

/// The path the loopback address answers the proxy's redirect on.
const CALLBACK: &str = "/callback";

/// What the person's browser is told when the code has arrived.
const DONE_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Canoa</title><body style=\"font-family:sans-serif\"><p>Signed in. You can close this page and go back to Canoa.</p>";

/// What a sign-in or a renewal made.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Tokens {
    /// The key a program sends.
    pub access_token: String,
    /// What renews it, when the proxy gave one.
    pub refresh_token: Option<String>,
    /// How long the key lasts, in seconds, when the proxy said.
    pub expires_in: Option<u64>,
    /// The person the proxy signed in, when it said.
    pub user_id: Option<String>,
}

impl core::fmt::Debug for Tokens {
    /// Names only: every field but the person is a credential.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tokens")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

fn failure(status: Status, why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(status, why.as_ref())
}

fn internal(why: impl AsRef<str>) -> ProviderError {
    failure(Status::GUATIAO_ERR_INTERNAL, why)
}

fn base64url(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

/// A random text of `bytes` bytes of randomness, in the characters a URL carries as they are.
fn random_text(bytes: usize) -> Result<String, ProviderError> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random)
        .map_err(|_| internal("the system has no randomness to sign in with"))?;
    Ok(base64url(&random))
}

/// The PKCE challenge of `verifier`: the unpadded URL-safe base64 of its SHA-256.
#[must_use]
pub fn challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// What a query value escapes: everything but the unreserved characters.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn escaped(text: &str) -> String {
    utf8_percent_encode(text, QUERY).to_string()
}

/// What a proxy said when it refused: its `error` word and its status, never more.
fn refused_by(step: &str, reply: &Reply) -> String {
    let word = reply
        .body
        .get("error")
        .and_then(Value::as_str)
        .filter(|word| word.len() <= 64 && word.chars().all(|c| c.is_ascii_graphic()));
    match word {
        Some(word) => format!("the proxy refused {step} ({}): {word}", reply.status),
        None => format!("the proxy refused {step} ({})", reply.status),
    }
}

fn tokens_of(reply: &Reply) -> Result<Tokens, String> {
    let text = |key: &str| {
        reply
            .body
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let access_token =
        text("access_token").ok_or_else(|| "the proxy's answer holds no key".to_owned())?;
    Ok(Tokens {
        access_token,
        refresh_token: text("refresh_token"),
        expires_in: reply.body.get("expires_in").and_then(Value::as_u64),
        user_id: text("user_id"),
    })
}

/// The whole text of the query `name` in `query`, decoded.
fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| {
            percent_encoding::percent_decode_str(&value.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        })
    })
}

/// What the browser's request to the loopback address asked for: the path and its query.
fn requested(stream: &mut TcpStream) -> Option<(String, String)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
        if stream.read(&mut byte).ok()? == 0 {
            break;
        }
        head.push(byte[0]);
    }
    let line = String::from_utf8_lossy(&head);
    let target = line
        .lines()
        .next()?
        .strip_prefix("GET ")?
        .split(' ')
        .next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Some((path.to_owned(), query.to_owned()))
}

fn answer(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Waits on `listener` for the redirect that carries the code, for the `state` given: the code, or why not.
fn await_code(
    listener: &TcpListener,
    state: &str,
    prompter: &Object<dyn LoginPrompter>,
    deadline: Instant,
) -> Result<String, ProviderError> {
    loop {
        if prompter.cancelled().unwrap_or(false) {
            return Err(failure(Status::GUATIAO_ERR_GONE, "login cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(failure(Status::GUATIAO_ERR_GONE, "login timed out"));
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => return Err(internal("the address that waits for the proxy failed")),
        };
        let _ = stream.set_nonblocking(false);
        let Some((path, query)) = requested(&mut stream) else {
            continue;
        };
        if path != CALLBACK {
            answer(&mut stream, "404 Not Found", "");
            continue;
        }
        // A request that does not carry this sign-in's state is somebody else's: it never ends the wait.
        if query_value(&query, "state").as_deref() != Some(state) {
            answer(&mut stream, "400 Bad Request", "");
            continue;
        }
        if let Some(error) = query_value(&query, "error") {
            answer(
                &mut stream,
                "200 OK",
                "<!doctype html><meta charset=\"utf-8\"><p>The sign-in was refused. Go back to Canoa.",
            );
            let word: String = error
                .chars()
                .filter(char::is_ascii_graphic)
                .take(64)
                .collect();
            return Err(failure(
                Status::GUATIAO_ERR_BAD_VALUE,
                format!("login refused: the proxy's sign-in said {word}"),
            ));
        }
        let Some(code) = query_value(&query, "code").filter(|code| !code.is_empty()) else {
            answer(&mut stream, "400 Bad Request", "");
            continue;
        };
        answer(&mut stream, "200 OK", DONE_PAGE);
        return Ok(code);
    }
}

/// Signs in at the proxy `base` through the person's browser: `prompter` is asked to open the proxy's page, and the key comes back
/// with the client it was registered for (which the refresh token belongs to). Ends at `deadline`, or when the person cancels.
///
/// # Errors
///
/// `GONE` when the person cancels or the time runs out; `BAD_VALUE` when the proxy refuses the sign-in; `INTERNAL` with the reason
/// when the proxy offers no such sign-in, does not answer, or answers what is not the flow. No reason holds a code or a key.
pub fn sign_in(
    base: &str,
    prompter: &Object<dyn LoginPrompter>,
    deadline: Instant,
) -> Result<(Tokens, String), ProviderError> {
    let found = http::get_json(&format!("{base}{DISCOVERY}"), None)
        .map_err(|why| internal(why.to_string()))?;
    if found.status != 200 {
        return Err(internal(format!(
            "the proxy offers no sign-in of this kind ({}): LiteLLM's CLI sign-in is a beta the proxy has to turn on and be recent enough for; use a key instead",
            found.status
        )));
    }
    let resource = found
        .body
        .get("resource")
        .and_then(Value::as_str)
        .map_or_else(|| base.to_owned(), str::to_owned);
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| internal("no address on this computer to receive the proxy's answer"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| internal("no address on this computer to receive the proxy's answer"))?;
    let port = listener
        .local_addr()
        .map_err(|_| internal("no address on this computer to receive the proxy's answer"))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}{CALLBACK}");
    let registered = http::post_json(
        &format!("{base}/register"),
        &json!({
            "client_name": "Canoa",
            "redirect_uris": [redirect],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        }),
    )
    .map_err(|why| internal(why.to_string()))?;
    if !(200..300).contains(&registered.status) {
        return Err(internal(refused_by(
            "to register this application",
            &registered,
        )));
    }
    let client = registered
        .body
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| internal("the proxy registered no client"))?
        .to_owned();
    let verifier = random_text(32)?;
    let state = random_text(16)?;
    let page = format!(
        "{base}/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&resource={}",
        escaped(&client),
        escaped(&redirect),
        escaped(&state),
        escaped(&challenge(&verifier)),
        escaped(&resource),
    );
    let mut open = OpenUrl::new(page);
    open.expires_ms = Some(
        u64::try_from(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(u64::MAX),
    );
    prompter.open_url(open)?;
    let code = await_code(&listener, &state, prompter, deadline)?;
    let granted = http::post_form(
        &format!("{base}/token"),
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &client),
            ("code_verifier", &verifier),
            ("resource", &resource),
        ],
    )
    .map_err(|why| internal(why.to_string()))?;
    if !(200..300).contains(&granted.status) {
        return Err(failure(
            Status::GUATIAO_ERR_BAD_VALUE,
            format!("login refused: {}", refused_by("the code", &granted)),
        ));
    }
    Ok((tokens_of(&granted).map_err(internal)?, client))
}

/// Why a renewal made no key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenewError {
    /// The proxy refused the refresh token (used, expired or revoked): only a new sign-in makes a key.
    Refused(String),
    /// The proxy could not be asked, or answered what is not the flow: the same token may be tried again.
    Failed(String),
}

/// Renews a key: the proxy's refresh grant for `refresh_token`, which it rotates, so the answer holds the next one.
///
/// # Errors
///
/// [`RenewError::Refused`] when the proxy turns the token down; [`RenewError::Failed`] when it could not be asked.
pub fn refresh(base: &str, client: &str, refresh_token: &str) -> Result<Tokens, RenewError> {
    let found = http::get_json(&format!("{base}{DISCOVERY}"), None)
        .map_err(|why| RenewError::Failed(why.to_string()))?;
    let resource = found
        .body
        .get("resource")
        .and_then(Value::as_str)
        .map_or_else(|| base.to_owned(), str::to_owned);
    let reply = http::post_form(
        &format!("{base}/token"),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client),
            ("resource", &resource),
        ],
    )
    .map_err(|why| RenewError::Failed(why.to_string()))?;
    match reply.status {
        200..=299 => tokens_of(&reply).map_err(RenewError::Failed),
        400 | 401 | 403 => Err(RenewError::Refused(refused_by("the refresh token", &reply))),
        _ => Err(RenewError::Failed(refused_by("to renew the key", &reply))),
    }
}

/// `seconds` from now, as an RFC 3339 time in UTC.
#[must_use]
pub fn expires_at(seconds: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    rfc3339(now.saturating_add(seconds))
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn rfc3339(unix: u64) -> String {
    let ts = i64::try_from(unix).unwrap_or(i64::MAX);
    let dt = time::OffsetDateTime::from_unix_timestamp(ts)
        .unwrap_or_else(|_| time::PrimitiveDateTime::MAX.assume_utc());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_s256_of_the_verifier() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_time_is_written_as_rfc_3339_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
        let parsed = time::OffsetDateTime::parse(
            &rfc3339(1_234_567_890),
            &time::format_description::well_known::Rfc3339,
        )
        .expect("it parses back");
        assert_eq!(parsed.unix_timestamp(), 1_234_567_890);
    }

    #[test]
    fn a_query_is_read_by_name_and_decoded() {
        let query = "code=a%2Fb+c&state=xyz&error=";
        assert_eq!(query_value(query, "code").as_deref(), Some("a/b c"));
        assert_eq!(query_value(query, "state").as_deref(), Some("xyz"));
        assert_eq!(query_value(query, "error").as_deref(), Some(""));
        assert_eq!(query_value(query, "missing"), None);
    }

    #[test]
    fn what_a_refusal_says_holds_a_word_and_a_status_and_nothing_else() {
        let reply = Reply::new(
            400,
            json!({"error": "invalid_grant", "error_description": "the code CANARY was used"}),
        );
        let said = refused_by("the code", &reply);
        assert_eq!(said, "the proxy refused the code (400): invalid_grant");
        let odd = Reply::new(500, json!({"error": "has spaces and a secret"}));
        assert_eq!(refused_by("it", &odd), "the proxy refused it (500)");
    }
}
