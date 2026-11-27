//! The hub, the notifiers it tells and the observer that fires the events of a connection's sessions.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa::notifier::{
    Delivery, Notice, Notifier, NotifierError, NotifierInfo, SendRequest, SessionRef,
};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_transport::{Channel, Failure};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::directory::Directory;
use super::hub::{Delivered, Hub, Occurrence};
use super::observer::Observed;
use crate::config::{NotificationsConfig, NotifierConfig, Settings};

/// What the recording notifier was sent: its `topic` setting, so each test reads only its own, and the notice.
static RECORDS: Mutex<Vec<(String, Notice)>> = Mutex::new(Vec::new());

/// The `_services` setting each recording carried, by topic: what a provider is handed to reach the host's session services.
static HANDLES: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

fn topic_of(settings: &str) -> String {
    serde_json::from_str::<Value>(settings)
        .ok()
        .and_then(|value| {
            value
                .get("topic")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Keeps every notice it is sent.
#[derive(Default, Provider)]
#[provider(Notifier, id = "recorder_notifier", crate = ::coqui_acp_api::guatiao)]
struct Recorder;

impl Notifier for Recorder {
    fn describe(&self) -> Result<NotifierInfo, ProviderError> {
        Ok(NotifierInfo::new("recorder", "Recorder"))
    }

    fn send(&self, request: SendRequest) -> Result<Delivery, ProviderError> {
        RECORDS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((topic_of(&request.settings), request.notice.clone()));
        let handle = serde_json::from_str::<Value>(&request.settings)
            .ok()
            .and_then(|value| {
                value
                    .get("_services")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        HANDLES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((topic_of(&request.settings), handle));
        Ok(Delivery::new(request.notice.id))
    }
}

/// Refuses every notice.
#[derive(Default, Provider)]
#[provider(Notifier, id = "failing_notifier", crate = ::coqui_acp_api::guatiao)]
struct Failing;

impl Notifier for Failing {
    fn describe(&self) -> Result<NotifierInfo, ProviderError> {
        Ok(NotifierInfo::new("failing", "Failing"))
    }

    fn send(&self, _request: SendRequest) -> Result<Delivery, ProviderError> {
        Err(NotifierError::Rejected("the service said no".to_owned()).into())
    }
}

/// Takes only the events of a turn that failed.
#[derive(Default, Provider)]
#[provider(Notifier, id = "alarm_notifier", crate = ::coqui_acp_api::guatiao)]
struct Alarm;

impl Notifier for Alarm {
    fn describe(&self) -> Result<NotifierInfo, ProviderError> {
        let mut info = NotifierInfo::new("alarm", "Alarm");
        info.events = Some(vec!["failed".to_owned()]);
        Ok(info)
    }

    fn send(&self, request: SendRequest) -> Result<Delivery, ProviderError> {
        RECORDS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                format!("alarm:{}", topic_of(&request.settings)),
                request.notice.clone(),
            ));
        Ok(Delivery::new(request.notice.id))
    }
}

mod library {
    use super::{Alarm, Failing, Recorder};

    coqui_acp_api::guatiao::local_providers!(
        id = "notify-standins",
        version = "1.0.0",
        providers = [Recorder, Failing, Alarm]
    );
}

fn registry() -> Registry {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("notify-standins", library::library)
        .expect("registers");
    assert!(loading.loaded().is_some());
    registry
}

fn entry(provider: &str, topic: &str) -> NotifierConfig {
    let mut settings = serde_json::Map::new();
    settings.insert("topic".to_owned(), json!(topic));
    NotifierConfig {
        provider: provider.to_owned(),
        origin: None,
        settings: Settings::new(settings),
    }
}

/// A hub whose `[notifications]` is `routes` over the entries `topic` names, each one of the stand-in providers.
fn hub(topic: &str, routes: &[(&str, &[&str])]) -> Arc<Hub> {
    let mut config = NotificationsConfig::default();
    config
        .notifiers
        .insert("rec".to_owned(), entry("recorder", topic));
    config
        .notifiers
        .insert("bad".to_owned(), entry("failing", topic));
    config
        .notifiers
        .insert("alarm".to_owned(), entry("alarm", topic));
    let mut table: IndexMap<String, Vec<String>> = IndexMap::new();
    for (event, names) in routes {
        table.insert(
            (*event).to_owned(),
            names.iter().map(|name| (*name).to_owned()).collect(),
        );
    }
    config.routes = table;
    Hub::fixed(Directory::from_registries(
        config,
        vec![("standins".to_owned(), registry())],
        None,
    ))
}

fn recorded(topic: &str) -> Vec<Notice> {
    RECORDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(held, _)| held == topic)
        .map(|(_, notice)| notice.clone())
        .collect()
}

fn session(id: &str) -> Option<SessionRef> {
    let mut reference = SessionRef::new(id);
    reference.title = Some("Fix the build".to_owned());
    reference.agent = Some("Claude".to_owned());
    Some(reference)
}

fn about(id: &str, request: Option<&str>) -> Occurrence {
    Occurrence {
        session: session(id),
        request: request.map(str::to_owned),
        figure: None,
    }
}

fn told(hub: &Arc<Hub>, event: &str, occurrence: Occurrence) -> Vec<Delivered> {
    hub.fire(event, occurrence)
        .map(|thread| thread.join().expect("the thread ends"))
        .unwrap_or_default()
}

#[test]
fn each_event_goes_to_the_notifiers_its_route_names_and_to_none_when_the_list_is_empty() {
    let hub = hub(
        "routes",
        &[
            ("permission", &["rec"]),
            ("question", &[]),
            ("finished", &["rec"]),
        ],
    );
    let done = told(&hub, "permission", about("s1", None));
    assert_eq!(
        done,
        [Delivered {
            notifier: "rec".to_owned(),
            outcome: Ok(())
        }]
    );
    assert!(told(&hub, "question", about("s1", None)).is_empty());
    told(&hub, "finished", about("s1", None));
    let notices = recorded("routes");
    assert_eq!(
        notices
            .iter()
            .map(|each| each.event.as_str())
            .collect::<Vec<_>>(),
        ["permission", "finished"]
    );
    let first = &notices[0];
    assert_eq!(first.title, "Fix the build");
    assert_eq!(first.body, "Claude asks leave to use a tool");
    assert_eq!(first.urgency, canoa::notifier::Urgency::High);
    assert_eq!(
        first.session.as_ref().map(|each| each.id.as_str()),
        Some("s1")
    );
}

#[test]
fn an_event_the_file_says_nothing_of_goes_where_the_catalog_says_and_the_two_host_events_go_nowhere()
 {
    let hub = hub("defaults", &[]);
    // The default route is the desktop, which no window is open to take: answered, not delivered.
    let done = told(&hub, "failed", about("s1", None));
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].notifier, "desktop");
    assert_eq!(done[0].outcome, Err("no window is open".to_owned()));
    assert!(told(&hub, "host_restarted", Occurrence::default()).is_empty());
    assert!(told(&hub, "host_updated", Occurrence::default()).is_empty());
    let hub = self::hub(
        "hostroutes",
        &[("host_restarted", &["rec"]), ("host_updated", &["rec"])],
    );
    told(&hub, "host_restarted", Occurrence::default());
    told(
        &hub,
        "host_updated",
        Occurrence {
            figure: Some("2.0".to_owned()),
            ..Occurrence::default()
        },
    );
    let notices = recorded("hostroutes");
    assert_eq!(notices.len(), 2);
    assert!(
        notices
            .iter()
            .all(|each| each.session.is_none() && each.title == "Canoa")
    );
    assert_eq!(notices[0].urgency, canoa::notifier::Urgency::Low);
    assert_eq!(notices[1].body, "The host was updated 2.0");
}

