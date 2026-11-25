//! The `sdk_provider` kind: a vendor's client for an account type.
//!
//! The host holds no vendor's address, request shape, model or capability: a provider says them. It `describe`s itself (the
//! credential standards of the accounts it serves, how the host applies the key, the operations it implements) and `open`s an
//! [`Sdk`] object. The provider never sees the key and never opens a socket: every request goes through a [`LentHttp`] the host
//! gives it, which is bound to one address, applies the credential, goes through the person's network and refuses to leave the
//! address. That keeps "an account's key would leave it" in one place, and lets a script provider be an SDK.
//!
//! Nothing calls a provider directly: the host's `assist` service asks for an operation and fulfils it from whatever can.
//! The models are plain data, the same in every language.

pub mod contract;
mod openai_shape;
pub mod testing;

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue, Value};
use coqui_acp_api::{CapabilitiesSource, ModelInfo};
use guatiao_intake::Screen;

pub use self::openai_shape::{Dialect, OpenAiShaped, SERVER_ID, Server, ServerSettings};

/// The name of the `sdk_provider` kind.
pub const KIND: &str = "sdk_provider";

/// The capability ids the host asks for. The set is open: a vendor may add one, and the host passes it through.
pub mod capability {
    /// Text in, text out.
    pub const CHAT: &str = "chat";
    /// Texts to vectors.
    pub const EMBED: &str = "embed";
    /// A query and passages to an order.
    pub const RERANK: &str = "rerank";
    /// A recording to text.
    pub const SPEECH_TO_TEXT: &str = "speech_to_text";
    /// Text to audio.
    pub const TEXT_TO_SPEECH: &str = "text_to_speech";
}

/// Message prefix of an error that means the key was refused, so the host can say the account needs a new sign-in.
pub const SIGNED_OUT_PREFIX: &str = "sign-in refused: ";

/// One operation a provider implements.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Capability {
    /// A word of lower-case letters, digits and `_`.
    pub id: String,
    /// Whether the operation streams its answer. A fact of the operation, not of a model.
    #[map(default)]
    pub stream: bool,
    /// The most items one call takes (texts to embed, passages to rank), when the vendor limits it.
    pub max_batch: Option<u32>,
    /// The most bytes of items one call takes, when the vendor limits it.
    pub max_batch_bytes: Option<u64>,
}

impl Capability {
    /// The operation `id`, with no limit.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            stream: false,
            max_batch: None,
            max_batch_bytes: None,
        }
    }
}

/// How the host applies the credential to a request: the header, and the word before the key (`Bearer`), if any.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Auth {
    /// The header's name.
    pub header: String,
    /// The scheme written before the key, with a space between; none sends the key alone.
    pub scheme: Option<String>,
}

impl Auth {
    /// `Authorization: Bearer KEY`.
    #[must_use]
    pub fn bearer() -> Self {
        Self {
            header: "authorization".to_owned(),
            scheme: Some("Bearer".to_owned()),
        }
    }

    /// The key alone in the header `name`.
    #[must_use]
    pub fn header(name: impl Into<String>) -> Self {
        Self {
            header: name.into(),
            scheme: None,
        }
    }

    /// The value of the header for `key`.
    #[must_use]
    pub fn value(&self, key: &str) -> String {
        match &self.scheme {
            Some(scheme) => alloc::format!("{scheme} {key}"),
            None => key.to_owned(),
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self::bearer()
    }
}

/// What a provider is: from [`SdkProvider::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SdkInfo {
    /// 1 to 32 lower-case letters and digits: the offer of an account is `ID:ACCOUNT`, one of the person's own is `NAME`.
    pub id: String,
    /// What it is called.
    pub title: String,
    /// What it is, in a sentence.
    pub summary: String,
    /// Where the text goes, in a sentence or two, said before the person sets anything up.
    pub location: String,
    /// The credential standards of the accounts it serves; empty: it needs no account (a server by address).
    #[map(default)]
    pub standards: Vec<String>,
    /// The address it is pinned to; absent, the account's own model API address.
    pub address: Option<String>,
    /// Which credential of the account holds the key.
    pub credential: String,
    /// How the host applies the key.
    #[map(default)]
    pub auth: Auth,
    /// The operations it implements.
    #[map(default)]
    pub capabilities: Vec<Capability>,
    /// The settings it takes, as a guatiao schema of an object, for a provider with no account.
    pub schema: Option<Value>,
    /// How the schema is shown, as a guatiao-intake form.
    pub form: Option<Value>,
}

