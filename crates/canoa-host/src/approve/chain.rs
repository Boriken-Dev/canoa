//! The order a permission request is put to its approvers in, and what the first answer does.
//!
//! 1. The approvers that answer on the spot (`instant`), in the file's order: the first that gives a verdict ends the request, an
//!    abstain moves to the next.
//! 2. The approvers that have to wait for someone, all at once, beside the window: the first answer from any of them wins and the
//!    others are withdrawn. When the window answers first they are all withdrawn.
//! 3. When every one of them has timed out or declined and no window is attached, the request is refused; with a window attached it
//!    stays with the window. A timeout never allows.
//!
//! An approver's allow is applied only when the agent offers an unambiguous option that allows once, and a policy's allow never to what
//! the host calls protected. A reject is always applied.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and clocks are std's"
)]

use core::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use canoa::approver::{
    Answer, Approver, ApproverInfo, Authority, Choice, MAX_WAIT_MS, Request, Step, Ticket,
};

use super::catalog::Catalog;
use super::config::{Approvers, Detail, Entry};

/// How a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// An approver allowed it once.
    Allowed,
    /// An approver refused it once.
    Rejected,
    /// Nobody answered in time, and no window is attached.
    Refused,
}

/// The answer that ended a request, and who gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settled {
    /// The entry's name, or `window` for the window's own timeout.
    pub(crate) by: String,
    pub(crate) outcome: Outcome,
}

/// Who is at the screen when a request is made.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Presence {
    /// Whether a window is attached: with one, a request nobody else answers stays with it.
    pub(crate) window: bool,
    /// Whether a window reports it is attending the session, so an entry that asks only when the person is away is not asked.
    pub(crate) attended: bool,
}

/// What is told of a request that is settled by an approver.
pub(crate) type Settle = Arc<dyn Fn(Settled) + Send + Sync>;

/// One approver of an entry, ready to be asked.
pub(crate) struct Member {
    name: String,
    entry: Entry,
    info: ApproverInfo,
    instance: Arc<dyn Approver>,
}

impl Member {
    fn new(entry: Entry, instance: Arc<dyn Approver>) -> Result<Self, String> {
        let info = instance.describe().map_err(|error| error.to_string())?;
        if let Some(problem) = info.problem() {
            return Err(problem);
        }
        Ok(Self {
            name: entry.name.clone(),
            entry,
            info,
            instance,
        })
    }

    /// The request as this approver is shown it: the command and the files only when its entry asks, or it needs them.
    fn view(&self, request: &Request) -> Request {
        let mut view = request.clone();
        // Only the person widens what an approver is shown, in the entry: what the provider says it could use changes nothing.
        if self.entry.detail != Detail::Command {
            view.tool.command = None;
            view.tool.paths.clear();
        }
        view.deadline = self.entry.timeout.and_then(|timeout| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            i64::try_from((now + timeout).as_secs()).ok()
        });
        view
    }

    /// Whether a verdict of this approver is applied to `request`.
    fn applies(&self, choice: Choice, request: &Request, can_allow: bool) -> bool {
        match choice {
            Choice::Reject => true,
            Choice::Allow => {
                let policy_on_protected =
                    self.info.answers_as == Authority::Policy && request.protected;
                if !can_allow {
                    tracing::info!(
                        approver = self.name,
                        "an allow was not applied: the agent offers no unambiguous option that allows once"
                    );
                }
                if policy_on_protected {
                    tracing::info!(
                        approver = self.name,
                        "an allow was not applied: the call names a place the person protects, and a rule never allows one"
                    );
                }
                can_allow && !policy_on_protected
            }
        }
    }
}

/// The approvers of a connection and how long the window is waited for.
pub(crate) struct Chain {
    members: Vec<Arc<Member>>,
    window_timeout: Option<Duration>,
    /// How long one `wait` is asked to take: at most [`MAX_WAIT_MS`].
    poll: Duration,
}

impl core::fmt::Debug for Chain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.members.iter().map(|member| &member.name))
            .finish()
    }
}

