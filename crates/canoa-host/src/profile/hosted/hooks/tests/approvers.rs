//! A permission request through the approvers the person added: the built-in `policy`, and a stand-in that waits for someone.

use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use canoa::approver::{Answer, Approver, ApproverInfo, Authority, Request, Ticket};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_transport::{Channel, Failure};

use super::*;
use crate::profile::hosted::Hosted;
use crate::scope::Scope;

/// Answers allow on the first look after it was asked.
#[derive(Default, Debug, Provider)]
#[provider(Approver, id = "soon", crate = ::coqui_acp_api::guatiao)]
struct Soon;

impl Approver for Soon {
    fn describe(&self) -> Result<ApproverInfo, ProviderError> {
        Ok(ApproverInfo::new("Soon", Authority::Person, false))
    }

    fn ask(&self, _: Request) -> Result<Answer, ProviderError> {
        Ok(Answer::waiting("soon-1"))
    }

    fn wait(&self, wait: Ticket) -> Result<Answer, ProviderError> {
        std::thread::sleep(Duration::from_millis(wait.wait_ms.min(30)));
        Ok(Answer::allow())
    }

    fn withdraw(&self, _: Ticket) -> Result<(), ProviderError> {
        Ok(())
    }
}

static WITHDRAWN: AtomicUsize = AtomicUsize::new(0);
/// The two tests that count withdrawals run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Never answers; counts the waits it is told to give up.
#[derive(Default, Debug, Provider)]
#[provider(Approver, id = "never", crate = ::coqui_acp_api::guatiao)]
struct Never;

impl Approver for Never {
    fn describe(&self) -> Result<ApproverInfo, ProviderError> {
        Ok(ApproverInfo::new("Never", Authority::Person, false))
    }

    fn ask(&self, _: Request) -> Result<Answer, ProviderError> {
        Ok(Answer::waiting("never-1"))
    }

    fn wait(&self, wait: Ticket) -> Result<Answer, ProviderError> {
        std::thread::sleep(Duration::from_millis(wait.wait_ms.min(20)));
        Ok(Answer::waiting(wait.ticket))
    }

    fn withdraw(&self, _: Ticket) -> Result<(), ProviderError> {
        WITHDRAWN.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-test-approvers",
    version = "0.0.0",
    providers = [Soon, Never]
);

fn stand_ins() -> Arc<Registry> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("canoa-test-approvers", library)
        .expect("registered");
    Arc::new(registry)
}

#[derive(Default)]
struct Client {
    to: Mutex<Vec<Vec<u8>>>,
}

impl Channel for Client {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.to.lock().expect("lock").push(message.to_vec());
        Ok(())
    }

    fn receive(&self, _timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        Ok(None)
    }

    fn close(&self) {}
}

/// A connection whose profile carries `permissions` (the text of canoa.cfg's `[permissions]` tables), with the stand-ins offered.
fn connection(temp: &Temp, permissions: &str) -> Connection {
    let config = Config::from_str_in(
        &format!(
            "[host.agents.a]\nprogram = \"agent\"\nprofiles = [\"main\"]\n\
             [host.profiles.main]\noverlays = []\n{permissions}"
        ),
        &temp.0,
    )
    .expect("parses");
    let crate::config::EntryKind::Agent { profiles, .. } = &config.entries[0].kind else {
        panic!("an agent");
    };
    let mut held = config.profiles.clone();
    for profile in held.values_mut() {
        profile.items = profile.items.clone().with_registries(vec![stand_ins()]);
    }
    let offer = Arc::into_inner(
        offer(profiles, &held, None, "launcher".into(), Vec::new()).expect("an offer"),
    )
    .expect("one owner");
    Connection::prepare(&offer, 0, "a", &real).expect("prepared")
}

fn options() -> Value {
    json!([
        {"optionId": "yes", "name": "Allow", "kind": "allow_once"},
        {"optionId": "yes-always", "name": "Always", "kind": "allow_always"},
        {"optionId": "no", "name": "Reject", "kind": "reject_once"}
    ])
}

fn ask(id: u64, call: Value) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission",
        "params": {"sessionId": "s", "toolCall": call, "options": options()}}),
    )
    .expect("JSON")
}

fn edit_of(path: &str) -> Value {
    json!({"toolCallId": "c1", "title": "Edit", "kind": "edit", "rawInput": {"file_path": path}})
}

fn run_of(command: &str) -> Value {
    json!({"toolCallId": "c2", "title": "Run", "kind": "execute", "rawInput": {"command": command}})
}

fn chosen(answer: Option<Vec<u8>>) -> Option<String> {
    let answer: Value = serde_json::from_slice(&answer?).expect("JSON");
    answer["result"]["outcome"]["optionId"]
        .as_str()
        .map(str::to_owned)
}

const RULES: &str = "[permissions.approvers.rules]\nprovider = \"policy\"\n\
    settings = { rules = [\
    { kind = \"execute\", command = \"rm *\", verdict = \"reject\" },\
    { kind = \"edit\", verdict = \"allow\" },\
    { kind = \"execute\", command = \"git status*\", verdict = \"allow\" } ] }\n";

#[test]
fn a_policy_answers_for_the_person_with_the_option_that_decides_once() {
    let temp = Temp::new();
    let connection = connection(&temp, RULES);
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(1, edit_of("src/lib.rs")));
    assert!(matches!(seen, Seen::Dropped), "{seen:?}");
    assert_eq!(
        chosen(hosted.hold().book.next_injected()).as_deref(),
        Some("yes")
    );
    let seen = hosted.on_agent(&ask(2, run_of("rm -rf build")));
    assert!(matches!(seen, Seen::Dropped));
    assert_eq!(
        chosen(hosted.hold().book.next_injected()).as_deref(),
        Some("no")
    );
    let seen = hosted.on_agent(&ask(3, run_of("git status -s")));
    assert!(matches!(seen, Seen::Dropped));
    assert_eq!(
        chosen(hosted.hold().book.next_injected()).as_deref(),
        Some("yes")
    );
}