impl SdkInfo {
    /// A provider `id` called `title` serving the accounts of `standards`, with no operation yet.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: String::new(),
            location: String::new(),
            standards: Vec::new(),
            address: None,
            credential: String::new(),
            auth: Auth::bearer(),
            capabilities: Vec::new(),
            schema: None,
            form: None,
        }
    }

    /// This provider taking the settings `S` describes.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Whether it implements `capability`.
    #[must_use]
    pub fn can(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|each| each.id == capability)
    }

    /// Why this description cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = |text: &str, extra: bool| {
            (1..=32).contains(&text.len())
                && text.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || (extra && byte == b'_')
                })
        };
        if !word(&self.id, false) {
            return Some(alloc::format!(
                "the provider id {:?} is not 1 to 32 lower-case letters and digits",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the provider {} has no title", self.id));
        }
        if !self.standards.is_empty() && self.credential.trim().is_empty() {
            return Some(alloc::format!(
                "the provider {} serves accounts but names no credential",
                self.id
            ));
        }
        if self.auth.header.trim().is_empty() {
            return Some(alloc::format!(
                "the provider {} names no auth header",
                self.id
            ));
        }
        if let Some(address) = &self.address
            && !(address.starts_with("https://") || address.starts_with("http://"))
        {
            return Some(alloc::format!(
                "the provider {} is pinned to {address:?}, which is not an http or https address",
                self.id
            ));
        }
        for (at, each) in self.capabilities.iter().enumerate() {
            if !word(&each.id, true) {
                return Some(alloc::format!(
                    "the capability id {:?} is not lower-case letters, digits and _",
                    each.id
                ));
            }
            if self.capabilities[..at]
                .iter()
                .any(|seen| seen.id == each.id)
            {
                return Some(alloc::format!(
                    "the capability {} is declared twice",
                    each.id
                ));
            }
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// What a host sends to [`SdkProvider::open`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct OpenRequest {
    /// The address the lent HTTP is bound to, which a provider reads to tell whether paths start with `/v1`. Never holds a key.
    pub address: String,
    /// The provider's own settings as a JSON object keyed by its schema's members; `{}` when it takes none.
    pub settings: String,
}

impl OpenRequest {
    /// A request for the address `address` with `settings`.
    #[must_use]
    pub fn new(address: impl Into<String>, settings: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            settings: settings.into(),
        }
    }
}

/// One header of a request or an answer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Header {
    /// Its name.
    pub name: String,
    /// Its value. A provider never puts the key here: the host applies it.
    pub value: String,
}

impl Header {
    /// The header `name: value`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A request below the address the host lent.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HttpRequest {
    /// `GET` or `POST`.
    pub method: String,
    /// The path and query below the address (`/embeddings`); never a URL.
    pub path: String,
    /// Headers beside the credential's (`content-type`, a version header).
    #[map(default)]
    pub headers: Vec<Header>,
    /// The body, text.
    pub body: Option<String>,
    /// The most bytes of the answer to read; the host's limit when absent.
    pub max_bytes: Option<u64>,
    /// The seconds the request may take; the host's limit when absent.
    pub timeout_secs: Option<u32>,
}

impl HttpRequest {
    /// A `POST` of the JSON `body` to `path`.
    #[must_use]
    pub fn post_json(path: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            method: "POST".to_owned(),
            path: path.into(),
            headers: alloc::vec![Header::new("content-type", "application/json")],
            body: Some(body.into()),
            max_bytes: None,
            timeout_secs: None,
        }
    }

    /// A `GET` of `path`.
    #[must_use]
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: "GET".to_owned(),
            path: path.into(),
            headers: Vec::new(),
            body: None,
            max_bytes: None,
            timeout_secs: None,
        }
    }
}

/// What a lent request answered, whatever its status.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HttpReply {
    /// The status code.
    pub status: u16,
    /// The body, text.
    pub body: String,
}

impl HttpReply {
    /// An answer with `status` and `body`.
    #[must_use]
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }
}

/// HTTP to the address of one account or server, made by the host: the credential applied, the person's network used, the answer
/// size limited, redirects refused, and nothing outside the address reachable. A transport failure is an error; a status is an
/// answer.
#[coqui_acp_api::guatiao::kind(object, name = "lent_http", crate = ::coqui_acp_api::guatiao)]
pub trait LentHttp: Send + Sync {
    /// Makes `request` below the address.
    fn request(&self, request: HttpRequest) -> Result<HttpReply, ProviderError>;
}

