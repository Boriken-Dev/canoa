//! Antigravity's accounts: the automatic `local` account, which is the login Antigravity keeps for itself, and the one way to
//! make it, Antigravity's own sign-in. Whether anyone is signed in is read from the stored login, with no process started;
//! the sign-in runs the program only when no login is stored, relays the page it prints and a code it asks for, and ends once
//! the login is stored.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;
use std::sync::Mutex;
use std::time::Instant;

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    ASK, AccountKeeper, AccountModelsQuery, AccountOffer, AccountProvider, AccountProviderInfo,
    AccountRef, AccountUsage, AccountUsageQuery, AccountWatch, Credentials, HARNESS_LOGIN,
    LoginForm, LoginMethodInfo, LoginOutcome, LoginPrompter, LoginRequest, ModelListing, OPEN_URL,
    OpenUrl, ProcessSpawner, RenewReport, RenewRequest, WatchRequest,
};
use coqui_acp_harness::antigravity::{AgyLogin, LoginReading, SignedIn};
use coqui_acp_harness::cli::{Cli, Next, address_in, plain};
use coqui_acp_harness::process::Launch;
use coqui_acp_harness::stdio::login::{self, Ctx};
use coqui_acp_harness::stdio::{StdioAccountSettings, StdioAccounts};
use serde_json::json;

use crate::login::{AgySettings, lock, reader};

/// The way to sign in: Antigravity's own sign-in, run only when no login is stored.
pub const SIGN_IN: &str = "sign_in";

/// How long a sign-in waits for each piece of the program's output before it looks at the stored login again.
const PIECE_WAIT: Duration = Duration::from_millis(100);

/// The most of one line of the program's output that is kept while it has no end.
const LINE_LIMIT: usize = 1 << 16;

/// Antigravity's accounts, as a provider of `account_provider` under the id `antigravity_accounts`.
#[derive(Debug, coqui_acp_api::guatiao::Provider)]
#[provider(
    AccountProvider,
    id = "antigravity_accounts",
    config = AgySettings,
    new = AgyAccounts::unconfigured,
    crate = ::coqui_acp_api::guatiao
)]
pub struct AgyAccounts {
    inner: StdioAccounts,
    settings: AgySettings,
}

impl From<AgySettings> for AgyAccounts {
    fn from(settings: AgySettings) -> Self {
        let mut stdio = StdioAccountSettings::new();
        stdio.executable.clone_from(&settings.executable);
        stdio.renew_poll_ms = settings.renew_poll_ms;
        stdio.command_timeout_ms = settings.command_timeout_ms;
        Self {
            inner: StdioAccounts::new(&coqui_acp_harness::vendors::antigravity::STDIO, stdio),
            settings,
        }
    }
}

impl AgyAccounts {
    /// A provider configured as `settings` say.
    #[must_use]
    pub fn new(settings: AgySettings) -> Self {
        settings.into()
    }

    fn unconfigured() -> Self {
        Self::new(AgySettings::default())
    }
}

/// The ways to sign in.
fn logins() -> Vec<LoginMethodInfo> {
    let mut method = LoginMethodInfo::new(SIGN_IN, "Sign in to Antigravity", HARNESS_LOGIN);
    method.description = "Runs Antigravity's own sign-in and opens Google's page for it, then reads the login it stores. When a login is already stored, nothing is run.".to_string();
    method.steps = alloc::vec![OPEN_URL.to_string(), ASK.to_string()];
    alloc::vec![method]
}

impl AccountProvider for AgyAccounts {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        let mut info = AccountProviderInfo::new("Antigravity accounts");
        info.description = "Antigravity keeps its own login; this provider reads who is signed in from it, and runs Antigravity's sign-in when no one is.".to_string();
        Ok(info)
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        let mut offer = self.inner.offers()?;
        offer.logins = logins();
        Ok(offer)
    }

    fn provides(&self, account: AccountRef) -> Result<Vec<String>, ProviderError> {
        self.inner.provides(account)
    }

    fn login(
        &self,
        request: LoginRequest,
        spawner: Object<dyn ProcessSpawner>,
        prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        if request.method != SIGN_IN {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "Antigravity offers no such way to sign in",
            ));
        }
        let stored = reader(&self.settings);
        if let Some(signed) = signed_in(&stored) {
            return Ok(outcome(&signed));
        }
        let settings = self.settings.clone();
        let dir = request.state_dir.clone();
        login::run(spawner, prompter, request.timeout_ms, move |ctx| {
            sign_in(ctx, &settings, &dir, &stored)
        })
    }

    fn renew(
        &self,
        request: RenewRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        self.inner.renew(request, spawner)
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        self.inner.watch(request, spawner, keeper)
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        self.inner.usage(query, spawner, keeper)
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        self.inner.models(query, spawner, keeper)
    }
}