#[test]
fn a_request_no_rule_matches_goes_to_the_window_as_it_would_have() {
    let temp = Temp::new();
    let connection = connection(&temp, RULES);
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(1, run_of("curl example.test")));
    assert!(matches!(seen, Seen::Passed(None)), "{seen:?}");
    assert_eq!(hosted.hold().book.next_injected(), None);
}

#[test]
fn a_policy_never_allows_a_protected_path_though_its_rule_names_the_call() {
    let temp = Temp::new();
    let connection = connection(&temp, RULES);
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(1, edit_of(".env")));
    assert!(matches!(seen, Seen::Passed(None)), "{seen:?}");
    assert_eq!(hosted.hold().book.next_injected(), None);
    // The same rule allows the same kind of call at a place that is not protected.
    let seen = hosted.on_agent(&ask(2, edit_of("src/lib.rs")));
    assert!(matches!(seen, Seen::Dropped));
}

#[test]
fn a_policy_that_rejects_a_protected_path_is_applied() {
    let temp = Temp::new();
    let rules = "[permissions.approvers.rules]\nprovider = \"policy\"\n\
        settings = { rules = [ { kind = \"edit\", verdict = \"reject\" } ] }\n";
    let connection = connection(&temp, rules);
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(1, edit_of(".env")));
    assert!(matches!(seen, Seen::Dropped));
    assert_eq!(
        chosen(hosted.hold().book.next_injected()).as_deref(),
        Some("no")
    );
}

#[test]
fn with_no_approver_added_the_window_alone_answers() {
    let temp = Temp::new();
    let connection = connection(&temp, "");
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(1, edit_of("src/lib.rs")));
    assert!(matches!(seen, Seen::Passed(None)));
    assert!(hosted.hold().book.waiting() == 0);
}

fn settle_until(hosted: &Hosted<'_, Client>, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        hosted.settle_approvals().expect("settled");
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn an_approver_that_answers_after_the_request_went_out_answers_the_agent_and_closes_the_card() {
    let temp = Temp::new();
    let connection = connection(
        &temp,
        "[permissions.approvers.phone]\nprovider = \"soon\"\n",
    );
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let seen = hosted.on_agent(&ask(7, edit_of("src/lib.rs")));
    assert!(matches!(seen, Seen::Passed(None)), "the window gets it too");
    assert_eq!(hosted.hold().book.waiting(), 1);
    let mut answer = None;
    assert!(settle_until(&hosted, || {
        answer = hosted.hold().book.next_injected();
        answer.is_some()
    }));
    assert_eq!(chosen(answer).as_deref(), Some("yes"));
    let methods: Vec<String> = client
        .to
        .lock()
        .expect("lock")
        .iter()
        .filter_map(|message| serde_json::from_slice::<Value>(message).ok())
        .filter_map(|message| message["method"].as_str().map(str::to_owned))
        .collect();
    assert!(
        methods.contains(&"_dev.boriken.canoa/approved".to_owned()),
        "{methods:?}"
    );
    // The window's own answer arrives after: the agent has its answer already, so it is not sent twice.
    let late = json!({"jsonrpc": "2.0", "id": 7, "result": {"outcome": {"outcome": "selected", "optionId": "no"}}});
    let late = serde_json::to_vec(&late).expect("JSON");
    assert!(hosted.on_client(late, &BTreeSet::new()).is_err());
}

#[test]
fn a_window_that_answers_first_ends_the_wait_and_the_approvers_answer_is_never_used() {
    let temp = Temp::new();
    let connection = connection(
        &temp,
        "[permissions.approvers.slow]\nprovider = \"never\"\n",
    );
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let _alone = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = WITHDRAWN.load(Ordering::SeqCst);
    let seen = hosted.on_agent(&ask(9, edit_of("src/lib.rs")));
    assert!(matches!(seen, Seen::Passed(None)));
    let window = json!({"jsonrpc": "2.0", "id": 9, "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}});
    let window = serde_json::to_vec(&window).expect("JSON");
    let forwarded = hosted.on_client(window.clone(), &BTreeSet::new());
    assert_eq!(forwarded.expect("goes on to the agent"), window);
    assert!(hosted.hold().book.waiting() == 0);
    let until = Instant::now() + Duration::from_secs(5);
    while WITHDRAWN.load(Ordering::SeqCst) == before && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        WITHDRAWN.load(Ordering::SeqCst) > before,
        "its ticket was withdrawn"
    );
    hosted.settle_approvals().expect("settled");
    assert_eq!(hosted.hold().book.next_injected(), None);
}

#[test]
fn a_cancelled_turn_and_a_closed_session_withdraw_what_approvers_wait_for() {
    let temp = Temp::new();
    let connection = connection(
        &temp,
        "[permissions.approvers.slow]\nprovider = \"never\"\n",
    );
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let _alone = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = WITHDRAWN.load(Ordering::SeqCst);
    assert!(matches!(
        hosted.on_agent(&ask(1, edit_of("src/lib.rs"))),
        Seen::Passed(None)
    ));
    let cancel =
        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "s"}});
    let _ = hosted.on_client(serde_json::to_vec(&cancel).expect("JSON"), &BTreeSet::new());
    assert!(hosted.hold().book.waiting() == 0);
    let until = Instant::now() + Duration::from_secs(5);
    while WITHDRAWN.load(Ordering::SeqCst) == before && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(WITHDRAWN.load(Ordering::SeqCst) > before);
}
