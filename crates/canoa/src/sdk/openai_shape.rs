//! The OpenAI-shaped client: chat completions, embeddings, rerank and the model list as an OpenAI-compatible server answers them,
//! and the `server` provider that offers it for any address the person names.
//!
//! A vendor whose service speaks this shape (OpenAI's own) builds [`OpenAiShaped`] with its own [`Dialect`] and model rule; the
//! `server` provider builds it with the capabilities the person says the server has.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use coqui_acp_api::{CapabilitiesSource, ModelInfo, ModelListing};
use guatiao_intake::Form;
use serde_json::{Value, json};

use super::{
    Capability, ChatReply, ChatRequest, EmbedReply, EmbedRequest, HttpReply, HttpRequest, LentHttp,
    OpenRequest, Ranked, RerankReply, RerankRequest, SIGNED_OUT_PREFIX, Sdk, SdkInfo, SdkProvider,
    capability,
};

/// The id of the `server` provider, which an offer of the person's own is made of (`[sdk.NAME] provider = "server"`).
pub const SERVER_ID: &str = "server";

/// How a service words the parts of the shape that vary.
#[derive(Debug, Clone, Copy)]
pub struct Dialect {
    /// The request key that caps the answer's tokens (`max_tokens`, or `max_completion_tokens` where the older one is refused).
    pub max_tokens_key: &'static str,
    /// Whether a model takes no temperature other than the default, so none is sent for it.
    pub fixed_temperature: fn(&str) -> bool,
}

impl Dialect {
    /// The widest-read dialect: `max_tokens` and a temperature for every model.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            max_tokens_key: "max_tokens",
            fixed_temperature: |_| false,
        }
    }
}

/// An OpenAI-shaped client over a lent address.
pub struct OpenAiShaped {
    http: Object<dyn LentHttp>,
    prefix: &'static str,
    dialect: Dialect,
    rule: Option<fn(ModelInfo) -> ModelInfo>,
    declared: Vec<String>,
}

impl core::fmt::Debug for OpenAiShaped {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OpenAiShaped")
            .field("prefix", &self.prefix)
            .field("declared", &self.declared)
            .finish_non_exhaustive()
    }
}

impl OpenAiShaped {
    /// A client over `http`, bound to `address`: paths start with `/v1` unless the address already ends with it.
    #[must_use]
    pub fn new(http: Object<dyn LentHttp>, address: &str) -> Self {
        let prefix = if address.trim_end_matches('/').ends_with("/v1") {
            ""
        } else {
            "/v1"
        };
        Self {
            http,
            prefix,
            dialect: Dialect::plain(),
            rule: None,
            declared: Vec::new(),
        }
    }

    /// The same client in `dialect`.
    #[must_use]
    pub fn dialect(mut self, dialect: Dialect) -> Self {
        self.dialect = dialect;
        self
    }

    /// The same client where each listed model is passed through `rule`, which says what the service's list does not.
    #[must_use]
    pub fn rule(mut self, rule: fn(ModelInfo) -> ModelInfo) -> Self {
        self.rule = Some(rule);
        self
    }

    /// The same client where each listed model is said to have `capabilities`, because the person said so.
    #[must_use]
    pub fn declared(mut self, capabilities: Vec<String>) -> Self {
        self.declared = capabilities;
        self
    }

    fn call(&self, request: HttpRequest) -> Result<Value, ProviderError> {
        let path = request.path.clone();
        let reply: HttpReply = self.http.request(request)?;
        if !(200..300).contains(&reply.status) {
            let said: String = reply.body.chars().take(200).collect();
            let why = alloc::format!("{path} answered {}: {said}", reply.status);
            return Err(if matches!(reply.status, 401 | 403) {
                crate::status::internal(alloc::format!("{SIGNED_OUT_PREFIX}{why}"))
            } else {
                crate::status::internal(why)
            });
        }
        serde_json::from_str(&reply.body).map_err(|error| {
            crate::status::internal(alloc::format!("{path}: the answer is not JSON: {error}"))
        })
    }

