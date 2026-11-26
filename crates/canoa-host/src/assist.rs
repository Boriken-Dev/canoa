//! `assist`: the one host service the host's own model work asks. Knowledge search ranks and embeds through it, and the jobs that need
//! a model (titles, summaries) and Voice come to it too; none of them calls an account's API, an agent or a program itself.
//!
//! A consumer names an operation (`complete`, `embed`, `rerank`) and what it asks for; the service fulfils it from whatever can: a
//! backend. The ones here are an `sdk_provider` on an account (an API key), a server by address and a harness session on a
//! subscription login (`[assist_harness.NAME]`); a local program or an extension's own are more backends of the same trait. The person chooses a backend per
//! operation (`[assist.OPERATION] use`) and the ones that follow it (`fallback`); an operation no backend can serve fails with the
//! reason, and the consumer decides what to do (knowledge search answers by words).
//!
//! A backend may hold state across requests: `&self` methods, a [`Backend::reap`] the host calls from time to time and a
//! [`Backend::close`] at shutdown. The harness backends keep a pool of warm agent sessions there (keyed by harness, account and
//! profile, started on first use, a new conversation for each request, an idle timeout and a cap, a broken session replaced).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and the clock are std's"
)]

mod harness;
mod http;
mod sdk_backend;

use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Instant;

use canoa::network::Network;
use canoa::sdk::{
    ChatReply, ChatRequest, EmbedReply, EmbedRequest, RerankReply, RerankRequest, capability,
};
use coqui_acp_api::ModelListing;

pub(crate) use self::harness::{DEFAULT_IDLE, DEFAULT_SESSIONS, HarnessBackend, Opener, Pool};
pub(crate) use self::sdk_backend::backends_of;
use crate::config::{AssistChoice, DEFAULT_CHOICE};

/// What an operation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Operation {
    /// Text in, text out.
    Complete,
    /// Texts to vectors.
    Embed,
    /// A query and passages to an order.
    Rerank,
}

impl Operation {
    /// The name `[assist.OPERATION]` uses.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Embed => "embed",
            Self::Rerank => "rerank",
        }
    }

    /// The capability a backend must have to serve it.
    pub(crate) const fn capability(self) -> &'static str {
        match self {
            Self::Complete => capability::CHAT,
            Self::Embed => capability::EMBED,
            Self::Rerank => capability::RERANK,
        }
    }
}

/// One request.
#[derive(Debug, Clone)]
pub(crate) enum Call {
    /// A chat completion.
    Complete(ChatRequest),
    /// Vectors of texts.
    Embed(EmbedRequest),
    /// An order of passages.
    Rerank(RerankRequest),
}

impl Call {
    fn operation(&self) -> Operation {
        match self {
            Self::Complete(_) => Operation::Complete,
            Self::Embed(_) => Operation::Embed,
            Self::Rerank(_) => Operation::Rerank,
        }
    }
}

/// What a backend answered.
#[derive(Debug, Clone)]
pub(crate) enum Answer {
    /// A chat answer.
    Complete(ChatReply),
    /// Vectors.
    Embed(EmbedReply),
    /// An order.
    Rerank(RerankReply),
}

/// Why a backend could not serve a request.
#[derive(Debug, Clone)]
pub(crate) struct Failure {
    /// What went wrong, for people and logs; never a key.
    pub(crate) why: String,
    /// Whether the account needs a new sign-in.
    pub(crate) signed_out: bool,
}

impl Failure {
    pub(crate) fn new(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            signed_out: false,
        }
    }
}

/// Something that can serve operations.
pub(crate) trait Backend: Send + Sync {
    /// The id a table names it by: `openai:work` for an account's, the table's name for a server, `harness:NAME` for a harness.
    fn id(&self) -> &str;

    /// What it is called for people.
    fn title(&self) -> &str;

    /// Where the text goes, in a sentence or two, for the page that offers it.
    fn location(&self) -> &str;

    /// What kind of backend it is, for the page that lists them: `account` (an API of an account of the person's), `server` (one
    /// the person named by address) or `harness` (an agent on a login).
    fn kind(&self) -> &'static str {
        "backend"
    }

    /// Where the text is sent, as an address, when it goes to one.
    fn address(&self) -> Option<&str> {
        None
    }

    /// Whether it can serve `operation` at all.
    fn serves(&self, operation: Operation) -> bool;

    /// Serves `call`, taking no longer than `timeout`, through `network`.
    fn serve(
        &self,
        network: &Arc<Network>,
        call: Call,
        timeout: Duration,
    ) -> Result<Answer, Failure>;

    /// The models it offers, each with what it can do.
    fn models(&self, network: &Arc<Network>) -> Result<ModelListing, Failure>;

    /// Ends what has been idle too long. Called from time to time by the host.
    fn reap(&self) {}

    /// Ends everything it holds, at shutdown.
    fn close(&self) {}
}

/// An answer and who gave it.
#[derive(Debug, Clone)]
pub(crate) struct Done<T> {
    /// The answer.
    pub(crate) value: T,
    /// The id of the backend that served it.
    pub(crate) backend: String,
}