#[test]
fn an_event_the_catalog_lacks_is_not_fired() {
    let hub = hub("nosuch", &[]);
    assert!(hub.fire("no_such_event", Occurrence::default()).is_none());
}

#[test]
fn one_request_tells_the_person_once() {
    let hub = hub("dedupe", &[("permission", &["rec"])]);
    told(&hub, "permission", about("s1", Some("7")));
    assert!(told(&hub, "permission", about("s1", Some("7"))).is_empty());
    told(&hub, "permission", about("s1", Some("8")));
    told(&hub, "permission", about("s2", Some("7")));
    assert_eq!(recorded("dedupe").len(), 3);
}

#[test]
fn a_session_the_person_is_looking_at_is_not_told_and_another_is() {
    let hub = hub("attended", &[("finished", &["rec"])]);
    let window = hub.subscribe("en");
    *window
        .attended
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some("s1".to_owned());
    assert!(told(&hub, "finished", about("s1", Some("1"))).is_empty());
    assert_eq!(told(&hub, "finished", about("s2", Some("1"))).len(), 1);
    // A host event names no session, so no window's attention holds it back.
    assert_eq!(told(&hub, "finished", Occurrence::default()).len(), 1);
    // The window looked away, then the link ended.
    *window
        .attended
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    assert_eq!(told(&hub, "finished", about("s1", Some("2"))).len(), 1);
    *window
        .attended
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some("s1".to_owned());
    hub.unsubscribe(&window);
    assert_eq!(told(&hub, "finished", about("s1", Some("3"))).len(), 1);
    assert_eq!(recorded("attended").len(), 4);
}