    fn path(&self, tail: &str) -> String {
        alloc::format!("{}{tail}", self.prefix)
    }
}

fn bad(why: impl AsRef<str>) -> ProviderError {
    crate::status::internal(why)
}

impl Sdk for OpenAiShaped {
    fn models(&self) -> Result<ModelListing, ProviderError> {
        let value = self.call(HttpRequest::get(self.path("/models")))?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("the model list has no `data` list"))?;
        let mut models = Vec::new();
        for item in data {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                continue;
            };
            let mut model = ModelInfo::new(id);
            if !self.declared.is_empty() {
                model = model.with_capabilities(
                    self.declared.iter().map(String::as_str),
                    CapabilitiesSource::Declared,
                );
            }
            models.push(match self.rule {
                Some(rule) => rule(model),
                None => model,
            });
        }
        Ok(ModelListing::of(models))
    }

    fn chat(&self, request: ChatRequest) -> Result<ChatReply, ProviderError> {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        for message in &request.messages {
            messages.push(json!({"role": message.role, "content": message.content}));
        }
        let mut body = json!({"model": request.model, "messages": messages, "stream": false});
        if let Some(tokens) = request.max_tokens {
            body[self.dialect.max_tokens_key] = json!(tokens);
        }
        if let Some(temperature) = request.temperature
            && !(self.dialect.fixed_temperature)(&request.model)
        {
            body["temperature"] = json!(temperature);
        }
        if request.json {
            body["response_format"] = json!({"type": "json_object"});
        }
        let value = self.call(HttpRequest::post_json(
            self.path("/chat/completions"),
            body.to_string(),
        ))?;
        let text = value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("the answer has no message"))?;
        let mut reply = ChatReply::new(text);
        reply.input_tokens = value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64);
        reply.output_tokens = value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64);
        Ok(reply)
    }

    fn embed(&self, request: EmbedRequest) -> Result<EmbedReply, ProviderError> {
        let mut body = json!({"model": request.model, "input": request.inputs});
        if let Some(dimensions) = request.dimensions {
            body["dimensions"] = json!(dimensions);
        }
        let value = self.call(HttpRequest::post_json(
            self.path("/embeddings"),
            body.to_string(),
        ))?;
        let vectors = vectors_of(&value, request.inputs.len())?;
        let mut reply = EmbedReply::new(vectors);
        reply.input_tokens = value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64);
        Ok(reply)
    }

    fn rerank(&self, request: RerankRequest) -> Result<RerankReply, ProviderError> {
        let mut body = json!({
            "model": request.model,
            "query": request.query,
            "documents": request.passages,
        });
        if let Some(top) = request.top {
            body["top_n"] = json!(top);
        }
        let value = self.call(HttpRequest::post_json(
            self.path("/rerank"),
            body.to_string(),
        ))?;
        ranked_of(&value, request.passages.len())
    }
}

/// The vectors an embeddings answer holds, ordered by their `index`.
fn vectors_of(value: &Value, expected: usize) -> Result<Vec<Vec<f32>>, ProviderError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("the answer has no `data` list"))?;
    if data.len() != expected {
        return Err(bad(alloc::format!(
            "{} vectors for {expected} inputs",
            data.len()
        )));
    }
    let mut slots: Vec<Option<Vec<f32>>> = alloc::vec![None; expected];
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
            .ok_or_else(|| bad("an item has no `embedding` list"))?
            .iter()
            .map(|number| {
                #[allow(clippy::cast_possible_truncation)]
                number.as_f64().map(|number| number as f32)
            })
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| bad("an embedding holds a value that is not a number"))?;
        let slot = slots
            .get_mut(index)
            .ok_or_else(|| bad(alloc::format!("an item's index {index} is out of range")))?;
        if slot.replace(vector).is_some() {
            return Err(bad(alloc::format!("two items have index {index}")));
        }
    }
    Ok(slots.into_iter().flatten().collect())
}