/// The service.
pub(crate) struct Assist {
    backends: Vec<Arc<dyn Backend>>,
    choices: indexmap::IndexMap<String, AssistChoice>,
    /// Backends that failed, and when: they are left alone for `retry`.
    down: Mutex<HashMap<String, (Instant, String)>>,
    retry: Duration,
    /// How the harness backends reach the host's agents, once the host has them.
    opener: Opener,
}

impl core::fmt::Debug for Assist {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Assist")
            .field(
                "backends",
                &self.backends.iter().map(|b| b.id()).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// How long a backend that failed is left alone, when `[host.kb] retry_seconds` does not say.
pub(crate) const DEFAULT_RETRY: Duration = Duration::from_secs(60);

impl Assist {
    /// The service over `backends`, in the order the first able one is taken, with the person's `choices`.
    #[cfg(test)]
    pub(crate) fn new(
        backends: Vec<Arc<dyn Backend>>,
        choices: indexmap::IndexMap<String, AssistChoice>,
        retry: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            backends,
            choices,
            down: Mutex::new(HashMap::new()),
            retry,
            opener: Opener::default(),
        })
    }

    /// Gives the harness backends the host they open agents through. Once.
    pub(crate) fn attach_host(&self, host: Arc<dyn crate::delegate::Host>) {
        let _ = self.opener.set(host);
    }

    /// The service of `config`: a backend for each of the person's accounts a loaded provider serves and for each `[sdk.NAME]`, the
    /// libraries read as `providers/list` reads them. `accounts` says where the accounts' keys rest; with none, no account is offered.
    pub(crate) fn of(
        config: &crate::config::Config,
        accounts: Option<(&crate::config::Layers, &std::path::Path)>,
    ) -> Arc<Self> {
        Self::build(config, accounts, Opener::default())
    }

    /// A service of `config` as it is now, which opens harnesses through the host this one does: for the page that lists what is
    /// offered and tests it, since canoa.cfg may have changed since this one was made. Its sessions are its own.
    pub(crate) fn fresh(
        &self,
        config: &crate::config::Config,
        accounts: Option<(&crate::config::Layers, &std::path::Path)>,
    ) -> Arc<Self> {
        Self::build(config, accounts, Arc::clone(&self.opener))
    }

    fn build(
        config: &crate::config::Config,
        accounts: Option<(&crate::config::Layers, &std::path::Path)>,
        opener: Opener,
    ) -> Arc<Self> {
        let registries = crate::control::providers::registries_of(Some(config));
        let retry = config
            .kb
            .retry_seconds
            .map_or(DEFAULT_RETRY, Duration::from_secs);
        let mut backends = backends_of(config, &registries, accounts);
        let pool = Pool::new(&config.assist.pool);
        for (name, table) in &config.assist.harnesses {
            backends.push(Arc::new(HarnessBackend::new(
                name,
                table,
                Arc::clone(&opener),
                Arc::clone(&pool),
            )));
        }
        for backend in &backends {
            tracing::info!(
                backend = backend.id(),
                "{}: {}",
                backend.title(),
                backend.location()
            );
        }
        let service = Arc::new(Self {
            backends,
            choices: config.assist.operations.clone(),
            down: Mutex::new(HashMap::new()),
            retry,
            opener,
        });
        if !config.assist.harnesses.is_empty() {
            Self::keep_tidy(&service, pool.idle_for());
        }
        service
    }