#[test]
fn a_notifier_that_fails_does_not_stop_the_next_and_a_notifier_takes_only_its_events() {
    let hub = hub(
        "failing",
        &[
            ("failed", &["bad", "rec", "alarm"]),
            ("finished", &["alarm", "rec"]),
        ],
    );
    let done = told(&hub, "failed", about("s1", None));
    assert_eq!(done.len(), 3);
    assert_eq!(done[0].notifier, "bad");
    assert!(
        done[0]
            .outcome
            .as_ref()
            .is_err_and(|why| why.contains("the service said no"))
    );
    assert!(done[1..].iter().all(|each| each.outcome.is_ok()));
    assert_eq!(recorded("failing").len(), 1);
    assert_eq!(recorded("alarm:failing").len(), 1);
    // The alarm takes no `finished`: it is skipped, and the recorder still gets it.
    let done = told(&hub, "finished", about("s1", None));
    assert_eq!(
        done.iter()
            .map(|each| each.notifier.as_str())
            .collect::<Vec<_>>(),
        ["rec"]
    );
    assert_eq!(recorded("alarm:failing").len(), 1);
}

#[test]
fn a_name_that_is_no_notifier_is_a_failure_of_that_name_only() {
    let hub = hub("unknown", &[("failed", &["ghost", "rec"])]);
    let done = told(&hub, "failed", about("s1", None));
    assert_eq!(done.len(), 2);
    assert!(
        done[0]
            .outcome
            .as_ref()
            .is_err_and(|why| why.contains("ghost"))
    );
    assert!(done[1].outcome.is_ok());
}

#[test]
fn the_desktop_notifier_pushes_the_notice_to_every_subscribed_window() {
    let hub = hub("desktop", &[("question", &["desktop"])]);
    let first = hub.subscribe("en");
    let second = hub.subscribe("es");
    let done = told(&hub, "question", about("s9", Some("q1")));
    assert_eq!(done[0].outcome, Ok(()));
    for window in [&first, &second] {
        let lines = window.take();
        assert_eq!(lines.len(), 1);
        let line: Value = serde_json::from_slice(&lines[0]).expect("JSON");
        assert_eq!(line["method"], "notify/notice");
        assert_eq!(line["params"]["event"], "question");
        assert_eq!(line["params"]["session"]["id"], "s9");
        assert_eq!(line["params"]["urgency"], "high");
        assert!(line["params"].get("actions").is_none());
        assert!(window.take().is_empty());
    }
    hub.unsubscribe(&first);
    hub.unsubscribe(&second);
    let done = told(&hub, "question", about("s9", Some("q2")));
    assert_eq!(done[0].outcome, Err("no window is open".to_owned()));
}

#[test]
fn a_test_notice_reaches_one_notifier_whatever_the_routes_say() {
    let hub = hub("testing", &[]);
    hub.test("rec").expect("delivered");
    let notices = recorded("testing");
    assert_eq!(notices.len(), 1);
    assert_eq!(
        (notices[0].title.as_str(), notices[0].body.as_str()),
        ("Canoa", "Test")
    );
    assert!(notices[0].session.is_none());
    assert!(
        hub.test("bad")
            .is_err_and(|why| why.contains("the service said no"))
    );
    assert!(hub.test("ghost").is_err());
}

#[test]
fn a_provider_is_handed_the_handle_of_the_hosts_services_and_none_when_the_host_has_none() {
    let handles = |topic: &str| -> Vec<Option<String>> {
        HANDLES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(held, _)| held == topic)
            .map(|(_, handle)| handle.clone())
            .collect()
    };
    let with = |topic: &str, services: Option<&str>| {
        let mut config = NotificationsConfig::default();
        config
            .notifiers
            .insert("rec".to_owned(), entry("recorder", topic));
        let directory = Arc::new(Directory::from_registries(
            config,
            vec![("standins".to_owned(), registry())],
            None,
        ));
        Hub::new(
            Box::new(move || Ok(Arc::clone(&directory))),
            services.map(str::to_owned),
        )
    };
    with("handle-some", Some("host-42"))
        .test("rec")
        .expect("delivered");
    with("handle-none", None).test("rec").expect("delivered");
    assert_eq!(handles("handle-some"), [Some("host-42".to_owned())]);
    assert_eq!(handles("handle-none"), [None]);
}

/// A channel with a queue of what the client sends and a record of what reaches it.
struct Pipe {
    from_client: Mutex<VecDeque<Vec<u8>>>,
    to_client: Mutex<Vec<Vec<u8>>>,
}

