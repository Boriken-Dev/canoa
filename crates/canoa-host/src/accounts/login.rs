//! Signing in: a provider's login method run in a directory of its own, its prompts relayed to whoever is signing in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "directories and flags are std's"
)]

use std::fmt;
use std::fs;
use std::sync::Arc;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::schema::{SchemaRef, inline_refs};
use coqui_acp_api::guatiao::{Status, Value};
use coqui_acp_api::{
    AcpHarness, Credentials, LoginAnswer, LoginForm, LoginOutcome, LoginPrompter, LoginRequest,
    OpenUrl, ProcessSpawner,
};
use coqui_acp_harness::LocalSpawner;

use super::Accounts;
use crate::config::Settings;
use crate::provider::map_of;

/// What a person signing in sees and answers. The command line implements it with a terminal, and an application hosting
/// the library with its own: one that cannot take a typed answer says so, and the sign-in that needs one is refused.
pub(crate) trait Interaction: Send + Sync {
    /// A line of what the sign-in is doing.
    fn say(&self, text: &str);

    /// A page to open, with the code to confirm there when there is one.
    fn open_url(&self, open: &OpenUrl);

    /// The answers to a form, the secret ones apart; the reason when this front cannot ask.
    fn ask(&self, form: &LoginForm) -> Result<LoginAnswer, String>;

    /// Whether the person has cancelled.
    fn cancelled(&self) -> bool;
}

struct Prompter(Arc<dyn Interaction>);

impl LoginPrompter for Prompter {
    fn say(&self, text: &str) -> Result<(), ProviderError> {
        if self.0.cancelled() {
            return Err(cancelled());
        }
        self.0.say(text);
        Ok(())
    }

    fn open_url(&self, open: OpenUrl) -> Result<(), ProviderError> {
        if self.0.cancelled() {
            return Err(cancelled());
        }
        self.0.open_url(&open);
        Ok(())
    }

    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        if self.0.cancelled() {
            return Err(cancelled());
        }
        self.0
            .ask(&form)
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, &why))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(self.0.cancelled())
    }
}

fn cancelled() -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, "login cancelled")
}

/// Why a sign-in did not produce an account.
#[derive(Debug)]
pub(crate) struct SignInError(pub(crate) String);

impl fmt::Display for SignInError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for SignInError {}

/// Runs the login `method` of `provider` with `inputs`, relaying its prompts to `interaction`. The provider works in a
/// directory made for this sign-in, which is removed however it ended.
pub(crate) fn sign_in(
    accounts: &Accounts,
    provider: &dyn AcpHarness,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    interaction: Arc<dyn Interaction>,
    timeout_ms: Option<u64>,
) -> Result<LoginOutcome, SignInError> {
    let dir = accounts
        .login_dir()
        .map_err(|error| SignInError(error.to_string()))?;
    let result = run(provider, method, inputs, &dir, interaction, timeout_ms);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn run(
    provider: &dyn AcpHarness,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    dir: &std::path::Path,
    interaction: Arc<dyn Interaction>,
    timeout_ms: Option<u64>,
) -> Result<LoginOutcome, SignInError> {
    let dir_text = dir.to_string_lossy().into_owned();
    let mut request = LoginRequest::new(method, dir_text.clone()).inputs(Value::from(
        map_of(&Settings::new(inputs.clone())).map_err(SignInError)?,
    ));
    if let Some(ms) = timeout_ms {
        request = request.timeout_ms(ms);
    }
    let spawner = LocalSpawner::new().files(&dir_text);
    provider
        .login(
            request,
            spawner.into_object(),
            Prompter(interaction).into_object(),
        )
        .map_err(|error| {
            let why = error.message();
            SignInError(if why.is_empty() {
                format!("the sign-in failed ({:?})", error.status)
            } else {
                why.to_owned()
            })
        })
}

/// The fields of a form's schema: key, title, and whether it is a secret.
pub(crate) fn form_fields(form: &LoginForm) -> Vec<(String, String, bool)> {
    let Ok(schema) = inline_refs(&form.schema) else {
        return Vec::new();
    };
    let Some(schema) = SchemaRef::new(&schema) else {
        return Vec::new();
    };
    schema
        .fields()
        .map(|field| {
            let title = field.title();
            (
                field.key().to_owned(),
                if title.is_empty() { field.key() } else { title }.to_owned(),
                field.is_sensitive(),
            )
        })
        .collect()
}

/// Answers a form from `read`, which is given each field's title and whether it is a secret and answers its text.
pub(crate) fn answer_form(
    form: &LoginForm,
    mut read: impl FnMut(&str, bool) -> Result<String, String>,
) -> Result<LoginAnswer, String> {
    let mut values = serde_json::Map::new();
    let mut secrets = Credentials::new();
    for (key, title, secret) in form_fields(form) {
        let text = read(&title, secret)?;
        if secret {
            secrets.insert(key, text);
        } else {
            values.insert(key, serde_json::Value::String(text));
        }
    }
    let values = Value::from(map_of(&Settings::new(values))?);
    Ok(LoginAnswer::new(values, secrets))
}