impl Chain {
    /// The approvers `approvers` names, made from `catalog`; one whose provider is missing, whose settings are refused or that cannot
    /// be described is left out and logged.
    pub(crate) fn new(approvers: &Approvers, catalog: &Catalog) -> Self {
        let mut members = Vec::new();
        for entry in &approvers.entries {
            let made = catalog
                .instance(&entry.provider, &entry.settings)
                .and_then(|instance| Member::new(entry.clone(), instance));
            match made {
                Ok(member) => {
                    if member.info.wants_detail && entry.detail != Detail::Command {
                        tracing::warn!(
                            approver = entry.name,
                            "this approver could use the command and files of a call; it is shown only the tool until its entry says detail = \"command\""
                        );
                    }
                    members.push(Arc::new(member));
                }
                Err(why) => {
                    tracing::warn!(approver = entry.name, "this approver is not used: {why}")
                }
            }
        }
        Self {
            members,
            window_timeout: approvers.window_timeout,
            poll: Duration::from_millis(1000),
        }
    }

    /// Whether anything but the window can answer.
    pub(crate) fn is_empty(&self) -> bool {
        self.members.is_empty() && self.window_timeout.is_none()
    }

    /// A chain of ready members, polling every `poll`.
    #[cfg(test)]
    pub(crate) fn of(
        members: Vec<(Entry, Arc<dyn Approver>)>,
        window_timeout: Option<Duration>,
        poll: Duration,
    ) -> Self {
        Self {
            members: members
                .into_iter()
                .map(|(entry, instance)| Arc::new(Member::new(entry, instance).expect("described")))
                .collect(),
            window_timeout,
            poll,
        }
    }

    /// The first verdict the approvers that answer on the spot give, in order, with its entry's name.
    pub(crate) fn instant(&self, request: &Request, can_allow: bool) -> Option<(String, Choice)> {
        for member in self.members.iter().filter(|member| member.info.instant) {
            match member.instance.ask(member.view(request)) {
                Ok(answer) if answer.problem().is_none() && answer.step == Step::Answered => {
                    let Some(choice) = answer.choice else {
                        continue;
                    };
                    if member.applies(choice, request, can_allow) {
                        return Some((member.name.clone(), choice));
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    approver = member.name,
                    "this approver could not answer: {error}"
                ),
            }
        }
        None
    }

    /// Puts `request` to the approvers that wait for someone, all at once, and times the window when its timeout is set; `settle`
    /// is called once, from a thread of the chain's, with the first answer. None when there is nobody to wait for.
    pub(crate) fn asking(
        &self,
        request: &Request,
        can_allow: bool,
        presence: Presence,
        settle: Settle,
    ) -> Option<Asking> {
        let members: Vec<Arc<Member>> = self
            .members
            .iter()
            .filter(|member| !member.info.instant)
            .filter(|member| !(member.entry.only_when_away && presence.attended))
            .cloned()
            .collect();
        if members.is_empty() && self.window_timeout.is_none() {
            return None;
        }
        let shared = Arc::new(Shared {
            closed: AtomicBool::new(false),
            live: AtomicUsize::new(members.len()),
        });
        if let Some(timeout) = self.window_timeout {
            let (shared, settle, poll) = (Arc::clone(&shared), Arc::clone(&settle), self.poll);
            let spawned = std::thread::Builder::new()
                .name("approver-window".to_owned())
                .spawn(move || {
                    let until = Instant::now() + timeout;
                    loop {
                        if shared.closed.load(Ordering::SeqCst) {
                            return;
                        }
                        let left = until.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        std::thread::sleep(left.min(poll.min(Duration::from_millis(100))));
                    }
                    if shared.close() {
                        settle(Settled {
                            by: super::config::WINDOW.to_owned(),
                            outcome: Outcome::Refused,
                        });
                    }
                });
            if let Err(error) = spawned {
                tracing::warn!("the window's timeout could not start: {error}");
            }
        }
        for member in members {
            let worker = Worker {
                member: Arc::clone(&member),
                request: request.clone(),
                can_allow,
                shared: Arc::clone(&shared),
                settle: Arc::clone(&settle),
                poll: self.poll,
                window: presence.window,
            };
            let spawned = std::thread::Builder::new()
                .name(format!("approver-{}", member.name))
                .spawn(move || worker.run());
            if let Err(error) = spawned {
                tracing::warn!(
                    approver = member.name,
                    "this approver could not start: {error}"
                );
                shared.live.fetch_sub(1, Ordering::SeqCst);
            }
        }
        Some(Asking { shared })
    }
}

/// What the workers of one request share.
struct Shared {
    /// Set once: by the first answer, the window's timeout, or the end of the request.
    closed: AtomicBool,
    /// How many workers have not finished.
    live: AtomicUsize,
}

impl Shared {
    /// Closes the request; true for the one caller that did.
    fn close(&self) -> bool {
        !self.closed.swap(true, Ordering::SeqCst)
    }
}

/// A request that approvers are answering. Ending it (the window answered, the turn was cancelled, the session or the link ended)
/// withdraws every ticket.
pub(crate) struct Asking {
    shared: Arc<Shared>,
}

impl core::fmt::Debug for Asking {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Asking")
    }
}