impl Pipe {
    fn new() -> Self {
        Self {
            from_client: Mutex::new(VecDeque::new()),
            to_client: Mutex::new(Vec::new()),
        }
    }

    fn client_says(&self, message: &Value) {
        self.from_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(message.to_string().into_bytes());
    }
}

impl Channel for Pipe {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.to_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.to_vec());
        Ok(())
    }

    fn receive(&self, _timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        Ok(self
            .from_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }

    fn close(&self) {}

    fn set_max_message(&self, _bytes: usize) {}
}

/// Runs the client's `prompt` and the agent's messages `agent` over an observed channel, and answers what the recorder was sent.
fn watched(topic: &str, agent: &[Value], ended: bool) -> Vec<Notice> {
    let routes: Vec<(&str, &[&str])> = ["permission", "question", "finished", "failed"]
        .into_iter()
        .map(|event| (event, &["rec"][..]))
        .collect();
    let hub = hub(topic, &routes);
    let pipe = Pipe::new();
    let observed = Observed::new(&pipe, Arc::clone(&hub), "Claude");
    pipe.client_says(&json!({"jsonrpc": "2.0", "id": 5, "method": "session/prompt", "params": {"sessionId": "s1", "prompt": []}}));
    observed.receive(None).expect("received");
    for message in agent {
        observed.send(message.to_string().as_bytes()).expect("sent");
    }
    if ended {
        observed.agent_ended();
    }
    // The events fire on threads of their own; the hub's dedupe is the only thing that waits, so wait for what is expected.
    for _ in 0..200 {
        std::thread::sleep(Duration::from_millis(10));
        if !recorded(topic).is_empty() {
            std::thread::sleep(Duration::from_millis(50));
            break;
        }
    }
    let delivered = pipe
        .to_client
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    assert_eq!(
        delivered,
        agent.len(),
        "every message still reaches the client"
    );
    recorded(topic)
}

fn events_of(notices: &[Notice]) -> Vec<&str> {
    notices.iter().map(|each| each.event.as_str()).collect()
}

#[test]
fn a_permission_request_put_to_the_client_fires_permission_with_the_title_the_agent_gave() {
    let notices = watched(
        "observe-permission",
        &[
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": {"sessionUpdate": "session_info_update", "title": "Port the parser"}}}),
            json!({"jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {"sessionId": "s1", "toolCall": {"toolCallId": "t", "title": "rm -rf /secret/path"}, "options": []}}),
        ],
        false,
    );
    assert_eq!(events_of(&notices), ["permission"]);
    assert_eq!(notices[0].title, "Port the parser");
    assert_eq!(notices[0].body, "Claude asks leave to use a tool");
    assert!(
        !format!("{notices:?}").contains("secret"),
        "a command and a path never enter a notice"
    );
}

#[test]
fn an_elicitation_put_to_the_client_fires_question() {
    let notices = watched(
        "observe-question",
        &[
            json!({"jsonrpc": "2.0", "id": 3, "method": "elicitation/create", "params": {"sessionId": "s1", "message": "Which one?"}}),
        ],
        false,
    );
    assert_eq!(events_of(&notices), ["question"]);
    assert_eq!(notices[0].body, "Claude has a question for you");
}

#[test]
fn a_prompt_answered_with_a_stop_reason_fires_finished_and_a_cancelled_one_does_not() {
    let notices = watched(
        "observe-finished",
        &[json!({"jsonrpc": "2.0", "id": 5, "result": {"stopReason": "end_turn"}})],
        false,
    );
    assert_eq!(events_of(&notices), ["finished"]);
    let cancelled = watched(
        "observe-cancelled",
        &[json!({"jsonrpc": "2.0", "id": 5, "result": {"stopReason": "cancelled"}})],
        false,
    );
    assert!(cancelled.is_empty(), "{cancelled:?}");
}

#[test]
fn a_prompt_answered_with_an_error_fires_failed() {
    let notices = watched(
        "observe-failed",
        &[
            json!({"jsonrpc": "2.0", "id": 5, "error": {"code": -32000, "message": "the model refused with this text"}}),
        ],
        false,
    );
    assert_eq!(events_of(&notices), ["failed"]);
    assert!(!format!("{notices:?}").contains("the model refused"));
}

#[test]
fn an_agent_that_ends_while_a_turn_runs_fires_failed_once() {
    let notices = watched("observe-ended", &[], true);
    assert_eq!(events_of(&notices), ["failed"]);
    // A turn that was answered is no longer running when the agent ends.
    let answered = watched(
        "observe-ended-after",
        &[json!({"jsonrpc": "2.0", "id": 5, "result": {"stopReason": "end_turn"}})],
        true,
    );
    assert_eq!(events_of(&answered), ["finished"]);
}
