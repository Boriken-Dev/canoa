use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use canoa::approver::{SessionRef, Tool};
use coqui_acp_api::guatiao::library::ProviderError;

use super::*;

/// What a stand-in approver does when asked.
#[derive(Clone)]
enum Plan {
    /// Answers at once.
    Now(Choice),
    /// Abstains at once.
    Abstain,
    /// Fails.
    Fails,
    /// Hands out a ticket, and answers `after` later with `choice`, or never when it is none.
    Later {
        after: Duration,
        choice: Option<Choice>,
    },
}

struct Stub {
    name: String,
    info: ApproverInfo,
    plan: Plan,
    started: Mutex<Option<Instant>>,
    log: Arc<Mutex<Vec<String>>>,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl Approver for Stub {
    fn describe(&self) -> Result<ApproverInfo, ProviderError> {
        Ok(self.info.clone())
    }

    fn ask(&self, request: Request) -> Result<Answer, ProviderError> {
        self.log
            .lock()
            .expect("log")
            .push(format!("ask:{}", self.name));
        self.seen.lock().expect("seen").push(request);
        match &self.plan {
            Plan::Now(Choice::Allow) => Ok(Answer::allow()),
            Plan::Now(Choice::Reject) => Ok(Answer::reject()),
            Plan::Abstain => Ok(Answer::abstain("none")),
            Plan::Fails => Err(canoa::status::unavailable("down")),
            Plan::Later { .. } => {
                *self.started.lock().expect("started") = Some(Instant::now());
                Ok(Answer::waiting(format!("t-{}", self.name)))
            }
        }
    }

    fn wait(&self, wait: Ticket) -> Result<Answer, ProviderError> {
        let Plan::Later { after, choice } = &self.plan else {
            return Ok(Answer::abstain("gone"));
        };
        let started = self.started.lock().expect("started").expect("asked first");
        let due = *after;
        let left = due.saturating_sub(started.elapsed());
        std::thread::sleep(left.min(Duration::from_millis(wait.wait_ms)));
        if started.elapsed() >= due {
            return Ok(match choice {
                Some(Choice::Allow) => Answer::allow(),
                Some(Choice::Reject) => Answer::reject(),
                None => Answer::waiting(wait.ticket),
            });
        }
        Ok(Answer::waiting(wait.ticket))
    }

    fn withdraw(&self, ticket: Ticket) -> Result<(), ProviderError> {
        self.log
            .lock()
            .expect("log")
            .push(format!("withdraw:{}:{}", self.name, ticket.ticket));
        Ok(())
    }
}

type Log = Arc<Mutex<Vec<String>>>;
type Seen = Arc<Mutex<Vec<Request>>>;

struct Rig {
    log: Arc<Mutex<Vec<String>>>,
    seen: Arc<Mutex<Vec<Request>>>,
    members: Vec<(Entry, Arc<dyn Approver>)>,
}

fn entry(name: &str) -> Entry {
    Entry {
        name: name.to_owned(),
        provider: "stub".to_owned(),
        settings: serde_json::Map::new(),
        notifier: None,
        detail: Detail::Tool,
        timeout: None,
        only_when_away: false,
    }
}

impl Rig {
    fn new() -> Self {
        Self {
            log: Arc::default(),
            seen: Arc::default(),
            members: Vec::new(),
        }
    }

    fn add(&mut self, entry: Entry, authority: Authority, instant: bool, plan: Plan) {
        let stub = Stub {
            name: entry.name.clone(),
            info: ApproverInfo::new("Stub", authority, instant),
            plan,
            started: Mutex::new(None),
            log: Arc::clone(&self.log),
            seen: Arc::clone(&self.seen),
        };
        self.members.push((entry, Arc::new(stub)));
    }

    /// A rule-like approver that says it could use the command and files.
    fn add_wanting_detail(&mut self, entry: Entry, plan: Plan) {
        let mut info = ApproverInfo::new("Stub", Authority::Policy, true);
        info.wants_detail = true;
        let stub = Stub {
            name: entry.name.clone(),
            info,
            plan,
            started: Mutex::new(None),
            log: Arc::clone(&self.log),
            seen: Arc::clone(&self.seen),
        };
        self.members.push((entry, Arc::new(stub)));
    }