/// One message of a chat.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ChatMessage {
    /// `user` or `assistant`.
    pub role: String,
    /// What was said.
    pub content: String,
}

impl ChatMessage {
    /// The message `content` said by `role`.
    #[must_use]
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

/// A chat request: text in, text out.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ChatRequest {
    /// The model.
    pub model: String,
    /// The instructions, apart from the conversation.
    pub system: Option<String>,
    /// The conversation, oldest first.
    pub messages: Vec<ChatMessage>,
    /// The most tokens of the answer.
    pub max_tokens: Option<u32>,
    /// The sampling temperature, for a model that takes one.
    pub temperature: Option<f32>,
    /// Whether the answer must be JSON.
    #[map(default)]
    pub json: bool,
}

impl ChatRequest {
    /// One user message to `model`.
    #[must_use]
    pub fn ask(model: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: None,
            messages: alloc::vec![ChatMessage::new("user", text)],
            max_tokens: None,
            temperature: None,
            json: false,
        }
    }
}

/// A chat answer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ChatReply {
    /// The answer's text.
    pub text: String,
    /// Tokens read, when the service says.
    pub input_tokens: Option<u64>,
    /// Tokens written, when the service says.
    pub output_tokens: Option<u64>,
}

impl ChatReply {
    /// An answer of `text`.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            input_tokens: None,
            output_tokens: None,
        }
    }
}

/// An embedding request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct EmbedRequest {
    /// The model.
    pub model: String,
    /// The texts, one vector each.
    pub inputs: Vec<String>,
    /// The vector length asked for, for a model that allows it.
    pub dimensions: Option<u32>,
}

impl EmbedRequest {
    /// `inputs` by `model`.
    #[must_use]
    pub fn new(model: impl Into<String>, inputs: Vec<String>) -> Self {
        Self {
            model: model.into(),
            inputs,
            dimensions: None,
        }
    }
}

/// The vectors of an embedding request, in the order of its inputs, all of one length.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct EmbedReply {
    /// The vectors.
    pub vectors: Vec<Vec<f32>>,
    /// Tokens read, when the service says.
    pub input_tokens: Option<u64>,
}

impl EmbedReply {
    /// An answer of `vectors`.
    #[must_use]
    pub fn new(vectors: Vec<Vec<f32>>) -> Self {
        Self {
            vectors,
            input_tokens: None,
        }
    }
}

/// A rerank request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RerankRequest {
    /// The model.
    pub model: String,
    /// What is looked for.
    pub query: String,
    /// The passages to order.
    pub passages: Vec<String>,
    /// The most results wanted.
    pub top: Option<u32>,
}

impl RerankRequest {
    /// `passages` ordered for `query` by `model`.
    #[must_use]
    pub fn new(model: impl Into<String>, query: impl Into<String>, passages: Vec<String>) -> Self {
        Self {
            model: model.into(),
            query: query.into(),
            passages,
            top: None,
        }
    }
}

/// One passage of a rerank answer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Ranked {
    /// The passage's place in the request, from 0.
    pub index: u32,
    /// Its score; higher is better.
    pub score: f64,
}

impl Ranked {
    /// The passage `index` scored `score`.
    #[must_use]
    pub fn new(index: u32, score: f64) -> Self {
        Self { index, score }
    }
}

/// A rerank answer: the passages that matter, best first.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RerankReply {
    /// The passages, best first; may be fewer than the request's.
    pub results: Vec<Ranked>,
}

impl RerankReply {
    /// An answer of `results`.
    #[must_use]
    pub fn new(results: Vec<Ranked>) -> Self {
        Self { results }
    }
}

/// `UNSUPPORTED`, naming the operation a provider did not describe. The host never calls one; this answers a caller that does.
#[must_use]
pub fn undeclared(operation: &str) -> ProviderError {
    crate::status::unsupported(alloc::format!(
        "this provider does not describe the operation {operation}"
    ))
}

/// An opened provider: every operation is called only if its capability was described.
#[coqui_acp_api::guatiao::kind(object, name = "sdk", crate = ::coqui_acp_api::guatiao)]
pub trait Sdk: Send + Sync {
    /// The models the account or server may use, each with the capabilities and limits the provider knows.
    fn models(&self) -> Result<coqui_acp_api::ModelListing, ProviderError>;