/// The order a rerank answer gives: a list of `{index, relevance_score}` under `results` or `data`, best first.
fn ranked_of(value: &Value, count: usize) -> Result<RerankReply, ProviderError> {
    let list = value
        .as_array()
        .or_else(|| {
            ["results", "data", "ranking", "rankings"]
                .iter()
                .find_map(|key| value.get(key).and_then(Value::as_array))
        })
        .ok_or_else(|| bad("the answer has no list of results"))?;
    let mut scored: Vec<Ranked> = Vec::new();
    for (position, item) in list.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok())
            .or_else(|| u32::try_from(position).ok())
            .ok_or_else(|| bad("a result has no index"))?;
        if usize::try_from(index).unwrap_or(usize::MAX) >= count {
            return Err(bad(alloc::format!(
                "a result's index {index} is out of range"
            )));
        }
        let score = ["relevance_score", "score", "relevance"]
            .iter()
            .find_map(|key| item.get(key).and_then(Value::as_f64))
            .or_else(|| item.as_f64())
            .ok_or_else(|| bad("a result has no score"))?;
        scored.push(Ranked::new(index, score));
    }
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(RerankReply::new(scored))
}

/// What the person says of a server by address: the operations it has. The address and the key variable are the host's.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct ServerSettings {
    /// The operations the server has, from `chat`, `embed` and `rerank`.
    #[schema(title = "What the server does")]
    #[map(default)]
    pub capabilities: Vec<String>,
}

/// Any server by address that speaks the OpenAI shape.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(SdkProvider, id = "server", crate = ::coqui_acp_api::guatiao)]
pub struct Server;

impl SdkProvider for Server {
    fn describe(&self) -> Result<SdkInfo, ProviderError> {
        let mut info = SdkInfo::new(SERVER_ID, "A server by address").taking::<ServerSettings>()?;
        info.summary = "Any server that speaks the OpenAI API, at an address you give.".to_owned();
        info.location =
            "The text goes to the address you gave, and whoever runs it sees it.".to_owned();
        info.capabilities = [capability::CHAT, capability::EMBED, capability::RERANK]
            .into_iter()
            .map(Capability::new)
            .collect();
        Ok(info)
    }

    fn open(
        &self,
        request: OpenRequest,
        http: Object<dyn LentHttp>,
    ) -> Result<Object<dyn Sdk>, ProviderError> {
        let settings: ServerSettings = if request.settings.trim().is_empty() {
            ServerSettings::default()
        } else {
            let json: Value = serde_json::from_str(&request.settings)
                .map_err(|error| crate::status::bad_value(error.to_string()))?;
            let capabilities = json
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            ServerSettings { capabilities }
        };
        Ok(OpenAiShaped::new(http, &request.address)
            .declared(settings.capabilities)
            .into_object())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_come_back_in_the_order_of_their_index_and_are_checked() {
        let value: Value = serde_json::from_str(
            r#"{"data":[{"index":1,"embedding":[3,4]},{"index":0,"embedding":[1,2]}]}"#,
        )
        .unwrap();
        assert_eq!(
            vectors_of(&value, 2).unwrap(),
            vec![vec![1.0, 2.0], vec![3.0, 4.0]]
        );
        assert!(vectors_of(&value, 3).is_err());
        assert!(vectors_of(&json!({}), 1).is_err());
    }

    #[test]
    fn a_rerank_answer_is_read_under_the_usual_names_best_first() {
        let value = json!({"results": [
            {"index": 0, "relevance_score": 0.1}, {"index": 2, "relevance_score": 0.9}]});
        let reply = ranked_of(&value, 3).unwrap();
        assert_eq!(reply.results[0].index, 2);
        assert!(
            ranked_of(&value, 2).is_err(),
            "an index out of range is refused"
        );
        assert!(ranked_of(&json!({"x": 1}), 1).is_err());
    }
}
