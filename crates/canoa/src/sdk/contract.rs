//! The contract every `sdk_provider` passes: it describes itself in a way the host can offer, does every operation it describes
//! against recorded answers, asks for nothing outside the address it was lent and never carries a credential in a request.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::testing::{Recorded, Route};
use super::{
    ChatRequest, EmbedRequest, LentHttp as _, OpenRequest, RerankRequest, SdkProvider, capability,
};

/// The recorded answers a provider is held to, and the model each operation names.
#[derive(Debug, Clone)]
pub struct Fixtures {
    /// The address the provider is opened over.
    pub address: String,
    /// The provider's own settings as a JSON object.
    pub settings: String,
    /// The recorded answers: one for the model list and one for each operation the provider describes.
    pub routes: Vec<Route>,
    /// The model the operations name.
    pub model: String,
}

/// Runs the contract on `provider`.
///
/// # Errors
///
/// The first thing the provider does that the contract forbids, in words.
pub fn run(provider: &dyn SdkProvider, fixtures: &Fixtures) -> Result<(), String> {
    let info = provider
        .describe()
        .map_err(|error| format!("describe: {error}"))?;
    if let Some(why) = info.problem() {
        return Err(format!("the description cannot be offered: {why}"));
    }
    let (http, asked) = Recorded::new(fixtures.routes.clone());
    let sdk = provider
        .open(
            OpenRequest::new(&fixtures.address, &fixtures.settings),
            http.into_object(),
        )
        .map_err(|error| format!("open: {error}"))?;
    let listing = sdk.models().map_err(|error| format!("models: {error}"))?;
    if !listing.known {
        return Err("the model list is not known".to_owned());
    }
    for (at, model) in listing.models.iter().enumerate() {
        if listing.models[..at].iter().any(|seen| seen.id == model.id) {
            return Err(format!("the model {} is listed twice", model.id));
        }
    }
    let model = fixtures.model.as_str();
    if info.can(capability::CHAT) {
        let reply = sdk
            .chat(ChatRequest::ask(model, "hello"))
            .map_err(|error| format!("chat: {error}"))?;
        if reply.text.is_empty() {
            return Err("chat answered no text".to_owned());
        }
    }
    if info.can(capability::EMBED) {
        let inputs = alloc::vec!["a".to_owned(), "b".to_owned()];
        let reply = sdk
            .embed(EmbedRequest::new(model, inputs))
            .map_err(|error| format!("embed: {error}"))?;
        if reply.vectors.len() != 2 || reply.vectors[0].len() != reply.vectors[1].len() {
            return Err("embed did not answer one vector of one length per input".to_owned());
        }
    }
    if info.can(capability::RERANK) {
        let passages = alloc::vec!["a".to_owned(), "b".to_owned()];
        let reply = sdk
            .rerank(RerankRequest::new(model, "q", passages))
            .map_err(|error| format!("rerank: {error}"))?;
        if reply.results.is_empty() {
            return Err("rerank answered no result".to_owned());
        }
    }
    for request in asked.requests() {
        if !request.path.starts_with('/') || request.path.contains("://") {
            return Err(format!(
                "a request names {:?}, which is not a path below the address",
                request.path
            ));
        }
        for header in &request.headers {
            let name = header.name.to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "authorization" | "x-api-key" | "api-key" | "cookie"
            ) {
                return Err(format!(
                    "a request carries the header {name}: the host applies the key"
                ));
            }
        }
    }
    Ok(())
}
