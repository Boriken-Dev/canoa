//! Interactive login prompt relay and cancel handling for CLI and hosted sign-ins.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and channels are std's"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use coqui_acp_api::{LoginAnswer, LoginForm, OpenUrl};

use super::super::Accounts;
use super::super::login::{self, Interaction};
use super::{Env, Signing};
use crate::cli::{Failed, Format, Front};

/// What a sign-in asks of its front, passed to the thread that owns it.
pub(crate) enum Event {
    Say(String),
    Open(OpenUrl),
    Ask(LoginForm, mpsc::Sender<Result<LoginAnswer, String>>),
}

/// The sign-in's side of the conversation: it queues what the provider says and waits for the answer to a question.
pub(crate) struct Relay {
    events: Mutex<mpsc::Sender<Event>>,
    cancelled: Arc<AtomicBool>,
}

impl Relay {
    pub(crate) const fn new(events: mpsc::Sender<Event>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            events: Mutex::new(events),
            cancelled,
        }
    }

    fn send(&self, event: Event) -> bool {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(event)
            .is_ok()
    }
}

impl Interaction for Relay {
    fn say(&self, text: &str) {
        self.send(Event::Say(text.to_owned()));
    }

    fn open_url(&self, open: &OpenUrl) {
        self.send(Event::Open(open.clone()));
    }

    fn ask(&self, form: &LoginForm) -> Result<LoginAnswer, String> {
        let (reply, answer) = mpsc::channel();
        if !self.send(Event::Ask(form.clone(), reply)) {
            return Err("the sign-in is over".to_owned());
        }
        loop {
            match answer.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => return answer,
                Err(mpsc::RecvTimeoutError::Timeout) if self.cancelled() => {
                    return Err("login cancelled".to_owned());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("the sign-in is over".to_owned());
                }
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Why a typed answer cannot be taken, when the front has no way to take one.
const NO_TYPING: &str = "this way of running canoa cannot take a typed secret: sealed prompts are not available to it yet, so run `canoa accounts add` in a terminal, or add the account in the app";

/// Runs the sign-in `method` of an account provider and answers what it made, showing its prompts through `env.front`. `cancel` is
/// set when the person gives up.
pub(super) fn sign_in(
    env: &Env<'_>,
    accounts: &Accounts,
    provider: &Signing<'_>,
    method: &str,
    inputs: &serde_json::Map<String, serde_json::Value>,
    format: Format,
    cancel: &Cancel,
) -> Result<coqui_acp_api::LoginOutcome, Failed> {
    let (events, incoming) = mpsc::channel();
    let relay = Arc::new(Relay::new(events, Arc::clone(&cancel.flag)));
    let front = env.front;
    thread::scope(|scope| {
        let pump = scope.spawn(move || {
            for event in incoming {
                match event {
                    Event::Say(text) => show(front, format, "say", &text, None),
                    Event::Open(open) => show(
                        front,
                        format,
                        "open_url",
                        &open.url,
                        Some((open.user_code.as_deref(), open.expires_ms)),
                    ),
                    Event::Ask(form, reply) => {
                        let answer = login::answer_form(&form, |title, secret| {
                            front
                                .ask(&format!("{title}: "), secret)
                                .map_err(|error| error.to_string())?
                                .ok_or_else(|| NO_TYPING.to_owned())
                        });
                        let _ = reply.send(answer);
                    }
                }
            }
        });
        let outcome = login::sign_in(
            accounts,
            provider.makers,
            provider.provider,
            method,
            inputs,
            relay.clone(),
            None,
        );
        // The pump ends when the relay, which holds the only sender, is dropped.
        drop(relay);
        let _ = pump.join();
        // The sign-in's directory went with it.
        cancel.finish();
        outcome.map_err(|error| Failed::run(error.to_string()))
    })
}

/// One thing the sign-in said, as a line the person reads or a JSON line an application reads.
fn show(
    front: &dyn Front,
    format: Format,
    kind: &str,
    text: &str,
    open: Option<(Option<&str>, Option<u64>)>,
) {
    match format {
        Format::Json => {
            let line = match (kind, open) {
                ("open_url", Some((code, expires))) => serde_json::json!({"open_url": {
                    "url": text, "user_code": code, "expires_ms": expires}}),
                _ => serde_json::json!({ kind: text }),
            };
            let _ = front.print(&line.to_string());
        }
        Format::Text => match open {
            Some((code, _)) => {
                front.say(&format!("Open this page: {text}"));
                if let Some(code) = code {
                    front.say(&format!("Confirm this code there: {code}"));
                }
            }
            None => front.say(text),
        },
    }
}

/// Whether the person has given up on a sign-in, and whether it has cleaned up after itself.
pub(super) struct Cancel {
    flag: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl Cancel {
    /// The sign-in has ended and removed what it made: whoever is waiting to end the process may.
    pub(super) fn finish(&self) {
        *self.done.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.done.1.notify_all();
    }
}

/// How long ending the process on a cancel waits for the sign-in to remove what it made, under the five seconds after
/// which a process that ends on its input closing ends whatever is left.
const CLEANUP_WAIT: Duration = Duration::from_secs(3);

/// Cancels a sign-in when the front's input closes, for an application that runs the command as a job, and, before a front
/// that ends the process afterwards does, waits for the sign-in to clean up.
pub(super) fn cancel_on_input_closing(front: &dyn Front, until: bool) -> Result<Cancel, Failed> {
    let cancel = Cancel {
        flag: Arc::new(AtomicBool::new(false)),
        done: Arc::new((Mutex::new(false), Condvar::new())),
    };
    if until {
        let (flag, done) = (Arc::clone(&cancel.flag), Arc::clone(&cancel.done));
        front
            .when_input_closes(
                1,
                Box::new(move || {
                    flag.store(true, Ordering::SeqCst);
                    let ended = done.0.lock().unwrap_or_else(PoisonError::into_inner);
                    let _ = done
                        .1
                        .wait_timeout_while(ended, CLEANUP_WAIT, |ended| !*ended);
                }),
            )
            .map_err(Failed::run)?;
    }
    Ok(cancel)
}