impl Asking {
    /// Ends the request: no answer that arrives later settles it, and each approver withdraws what it waits for.
    pub(crate) fn end(&self) {
        self.shared.closed.store(true, Ordering::SeqCst);
    }

    /// Whether an approver, or the window's timeout, has already settled or ended it.
    #[cfg(test)]
    pub(crate) fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        self.end();
    }
}

struct Worker {
    member: Arc<Member>,
    request: Request,
    can_allow: bool,
    shared: Arc<Shared>,
    settle: Settle,
    poll: Duration,
    window: bool,
}

/// How a worker's wait ended.
enum Ended {
    /// It gave a choice.
    Chose(Choice),
    /// It abstained, timed out, failed or was ended, still holding `ticket` when it has one.
    None(Option<String>),
}

impl Worker {
    fn run(self) {
        let name = &self.member.name;
        let ended = self.wait_for_answer();
        match ended {
            Ended::Chose(choice) => {
                if self.member.applies(choice, &self.request, self.can_allow) && self.shared.close()
                {
                    tracing::info!(approver = name.as_str(), "answered a permission request");
                    (self.settle)(Settled {
                        by: name.clone(),
                        outcome: match choice {
                            Choice::Allow => Outcome::Allowed,
                            Choice::Reject => Outcome::Rejected,
                        },
                    });
                }
            }
            Ended::None(ticket) => {
                if let Some(ticket) = ticket
                    && let Err(error) = self.member.instance.withdraw(Ticket::new(ticket, 0))
                {
                    tracing::warn!(
                        approver = name.as_str(),
                        "a wait could not be withdrawn: {error}"
                    );
                }
            }
        }
        let last = self.shared.live.fetch_sub(1, Ordering::SeqCst) == 1;
        if last && !self.window && self.shared.close() {
            tracing::info!(
                "no approver answered and no window is attached: the request is refused"
            );
            (self.settle)(Settled {
                by: name.clone(),
                outcome: Outcome::Refused,
            });
        }
    }

    fn wait_for_answer(&self) -> Ended {
        let name = self.member.name.as_str();
        let view = self.member.view(&self.request);
        let answer = match self.member.instance.ask(view) {
            Ok(answer) => answer,
            Err(error) => {
                tracing::warn!(approver = name, "this approver could not answer: {error}");
                return Ended::None(None);
            }
        };
        if let Some(problem) = answer.problem() {
            tracing::warn!(
                approver = name,
                "this approver's answer is unusable: {problem}"
            );
            return Ended::None(None);
        }
        let ticket = match answer.step {
            Step::Answered => return answer.choice.map_or(Ended::None(None), Ended::Chose),
            Step::Abstain => return Ended::None(None),
            Step::Waiting => answer.ticket.unwrap_or_default(),
        };
        let until = self
            .member
            .entry
            .timeout
            .map(|timeout| Instant::now() + timeout);
        loop {
            if self.shared.closed.load(Ordering::SeqCst) {
                return Ended::None(Some(ticket));
            }
            let left = until.map(|until| until.saturating_duration_since(Instant::now()));
            if left.is_some_and(|left| left.is_zero()) {
                tracing::info!(approver = name, "this approver did not answer in time");
                return Ended::None(Some(ticket));
            }
            let slice = left.map_or(self.poll, |left| left.min(self.poll));
            let wait_ms = u64::try_from(slice.as_millis())
                .unwrap_or(MAX_WAIT_MS)
                .clamp(1, MAX_WAIT_MS);
            match self
                .member
                .instance
                .wait(Ticket::new(ticket.clone(), wait_ms))
            {
                Ok(Answer {
                    step: Step::Answered,
                    choice: Some(choice),
                    ..
                }) => return Ended::Chose(choice),
                Ok(Answer {
                    step: Step::Waiting,
                    ..
                }) => {}
                Ok(_) => return Ended::None(None),
                Err(error) => {
                    tracing::warn!(
                        approver = name,
                        "this approver could not be waited for: {error}"
                    );
                    return Ended::None(Some(ticket));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