    /// Ends the sessions the backends hold idle, from time to time, for as long as the service lives.
    fn keep_tidy(service: &Arc<Self>, idle: Duration) {
        let every = (idle / 2).clamp(Duration::from_secs(1), Duration::from_secs(30));
        let weak: Weak<Self> = Arc::downgrade(service);
        let _ = std::thread::Builder::new()
            .name("canoa-assist-tidy".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(every);
                    let Some(service) = weak.upgrade() else {
                        break;
                    };
                    service.reap();
                }
            });
    }

    /// A service with no backend: every request fails with the reason.
    #[cfg(test)]
    pub(crate) fn none() -> Arc<Self> {
        Self::new(Vec::new(), indexmap::IndexMap::new(), DEFAULT_RETRY)
    }

    /// Every backend, in order.
    pub(crate) fn backends(&self) -> &[Arc<dyn Backend>] {
        &self.backends
    }

    /// Ends what the backends hold idle.
    pub(crate) fn reap(&self) {
        for backend in &self.backends {
            backend.reap();
        }
    }

    /// Ends everything the backends hold, at shutdown.
    pub(crate) fn close(&self) {
        for backend in &self.backends {
            backend.close();
        }
    }

    fn find(&self, id: &str) -> Option<&Arc<dyn Backend>> {
        self.backends.iter().find(|backend| backend.id() == id)
    }

    /// The backends asked for `operation` under `choice`, in the order they are asked: a backend's id asks that one alone; `default`
    /// asks what `[assist.OPERATION]` names and then its `fallback`, or every backend that can when it names none.
    ///
    /// # Errors
    ///
    /// Why no backend can serve it.
    pub(crate) fn chain(
        &self,
        operation: Operation,
        choice: &str,
    ) -> Result<Vec<Arc<dyn Backend>>, String> {
        let name = operation.name();
        let ids: Vec<String> = if choice == DEFAULT_CHOICE {
            match self.choices.get(name) {
                Some(table) if table.choice.is_some() || !table.fallback.is_empty() => table
                    .choice
                    .iter()
                    .chain(&table.fallback)
                    .cloned()
                    .collect(),
                _ => self
                    .backends
                    .iter()
                    .filter(|backend| backend.serves(operation))
                    .map(|backend| backend.id().to_owned())
                    .collect(),
            }
        } else {
            vec![choice.to_owned()]
        };
        let mut chain = Vec::new();
        let mut why = Vec::new();
        for id in ids {
            match self.find(&id) {
                None => why.push(format!("there is no backend {id}")),
                Some(backend) if !backend.serves(operation) => {
                    why.push(format!("{id} cannot {name}"));
                }
                Some(backend) => chain.push(Arc::clone(backend)),
            }
        }
        if chain.is_empty() {
            return Err(if why.is_empty() {
                format!(
                    "no backend is set up to {name}: add an account with an API key, or a server in [sdk.NAME]"
                )
            } else {
                why.join("; ")
            });
        }
        Ok(chain)
    }

    /// Whether `choice` has a backend that can serve `operation`.
    pub(crate) fn can(&self, operation: Operation, choice: &str) -> bool {
        self.chain(operation, choice).is_ok()
    }

    /// The id of the first backend `choice` would ask for `operation`.
    pub(crate) fn first(&self, operation: Operation, choice: &str) -> Option<String> {
        self.chain(operation, choice)
            .ok()?
            .first()
            .map(|backend| backend.id().to_owned())
    }

    fn serve(
        &self,
        network: &Arc<Network>,
        choice: &str,
        call: &Call,
        timeout: Duration,
    ) -> Result<Done<Answer>, String> {
        let operation = call.operation();
        let chain = self.chain(operation, choice)?;
        let mut why = Vec::new();
        let mut waiting = Vec::new();
        for backend in chain {
            let id = backend.id().to_owned();
            if let Some((when, said)) = self
                .down
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id)
                && when.elapsed() < self.retry
            {
                waiting.push(format!(
                    "{id} failed a moment ago ({said}) and is left alone for now"
                ));
                continue;
            }
            match backend.serve(network, call.clone(), timeout) {
                Ok(value) => {
                    self.down
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                    return Ok(Done { value, backend: id });
                }
                Err(failure) => {
                    let said = if failure.signed_out {
                        format!("{id}: the account needs a new sign-in ({})", failure.why)
                    } else {
                        format!("{id}: {}", failure.why)
                    };
                    self.down
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(id, (Instant::now(), failure.why));
                    why.push(said);
                }
            }
        }
        why.extend(waiting);
        Err(why.join("; "))
    }

    /// A chat completion.
    ///
    /// # Errors
    ///
    /// Why no backend served it.
    pub(crate) fn complete(
        &self,
        network: &Arc<Network>,
        choice: &str,
        request: ChatRequest,
        timeout: Duration,
    ) -> Result<Done<ChatReply>, String> {
        let done = self.serve(network, choice, &Call::Complete(request), timeout)?;
        match done.value {
            Answer::Complete(value) => Ok(Done {
                value,
                backend: done.backend,
            }),
            _ => Err(format!("{} answered another operation", done.backend)),
        }
    }

    /// Vectors of texts.
    ///
    /// # Errors
    ///
    /// Why no backend served it.
    pub(crate) fn embed(
        &self,
        network: &Arc<Network>,
        choice: &str,
        request: EmbedRequest,
        timeout: Duration,
    ) -> Result<Done<EmbedReply>, String> {
        let done = self.serve(network, choice, &Call::Embed(request), timeout)?;
        match done.value {
            Answer::Embed(value) => Ok(Done {
                value,
                backend: done.backend,
            }),
            _ => Err(format!("{} answered another operation", done.backend)),
        }
    }

    /// An order of passages.
    ///
    /// # Errors
    ///
    /// Why no backend served it.
    pub(crate) fn rerank(
        &self,
        network: &Arc<Network>,
        choice: &str,
        request: RerankRequest,
        timeout: Duration,
    ) -> Result<Done<RerankReply>, String> {
        let done = self.serve(network, choice, &Call::Rerank(request), timeout)?;
        match done.value {
            Answer::Rerank(value) => Ok(Done {
                value,
                backend: done.backend,
            }),
            _ => Err(format!("{} answered another operation", done.backend)),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;

/// The libraries a test reads providers from: the ones this build carries.
#[cfg(test)]
pub(crate) mod tests_support {
    use alloc::sync::Arc;
    use coqui_acp_api::guatiao::library::Registry;

    pub(crate) fn registries() -> Vec<Arc<Registry>> {
        crate::control::providers::registries_of(None)
    }
}