    fn chain(self, window_timeout: Option<Duration>) -> (Chain, Log, Seen) {
        let chain = Chain::of(self.members, window_timeout, Duration::from_millis(10));
        (chain, self.log, self.seen)
    }
}

fn request() -> Request {
    let mut tool = Tool::new("edit", "Edit", "Edit a file");
    tool.command = Some("secret command".to_owned());
    tool.paths = vec!["/work/a.rs".to_owned()];
    Request::new("r1", SessionRef::new("s1", "claude"), tool)
}

fn present(window: bool) -> Presence {
    Presence {
        window,
        attended: false,
    }
}

/// A settle callback and the channel it reports to.
fn settling() -> (Settle, mpsc::Receiver<Settled>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    (
        Arc::new(move |settled| {
            let _ = tx.lock().expect("sender").send(settled);
        }),
        rx,
    )
}

fn lines(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().expect("log").clone()
}

fn eventually(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

#[test]
fn instant_approvers_are_asked_in_the_files_order_and_the_first_verdict_ends_it() {
    let mut rig = Rig::new();
    rig.add(entry("a"), Authority::Policy, true, Plan::Abstain);
    rig.add(
        entry("b"),
        Authority::Policy,
        true,
        Plan::Now(Choice::Reject),
    );
    rig.add(
        entry("c"),
        Authority::Policy,
        true,
        Plan::Now(Choice::Allow),
    );
    rig.add(
        entry("d"),
        Authority::Person,
        false,
        Plan::Now(Choice::Allow),
    );
    let (chain, log, _) = rig.chain(None);
    assert_eq!(
        chain.instant(&request(), true),
        Some(("b".to_owned(), Choice::Reject))
    );
    assert_eq!(
        lines(&log),
        ["ask:a", "ask:b"],
        "c and the waiting d are not asked"
    );
}

#[test]
fn an_approver_that_fails_or_abstains_answers_nothing() {
    let mut rig = Rig::new();
    rig.add(entry("a"), Authority::Policy, true, Plan::Fails);
    rig.add(entry("b"), Authority::Policy, true, Plan::Abstain);
    let (chain, log, _) = rig.chain(None);
    assert_eq!(chain.instant(&request(), true), None);
    assert_eq!(lines(&log), ["ask:a", "ask:b"]);
}

#[test]
fn a_policy_never_allows_a_protected_request_and_a_person_does() {
    let mut protected = request();
    protected.protected = true;
    let mut rig = Rig::new();
    rig.add(
        entry("rule"),
        Authority::Policy,
        true,
        Plan::Now(Choice::Allow),
    );
    let (chain, _, _) = rig.chain(None);
    assert_eq!(chain.instant(&protected, true), None);
    assert_eq!(
        chain.instant(&request(), true),
        Some(("rule".to_owned(), Choice::Allow)),
        "the same rule allows what is not protected"
    );

    let mut rig = Rig::new();
    rig.add(
        entry("rule"),
        Authority::Policy,
        true,
        Plan::Now(Choice::Reject),
    );
    let (chain, _, _) = rig.chain(None);
    assert_eq!(
        chain.instant(&protected, true),
        Some(("rule".to_owned(), Choice::Reject)),
        "a reject applies to a protected place"
    );

    let mut rig = Rig::new();
    rig.add(
        entry("phone"),
        Authority::Person,
        true,
        Plan::Now(Choice::Allow),
    );
    let (chain, _, _) = rig.chain(None);
    assert_eq!(
        chain.instant(&protected, true),
        Some(("phone".to_owned(), Choice::Allow))
    );
}

#[test]
fn an_allow_with_no_unambiguous_allow_once_is_not_applied_and_a_reject_is() {
    let mut rig = Rig::new();
    rig.add(
        entry("a"),
        Authority::Person,
        true,
        Plan::Now(Choice::Allow),
    );
    rig.add(
        entry("b"),
        Authority::Person,
        true,
        Plan::Now(Choice::Reject),
    );
    let (chain, _, _) = rig.chain(None);
    assert_eq!(
        chain.instant(&request(), false),
        Some(("b".to_owned(), Choice::Reject))
    );
}

#[test]
fn an_approver_sees_the_command_and_files_only_when_its_entry_says_so_whatever_the_provider_asks_for()
 {
    let mut rig = Rig::new();
    rig.add(entry("plain"), Authority::Policy, true, Plan::Abstain);
    rig.add_wanting_detail(entry("hungry"), Plan::Abstain);
    let mut detailed = entry("detailed");
    detailed.detail = Detail::Command;
    rig.add(detailed, Authority::Policy, true, Plan::Abstain);
    let (chain, _, seen) = rig.chain(None);
    assert_eq!(chain.instant(&request(), true), None);
    let seen = seen.lock().expect("seen");
    assert_eq!(seen[0].tool.command, None);
    assert!(seen[0].tool.paths.is_empty());
    assert_eq!(seen[0].tool.title, "Edit a file");
    // The provider says it could use the detail, and its entry says tool: it is shown the tool alone.
    assert_eq!(seen[1].tool.command, None);
    assert!(seen[1].tool.paths.is_empty());
    assert_eq!(seen[2].tool.command.as_deref(), Some("secret command"));
    assert_eq!(seen[2].tool.paths, ["/work/a.rs"]);
}

#[test]
fn the_first_answer_among_waiting_approvers_wins_and_the_others_are_withdrawn() {
    let mut rig = Rig::new();
    rig.add(
        entry("slow"),
        Authority::Person,
        false,
        Plan::Later {
            after: Duration::from_secs(60),
            choice: Some(Choice::Allow),
        },
    );
    rig.add(
        entry("fast"),
        Authority::Person,
        false,
        Plan::Later {
            after: Duration::from_millis(30),
            choice: Some(Choice::Reject),
        },
    );
    let (chain, log, _) = rig.chain(None);
    let (settle, rx) = settling();
    let asking = chain
        .asking(&request(), true, present(true), settle)
        .expect("someone to wait for");
    let settled = rx.recv_timeout(Duration::from_secs(5)).expect("settled");
    assert_eq!(
        settled,
        Settled {
            by: "fast".to_owned(),
            outcome: Outcome::Rejected
        }
    );
    assert!(eventually(
        || lines(&log).contains(&"withdraw:slow:t-slow".to_owned())
    ));
    assert!(
        !lines(&log)
            .iter()
            .any(|line| line.starts_with("withdraw:fast"))
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "settled once"
    );
    assert!(asking.is_closed());
}

#[test]
fn a_window_that_answers_first_withdraws_every_ticket_and_nothing_settles() {
    let mut rig = Rig::new();
    for name in ["a", "b"] {
        rig.add(
            entry(name),
            Authority::Person,
            false,
            Plan::Later {
                after: Duration::from_millis(300),
                choice: Some(Choice::Allow),
            },
        );
    }
    let (chain, log, _) = rig.chain(None);
    let (settle, rx) = settling();
    let asking = chain
        .asking(&request(), true, present(true), settle)
        .expect("waiting");
    assert!(eventually(|| lines(&log).len() == 2), "both asked");
    asking.end();
    assert!(eventually(|| {
        let log = lines(&log);
        log.contains(&"withdraw:a:t-a".to_owned()) && log.contains(&"withdraw:b:t-b".to_owned())
    }));
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "an answer after the window's settles nothing"
    );
}

#[test]
fn every_approver_timing_out_refuses_with_no_window_and_leaves_it_with_one() {
    for window in [false, true] {
        let mut rig = Rig::new();
        let mut waits = entry("phone");
        waits.timeout = Some(Duration::from_millis(60));
        rig.add(
            waits,
            Authority::Person,
            false,
            Plan::Later {
                after: Duration::from_secs(60),
                choice: Some(Choice::Allow),
            },
        );
        let (chain, log, _) = rig.chain(None);
        let (settle, rx) = settling();
        let asking = chain
            .asking(&request(), true, present(window), settle)
            .expect("waiting");
        assert!(eventually(
            || lines(&log).contains(&"withdraw:phone:t-phone".to_owned())
        ));
        if window {
            assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
            assert!(!asking.is_closed(), "the request stays with the window");
        } else {
            let settled = rx.recv_timeout(Duration::from_secs(5)).expect("refused");
            assert_eq!(settled.outcome, Outcome::Refused);
            assert_ne!(settled.outcome, Outcome::Allowed, "a timeout never allows");
        }
    }
}

#[test]
fn a_window_timeout_refuses_a_request_the_window_leaves_and_an_answer_before_it_wins() {
    let mut rig = Rig::new();
    rig.add(
        entry("phone"),
        Authority::Person,
        false,
        Plan::Later {
            after: Duration::from_secs(60),
            choice: None,
        },
    );
    let (chain, _, _) = rig.chain(Some(Duration::from_millis(80)));
    let (settle, rx) = settling();
    let _asking = chain
        .asking(&request(), true, present(true), settle)
        .expect("waiting");
    let settled = rx.recv_timeout(Duration::from_secs(5)).expect("refused");
    assert_eq!(settled.by, "window");
    assert_eq!(settled.outcome, Outcome::Refused);

    let mut rig = Rig::new();
    rig.add(
        entry("phone"),
        Authority::Person,
        false,
        Plan::Later {
            after: Duration::from_millis(10),
            choice: Some(Choice::Allow),
        },
    );
    let (chain, _, _) = rig.chain(Some(Duration::from_secs(30)));
    let (settle, rx) = settling();
    let _asking = chain
        .asking(&request(), true, present(true), settle)
        .expect("waiting");
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5))
            .expect("allowed")
            .outcome,
        Outcome::Allowed
    );
}

#[test]
fn an_entry_that_asks_only_when_the_person_is_away_is_not_asked_while_a_window_attends() {
    let mut rig = Rig::new();
    let mut away = entry("phone");
    away.only_when_away = true;
    rig.add(away, Authority::Person, false, Plan::Now(Choice::Allow));
    let (chain, log, _) = rig.chain(None);
    let (settle, _rx) = settling();
    let attended = Presence {
        window: true,
        attended: true,
    };
    assert!(
        chain
            .asking(&request(), true, attended, Arc::clone(&settle))
            .is_none()
    );
    assert!(lines(&log).is_empty());
    assert!(
        chain
            .asking(&request(), true, present(true), settle)
            .is_some()
    );
}

#[test]
fn an_instant_approver_is_never_started_as_a_waiting_one() {
    let mut rig = Rig::new();
    rig.add(entry("rule"), Authority::Policy, true, Plan::Abstain);
    let (chain, log, _) = rig.chain(None);
    let (settle, _rx) = settling();
    assert!(
        chain
            .asking(&request(), true, present(false), settle)
            .is_none()
    );
    assert!(lines(&log).is_empty());
}
