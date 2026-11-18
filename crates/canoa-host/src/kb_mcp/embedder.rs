//! The embeddings endpoint the person names (`[host.kb] embeddings`): an OpenAI-compatible `/v1/embeddings`, reached through
//! [`Network`] like every other fetch, so a server beyond the system's proxy is found as it would be from a browser.

use alloc::sync::Arc;
use core::time::Duration;

use canoa::network::Network;
use coqui_aos::index::{Embedder, Error};
use serde_json::{Value, json};

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(120);
/// The most bytes read from an answer.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

/// An OpenAI-compatible embeddings endpoint.
#[derive(Clone, Debug)]
pub(crate) struct NetworkEmbedder {
    url: String,
    model: String,
    bearer: Option<String>,
    /// The network a session's requests go through; the process's when none.
    network: Option<Arc<Network>>,
}

impl NetworkEmbedder {
    /// The endpoint of the server at `base` (`http://127.0.0.1:60570`, with or without a trailing `/v1`) for `model`, the exact id
    /// the server loaded.
    pub(crate) fn new(base: &str, model: impl Into<String>) -> Self {
        let base = base.trim_end_matches('/');
        let url = if base.ends_with("/v1") {
            format!("{base}/embeddings")
        } else {
            format!("{base}/v1/embeddings")
        };
        Self {
            url,
            model: model.into(),
            bearer: None,
            network: None,
        }
    }

    /// The same endpoint reached through `network`, as one session's profile chose it; the process's when `None`.
    pub(crate) fn on(mut self, network: Option<Arc<Network>>) -> Self {
        self.network = network;
        self
    }

    /// Sends `token` as the request's bearer, for an endpoint that asks for a key.
    pub(crate) fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
    }

    fn failed(&self, why: impl core::fmt::Display) -> Error {
        Error::Embed(format!("{}: {why}", self.url))
    }
}

impl Embedder for NetworkEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, Error> {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .build();
        let agent = self
            .network
            .clone()
            .unwrap_or_else(Network::current)
            .agent(config)
            .map_err(|error| self.failed(error))?;
        let body = json!({"model": self.model, "input": inputs}).to_string();
        let mut request = agent
            .post(&self.url)
            .header("content-type", "application/json");
        if let Some(token) = &self.bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = request
            .send(body.as_str())
            .map_err(|error| self.failed(error))?;
        let text = response
            .into_body()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
            .map_err(|error| self.failed(error))?;
        parse(&text, inputs.len())
    }
}

/// The vectors an embeddings response holds, ordered by their `index`.
fn parse(text: &str, expected: usize) -> Result<Vec<Vec<f32>>, Error> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| Error::Embed(format!("the response is not JSON: {error}")))?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Embed("the response has no `data` list".into()))?;
    if data.len() != expected {
        return Err(Error::Embed(format!(
            "{} vectors for {expected} inputs",
            data.len()
        )));
    }
    let mut slots: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (position, item) in data.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map_or(position, |index| {
                usize::try_from(index).unwrap_or(usize::MAX)
            });
        let vector = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Embed("an item has no `embedding` list".into()))?
            .iter()
            .map(|number| {
                #[allow(clippy::cast_possible_truncation)]
                number.as_f64().map(|number| number as f32)
            })
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| {
                Error::Embed("an embedding holds a value that is not a number".into())
            })?;
        let slot = slots
            .get_mut(index)
            .ok_or_else(|| Error::Embed(format!("an item's index {index} is out of range")))?;
        if slot.replace(vector).is_some() {
            return Err(Error::Embed(format!("two items have index {index}")));
        }
    }
    Ok(slots.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_takes_a_base_with_or_without_v1() {
        assert_eq!(
            NetworkEmbedder::new("http://127.0.0.1:1/", "m").url,
            "http://127.0.0.1:1/v1/embeddings"
        );
        assert_eq!(
            NetworkEmbedder::new("http://127.0.0.1:1/v1", "m").url,
            "http://127.0.0.1:1/v1/embeddings"
        );
    }

    #[test]
    fn a_response_is_ordered_by_index_and_checked() {
        let text = r#"{"data":[{"index":1,"embedding":[3,4]},{"index":0,"embedding":[1,2]}]}"#;
        assert_eq!(
            parse(text, 2).unwrap(),
            vec![vec![1.0, 2.0], vec![3.0, 4.0]]
        );
        assert!(parse(text, 3).is_err());
        assert!(parse("{}", 1).is_err());
    }
}
