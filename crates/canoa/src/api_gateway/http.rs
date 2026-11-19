//! A request to a gateway, through the person's own network.
//!
//! The request goes the way every other request of the host does (the system's proxy and its login, the operating system's
//! certificate verifier, the person's overrides: [`crate::network::Network::current`]), never follows a redirect, so a key is shown
//! to the address it was given for and no other, and answers the status and the JSON the address sent, however it was refused. What
//! it says of a failure holds no key and no query.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a connection is std's"
)]

use std::io::Read as _;
use std::time::Duration;

use serde_json::Value;

use crate::network::Network;

/// How long one request may take, from the first byte sent to the last read.
pub const WAIT: Duration = Duration::from_secs(30);

/// The most of an answer that is read: more is cut, and is no JSON.
const ANSWER_LIMIT: u64 = 8 * 1024 * 1024;

/// What an address answered.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The body as JSON; `null` when it was empty or not JSON.
    pub body: Value,
}

/// An agent for one request: no redirect, no error for a status, [`WAIT`] at most, through the person's network.
///
/// # Errors
///
/// The network's own settings cannot be applied.
pub fn agent() -> Result<ureq::Agent, String> {
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(WAIT))
        .build();
    Network::current()
        .agent(config)
        .map_err(|error| format!("the network could not be set up: {error}"))
}

fn read(response: ureq::http::Response<ureq::Body>) -> Result<Reply, String> {
    let status = response.status().as_u16();
    let mut text = String::new();
    response
        .into_body()
        .into_reader()
        .take(ANSWER_LIMIT)
        .read_to_string(&mut text)
        .map_err(|error| format!("the answer was cut off: {error}"))?;
    Ok(Reply {
        status,
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
    })
}

fn failed(error: &ureq::Error) -> String {
    format!("the address did not answer: {error}")
}

/// `GET url`, with `Authorization: Bearer KEY` when a key is given.
///
/// # Errors
///
/// The address did not answer in time, refused the connection, or sent an answer that was cut off. A status is not an error.
pub fn get_json(url: &str, bearer: Option<&str>) -> Result<Reply, String> {
    let agent = agent()?;
    let mut request = agent.get(url).header("Accept", "application/json");
    if let Some(key) = bearer {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    read(request.call().map_err(|error| failed(&error))?)
}

/// `POST url` with `fields` as a form (`application/x-www-form-urlencoded`).
///
/// # Errors
///
/// As [`get_json`].
pub fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<Reply, String> {
    let agent = agent()?;
    let body = fields
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(name, FORM),
                percent_encoding::utf8_percent_encode(value, FORM)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let request = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded");
    read(request.send(body).map_err(|error| failed(&error))?)
}

/// `POST url` with `body` as JSON.
///
/// # Errors
///
/// As [`get_json`].
pub fn post_json(url: &str, body: &Value) -> Result<Reply, String> {
    let agent = agent()?;
    let request = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");
    read(
        request
            .send(body.to_string())
            .map_err(|error| failed(&error))?,
    )
}

/// What a form value escapes: everything but the unreserved characters.
const FORM: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
