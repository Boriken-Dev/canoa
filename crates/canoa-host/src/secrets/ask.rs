//! The host asking the application in its own process for a form of its own: a sign-in's key or code. The answer is sealed to a
//! key made for the request and bound by the link's secret, as an agent's `secret_request` is, and the host opens it itself.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use coqui_acp::secret::{
    Context, DEFAULT_LIFETIME, IdentityId, Limits, LinkKey, LinkRole, Method, Prover, Receiver,
    RequestParams, RequestReply, Verifier,
};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

/// The method the host puts the form with: the secret-prompt extension's request.
pub(crate) const METHOD: &str = coqui_acp::secret::REQUEST_METHOD;

/// What the host asks with and checks an answer by, for the application in this process.
pub(crate) struct Asker {
    prover: Prover,
    verifier: Verifier,
    app: IdentityId,
    next: AtomicU64,
}

/// One form put to the person, waiting for its answer.
pub(crate) struct Asked {
    receiver: Receiver,
    request_id: String,
}

/// What the person answered: the plain values, and the secret ones as text.
pub(crate) struct Answer {
    pub(crate) plain: Map<String, Value>,
    pub(crate) secrets: BTreeMap<String, Zeroizing<String>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl Asker {
    pub(super) fn new(key: &LinkKey) -> Option<Self> {
        let host = key.identity(LinkRole::Host);
        let verifier = Verifier::builder(host)
            .link(key, LinkRole::Host)
            .ok()?
            .build();
        Some(Self {
            prover: Prover::linked(key, LinkRole::Host),
            verifier,
            app: key.identity(LinkRole::App),
            next: AtomicU64::new(0),
        })
    }

    /// What asks a browser's key, which no process secret binds: its prover and verifier are the ones secret tools use for it.
    pub(super) fn bound(prover: Prover, verifier: Verifier, app: IdentityId) -> Self {
        Self {
            prover,
            verifier,
            app,
            next: AtomicU64::new(0),
        }
    }

    /// The request that puts `schema` (JSON text of a form), titled `title`, to the person, and what is needed to open the answer.
    pub(crate) fn ask(&self, title: &str, schema: &str) -> Result<(Value, Asked), String> {
        let mut random = [0_u8; 12];
        getrandom::fill(&mut random).map_err(|_| "no randomness".to_owned())?;
        let hex: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let request_id = format!(
            "canoa-secret-{}-{hex}",
            self.next.fetch_add(1, Ordering::SeqCst)
        );
        let context = Context {
            session_id: "accounts".to_owned(),
            request_id: request_id.clone(),
            message: title.to_owned(),
            schema: schema.to_owned(),
            form: None,
            values: None,
        };
        let receiver = Receiver::new(
            Method::Request,
            &context,
            &Limits::new(),
            &self.prover,
            &self.app,
            now(),
            DEFAULT_LIFETIME,
        )
        .map_err(|_| "the form cannot be put to the person".to_owned())?;
        let params = RequestParams {
            context,
            recipient: receiver.recipient().clone(),
        }
        .to_value();
        Ok((
            params,
            Asked {
                receiver,
                request_id,
            },
        ))
    }

    /// Opens the application's `result` to the request `asked`: `Ok(None)` when the person declined or dismissed it.
    pub(crate) fn open(&self, asked: Asked, result: &Value) -> Result<Option<Answer>, String> {
        let reply = RequestReply::from_value(result, |_| true)
            .map_err(|_| "the answer is not one the host can read".to_owned())?;
        let RequestReply::Accept {
            plain,
            sealed,
            auth,
        } = reply
        else {
            return Ok(None);
        };
        let opened = asked
            .receiver
            .open_values(&self.verifier, &self.app, &plain, &sealed, &auth, now())
            .map_err(|_| "the answer was refused: it is not bound to this request".to_owned())?;
        let secrets = opened
            .secrets
            .iter()
            .map(|(name, value)| (name.clone(), Zeroizing::new(value.expose_str().to_owned())))
            .collect();
        Ok(Some(Answer {
            plain: opened.plain,
            secrets,
        }))
    }
}

impl Asked {
    /// The id the application's answer carries.
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
}
