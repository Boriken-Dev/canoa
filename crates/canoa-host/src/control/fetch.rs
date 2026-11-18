//! `network/fetch`: one HTTP request the app makes, sent from this host so it follows the host's network (the system's proxy and its
//! login, the operating system's verifier, the client certificate of its keystores, `[host.network]`) and not the app's own library.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "bodies and clocks are std's"
)]

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use canoa::network::Network;
use data_encoding::BASE64;
use serde_json::{Value, json};

use super::{REFUSED, bad, text_param};

type Outcome = Result<Value, (i32, String)>;

/// The most bytes of a reply's body that are returned.
pub(crate) const MAX_REPLY_BYTES: u64 = 16 * 1024 * 1024;
/// The most bytes of a request's body.
pub(crate) const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
/// How long a request has to complete unless it says otherwise.
const DEFAULT_SECONDS: u64 = 30;
/// The longest a request may ask for.
const MAX_SECONDS: u64 = 300;

/// The requests the app sends through this host.
#[derive(Clone, Default)]
pub(crate) struct Fetch {
    /// The network they go through; the process's when `None`.
    network: Option<Arc<Network>>,
}

impl core::fmt::Debug for Fetch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fetch").finish_non_exhaustive()
    }
}

impl Fetch {
    /// Sends through `network` instead of the process's.
    #[cfg(test)]
    pub(crate) fn on(network: Arc<Network>) -> Self {
        Self {
            network: Some(network),
        }
    }

    /// `network/fetch` `{"url", "method"?, "headers"?: [[name, value]], "body"?: base64, "seconds"?}` ->
    /// `{"status", "headers": [[name, value]], "body": base64}`. Any status is an answer; only a request that got none is refused.
    pub(crate) fn run(&self, params: &Value) -> Outcome {
        let url = text_param(params, "url")?.ok_or_else(|| bad("url is needed"))?;
        let scheme = url.split_once("://").map(|(scheme, _)| scheme);
        if !matches!(scheme, Some(s) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
        {
            return Err(bad("url is http or https"));
        }
        let method = text_param(params, "method")?.unwrap_or("GET");
        let method = ureq::http::Method::from_bytes(method.to_ascii_uppercase().as_bytes())
            .map_err(|_| bad("method is an HTTP method"))?;
        let seconds = match params.get("seconds") {
            None | Some(Value::Null) => DEFAULT_SECONDS,
            Some(value) => value
                .as_u64()
                .filter(|seconds| (1..=MAX_SECONDS).contains(seconds))
                .ok_or_else(|| bad("seconds is 1 to 300"))?,
        };
        let body = match text_param(params, "body")? {
            None => Vec::new(),
            Some(text) => BASE64
                .decode(text.as_bytes())
                .map_err(|_| bad("body is base64"))?,
        };
        if body.len() > MAX_REQUEST_BYTES {
            return Err(bad("body is larger than a request may carry"));
        }

        let mut request = ureq::http::Request::builder().method(method).uri(url);
        if let Some(headers) = params.get("headers") {
            let pairs = headers
                .as_array()
                .ok_or_else(|| bad("headers is a list of [name, value]"))?;
            for pair in pairs {
                let (Some(name), Some(value)) = (
                    pair.get(0).and_then(Value::as_str),
                    pair.get(1).and_then(Value::as_str),
                ) else {
                    return Err(bad("headers is a list of [name, value]"));
                };
                request = request.header(name, value);
            }
        }
        let request = request
            .body(body)
            .map_err(|error| bad(&format!("the request is not valid: {error}")))?;

        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(seconds)))
            .build();
        let network = self.network.clone().unwrap_or_else(Network::current);
        let agent = network
            .agent(config)
            .map_err(|error| (REFUSED, format!("The network cannot be used: {error}")))?;
        let mut response = agent
            .run(request)
            .map_err(|error| (REFUSED, format!("The request got no answer: {error}")))?;
        let status = response.status().as_u16();
        let headers: Vec<Value> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| json!([name.as_str(), value]))
            })
            .collect();
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_REPLY_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|error| {
                (
                    REFUSED,
                    format!("The answer did not finish arriving: {error}"),
                )
            })?;
        if body.len() as u64 > MAX_REPLY_BYTES {
            return Err((
                REFUSED,
                "The answer is larger than the host passes on".to_owned(),
            ));
        }
        Ok(json!({"status": status, "headers": headers, "body": BASE64.encode(&body)}))
    }
}