/// The stored login, when one is: a login that cannot be read counts as none here, since the sign-in makes a new one.
fn signed_in(stored: &Arc<Mutex<AgyLogin>>) -> Option<SignedIn> {
    match lock(stored).read(&[]) {
        Ok(LoginReading::SignedIn(signed)) => Some(signed),
        _ => None,
    }
}

/// The automatic `local` account a stored login is, named by its email when the login says it.
fn outcome(signed: &SignedIn) -> LoginOutcome {
    let mut outcome = LoginOutcome::new(HARNESS_LOGIN, Credentials::new());
    outcome.label = signed
        .account
        .email
        .clone()
        .or_else(|| signed.account.name.clone());
    outcome.checked = signed.usage.is_ok();
    outcome
}

/// Runs the program's sign-in in `dir` until a login is stored, relaying the page it prints and a code it asks for.
fn sign_in(
    ctx: &Ctx<'_>,
    settings: &AgySettings,
    dir: &str,
    stored: &Arc<Mutex<AgyLogin>>,
) -> Result<LoginOutcome, ProviderError> {
    let launch = Launch::new(settings.program())
        .args(settings.login_args.iter().map(String::as_str))
        .cwd(dir);
    let child = ctx.spawn(&launch)?;
    let cli = child.input();
    let result = follow(ctx, settings, &cli, stored);
    cli.kill();
    result
}

fn follow(
    ctx: &Ctx<'_>,
    settings: &AgySettings,
    cli: &Arc<Cli>,
    stored: &Arc<Mutex<AgyLogin>>,
) -> Result<LoginOutcome, ProviderError> {
    let hosts: Vec<&str> = settings.login_hosts.iter().map(String::as_str).collect();
    let poll = Duration::from_millis(settings.login_poll_ms);
    let mut looked = Instant::now();
    let mut pending: Vec<u8> = Vec::new();
    let mut opened: Vec<String> = Vec::new();
    loop {
        ctx.check()?;
        let ended = match cli.next(PIECE_WAIT) {
            Next::Piece(piece) => {
                pending.extend_from_slice(&piece.bytes);
                false
            }
            Next::Idle => false,
            Next::Ended => true,
        };
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            heard(ctx, cli, &line, &hosts, &mut opened)?;
        }
        if pending.len() > LINE_LIMIT {
            pending.clear();
        }
        if ended || looked.elapsed() >= poll {
            looked = Instant::now();
            if let Some(signed) = signed_in(stored) {
                return Ok(outcome(&signed));
            }
        }
        if ended {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "login refused: Antigravity's sign-in ended and no login is stored",
            ));
        }
    }
}

/// One line of the sign-in's output: a page to open is offered to the person once, and a request for the code the page shows
/// is asked of them and typed in.
fn heard(
    ctx: &Ctx<'_>,
    cli: &Cli,
    line: &str,
    hosts: &[&str],
    opened: &mut Vec<String>,
) -> Result<(), ProviderError> {
    if let Some(url) = address_in(line, hosts) {
        if !opened.contains(&url) {
            ctx.open_url(OpenUrl::new(url.clone()))?;
            opened.push(url);
        }
        return Ok(());
    }
    let shown = plain(line);
    if !asks_for_code(&shown) {
        return Ok(());
    }
    let form = LoginForm::new(
        "Sign in to Antigravity",
        coqui_acp_harness::provider::schema(&json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "title": "Authorization code",
                    "description": shown.trim(),
                    "writeOnly": true,
                    "x-sensitive": true,
                },
            },
            "required": ["code"],
        })),
    );
    let answer = ctx.ask(form)?;
    let code = answer
        .secrets
        .expose()
        .get("code")
        .unwrap_or_default()
        .trim()
        .to_owned();
    cli.write(alloc::format!("{code}\n").as_bytes())
        .map_err(|error| {
            ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                &alloc::format!("Antigravity's sign-in does not read its input ({error})"),
            )
        })
}

/// Whether a line the sign-in printed asks for the authorization code its page shows.
fn asks_for_code(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("authorization code") && (line.contains("paste") || line.contains("enter"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_offers_its_own_sign_in_and_the_local_account() {
        let offer = AgyAccounts::new(AgySettings::default())
            .offers()
            .expect("an offer");
        assert!(offer.local, "{offer:?}");
        assert_eq!(offer.logins.len(), 1);
        assert_eq!(offer.logins[0].id, SIGN_IN);
        assert_eq!(offer.logins[0].title, "Sign in to Antigravity");
        assert_eq!(offer.logins[0].produces, HARNESS_LOGIN);
    }

    #[test]
    fn a_line_that_asks_for_the_code_is_told_from_one_that_does_not() {
        assert!(asks_for_code(
            "Paste the authorization code shown in your browser:"
        ));
        assert!(asks_for_code("Enter the authorization code: "));
        assert!(!asks_for_code("Waiting for you to sign in..."));
        assert!(!asks_for_code("Authorization code submission timed out."));
    }
}