    /// A chat completion (`chat`).
    fn chat(&self, request: ChatRequest) -> Result<ChatReply, ProviderError>;

    /// Vectors of texts (`embed`).
    fn embed(&self, request: EmbedRequest) -> Result<EmbedReply, ProviderError>;

    /// An order of passages (`rerank`).
    fn rerank(&self, request: RerankRequest) -> Result<RerankReply, ProviderError>;
}

/// A vendor's client for an account type.
#[coqui_acp_api::guatiao::kind(name = "sdk_provider", crate = ::coqui_acp_api::guatiao)]
pub trait SdkProvider: Send + Sync {
    /// What it is and what it implements. The same answer on every call, with nothing run and no connection opened.
    fn describe(&self) -> Result<SdkInfo, ProviderError>;

    /// Opens the client over `http`, which the provider owns from then on.
    fn open(
        &self,
        request: OpenRequest,
        http: Object<dyn LentHttp>,
    ) -> Result<Object<dyn Sdk>, ProviderError>;
}

/// The models of a listing that can do `capability`, and the ones that do not report any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtered {
    /// Models that report `capability`.
    pub offered: Vec<ModelInfo>,
    /// Models that report no capabilities at all: not offered by default, shown apart, and a typed id is always accepted.
    pub unreported: Vec<ModelInfo>,
}

/// Splits `models` for a consumer that needs `capability`: a model that reports capabilities and lacks it is left out.
#[must_use]
pub fn models_for(models: &[ModelInfo], capability: &str) -> Filtered {
    let mut out = Filtered::default();
    for model in models {
        if model.can(capability) {
            out.offered.push(model.clone());
        } else if !model.reports_capabilities() {
            out.unreported.push(model.clone());
        }
    }
    out
}

/// A model `id` that can do `capabilities`, from `source`.
#[must_use]
pub fn model(id: &str, capabilities: &[&str], source: CapabilitiesSource) -> ModelInfo {
    ModelInfo::new(id).with_capabilities(capabilities.iter().copied(), source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> SdkInfo {
        let mut info = SdkInfo::new("acme", "Acme");
        info.standards = alloc::vec!["acme-api".to_owned()];
        info.credential = "api_key".to_owned();
        info.capabilities = alloc::vec![Capability::new("chat")];
        info
    }

    #[test]
    fn a_description_with_what_a_host_needs_is_offered() {
        assert_eq!(info().problem(), None);
        assert!(info().can("chat") && !info().can("embed"));
    }

    #[test]
    fn a_description_missing_what_a_host_needs_says_what() {
        for (change, word) in [
            (
                (|i: &mut SdkInfo| i.id = "Acme-1".into()) as fn(&mut _),
                "lower-case",
            ),
            (|i| i.title = " ".into(), "title"),
            (|i| i.credential.clear(), "credential"),
            (|i| i.address = Some("ftp://x".into()), "http"),
            (|i| i.auth.header.clear(), "auth"),
            (|i| i.capabilities.push(Capability::new("chat")), "twice"),
            (
                |i| i.capabilities.push(Capability::new("Bad-Id")),
                "capability id",
            ),
        ] {
            let mut made = info();
            change(&mut made);
            assert!(
                made.problem().is_some_and(|why| why.contains(word)),
                "{word}: {:?}",
                made.problem()
            );
        }
    }

    #[test]
    fn the_credential_is_written_as_the_auth_says() {
        assert_eq!(Auth::bearer().value("k"), "Bearer k");
        assert_eq!(Auth::header("x-api-key").value("k"), "k");
    }

    #[test]
    fn models_are_filtered_by_what_they_report_and_the_rest_are_kept_apart() {
        let models = alloc::vec![
            model("e", &["embed"], CapabilitiesSource::Declared),
            model("c", &["chat", "tool_use"], CapabilitiesSource::Listed),
            ModelInfo::new("unknown"),
        ];
        let embed = models_for(&models, "embed");
        assert_eq!(embed.offered.len(), 1);
        assert_eq!(embed.offered[0].id, "e");
        assert_eq!(embed.unreported.len(), 1);
        assert_eq!(embed.unreported[0].id, "unknown");
        let chat = models_for(&models, "chat");
        assert_eq!(chat.offered[0].id, "c");
    }
}
