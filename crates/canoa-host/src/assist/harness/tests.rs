//! The harness backend against the scripted stand-in agent (`stdio_agent --scripted`), run as a process and spoken to over a link,
//! with no model and no login. They need the examples built: `cargo build -p canoa-host --examples`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa::network::Network;
use canoa::sdk::ChatRequest;
use serde_json::Value;

use super::*;
use crate::delegate::Source;
use crate::local::pair;
use crate::serve::LocalError;

const LONG: Duration = Duration::from_secs(20);

/// The scripted agent, each connection a process of its own.
struct Process {
    program: PathBuf,
    args: Vec<String>,
    /// Connections opened.
    opened: AtomicUsize,
    /// Processes running.
    live: Arc<AtomicUsize>,
}

impl Host for Process {
    fn sources(&self) -> Vec<Source> {
        Vec::new()
    }

    fn may_use(&self, _who: &Principal, _source: &str) -> bool {
        true
    }

    fn kind(&self, _source: &str) -> Option<String> {
        None
    }

    fn open(&self, _source: &str, _context: ChildContext) -> Result<Link, LocalError> {
        let (client, host) = pair(1 << 20, Arc::new(AtomicBool::new(false)));
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| LocalError::Thread)?;
        let mut input = child.stdin.take().expect("piped");
        let output = child.stdout.take().expect("piped");
        self.opened.fetch_add(1, Ordering::SeqCst);
        self.live.fetch_add(1, Ordering::SeqCst);
        let host = Arc::new(host);
        let child: Arc<Mutex<Child>> = Arc::new(Mutex::new(child));
        let reading = Arc::clone(&host);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if reading.send(line.as_bytes()).is_err() {
                    break;
                }
            }
            reading.close();
        });
        let live = Arc::clone(&self.live);
        std::thread::spawn(move || {
            while let Ok(Some(message)) = host.receive(None) {
                if input.write_all(&message).is_err() || input.write_all(b"\n").is_err() {
                    break;
                }
                let _ = input.flush();
            }
            let mut child = child.lock().expect("not poisoned");
            let _ = child.kill();
            let _ = child.wait();
            live.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(client)
    }

    fn prover(&self, _peer: &coqui_peer_webcrypto::PeerId) -> Result<cabuya::auth::Proof, String> {
        Err("no proof in a test".to_owned())
    }
}

fn example(name: &str) -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    let profile = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps");
    let path = profile
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "{} is built by `cargo build -p canoa-host --examples`",
        path.display()
    );
    path
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("canoa-harness-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Rig {
    host: Arc<Process>,
    opener: Opener,
    folder: Folder,
}

impl Rig {
    fn new(name: &str) -> Self {
        let folder = Folder::new(name);
        let host = Arc::new(Process {
            program: example("stdio_agent"),
            args: vec![
                "--scripted".to_owned(),
                "--modes=ask,plan".to_owned(),
                format!("--state={}", folder.file("state.json").display()),
            ],
            opened: AtomicUsize::new(0),
            live: Arc::new(AtomicUsize::new(0)),
        });
        let opener = Opener::default();
        let _ = opener.set(host.clone() as Arc<dyn Host>);
        Self {
            host,
            opener,
            folder,
        }
    }

    fn backend(&self, pool: &Arc<Pool>, name: &str, table: HarnessTable) -> HarnessBackend {
        HarnessBackend::new(name, &table, Arc::clone(&self.opener), Arc::clone(pool))
    }

    fn opened(&self) -> usize {
        self.host.opened.load(Ordering::SeqCst)
    }

    /// Waits for the processes running to be `count`.
    fn running(&self, count: usize) {
        let until = Instant::now() + LONG;
        while self.host.live.load(Ordering::SeqCst) != count {
            assert!(
                Instant::now() < until,
                "{} processes run, not {count}",
                self.host.live.load(Ordering::SeqCst)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn sessions(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.folder.file("state.json")).expect("a state"))
            .expect("JSON")
    }
}

fn table(source: &str) -> HarnessTable {
    HarnessTable {
        source: source.to_owned(),
        title: None,
        profile: None,
        account: None,
        model: None,
        effort: None,
    }
}

fn ask(backend: &HarnessBackend, text: &str) -> Result<String, String> {
    ask_within(backend, text, LONG)
}

fn ask_within(backend: &HarnessBackend, text: &str, within: Duration) -> Result<String, String> {
    backend
        .serve(
            &Network::current(),
            Call::Complete(ChatRequest::ask("", text)),
            within,
        )
        .map(|answer| match answer {
            Answer::Complete(reply) => reply.text,
            other => panic!("{other:?}"),
        })
        .map_err(|failure| failure.why)
}

fn pool(idle: Duration, cap: usize) -> Arc<Pool> {
    Pool::with(idle, cap)
}

#[test]
fn a_warm_session_is_reused_and_each_request_is_a_new_conversation() {
    let rig = Rig::new("reuse");
    let pool = pool(Duration::from_secs(60), 2);
    let backend = rig.backend(&pool, "one", table("scripted"));
    assert_eq!(
        ask(&backend, "Say alpha-secret").as_deref(),
        Ok("alpha-secret")
    );
    assert_eq!(ask(&backend, "Say beta").as_deref(), Ok("beta"));
    assert_eq!(rig.opened(), 1, "the second request started no agent");
    assert_eq!(pool.live(), 1);
    // Two conversations in the one agent, and the second holds nothing of the first.
    let sessions = rig.sessions();
    let sessions = sessions.as_object().expect("sessions");
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    let held: Vec<String> = sessions
        .values()
        .map(|session| session["history"].to_string())
        .collect();
    assert!(
        held.iter()
            .any(|text| text.contains("alpha-secret") && !text.contains("beta"))
    );
    assert!(
        held.iter()
            .any(|text| text.contains("beta") && !text.contains("alpha-secret"))
    );
    pool.close();
    rig.running(0);
}

#[test]
fn a_harness_with_another_account_or_profile_is_another_session() {
    let rig = Rig::new("keys");
    let pool = pool(Duration::from_secs(60), 3);
    let plain = rig.backend(&pool, "plain", table("scripted"));
    let mut other = table("scripted");
    other.account = Some("work".to_owned());
    let work = rig.backend(&pool, "work", other);
    let mut third = table("scripted");
    third.profile = Some("terse".to_owned());
    let terse = rig.backend(&pool, "terse", third);
    for backend in [&plain, &work, &terse, &plain, &work, &terse] {
        assert!(ask(backend, "Say hi").is_ok());
    }
    assert_eq!(
        rig.opened(),
        3,
        "one agent for each of the three, each reused"
    );
    pool.close();
    rig.running(0);
}

#[test]
fn the_initialize_names_the_account_and_profile_and_the_empty_loadout_and_gives_the_client_nothing()
{
    let start = Start {
        source: "claude".to_owned(),
        account: Some("work".to_owned()),
        profile: Some("terse".to_owned()),
    };
    let init = start.initialize();
    assert_eq!(init["clientCapabilities"], serde_json::json!({}));
    let meta = init["_meta"].as_object().expect("a _meta");
    assert_eq!(meta[crate::ACCOUNT_META], "work");
    assert_eq!(meta[crate::profile::hosted::PROFILE_META], "terse");
    assert_eq!(meta[crate::profile::hosted::LOADOUT_META], "");
    assert_eq!(meta.len(), 3);
}

#[test]
fn the_session_is_in_the_mode_that_asks_for_the_least_and_takes_the_model_and_effort_asked() {
    let rig = Rig::new("mode");
    let pool = pool(Duration::from_secs(60), 2);
    let plain = rig.backend(&pool, "plain", table("scripted"));
    assert_eq!(ask(&plain, "Mode").as_deref(), Ok("mode: plan"));
    let mut chosen = table("scripted");
    chosen.model = Some("fast".to_owned());
    chosen.effort = Some("high".to_owned());
    let chosen = rig.backend(&pool, "chosen", chosen);
    assert_eq!(
        ask(&chosen, "Chosen").as_deref(),
        Ok("model=fast effort=high")
    );
    // A model the agent does not list is refused by name and the session stays.
    let mut absent = table("scripted");
    absent.model = Some("not-there".to_owned());
    let absent = rig.backend(&pool, "absent", absent);
    let why = ask(&absent, "Say never").expect_err("refused");
    assert!(why.contains("not-there") && why.contains("offers"), "{why}");
    assert_eq!(
        pool.live(),
        1,
        "the agent was kept; one agent serves all three"
    );
    pool.close();
    rig.running(0);
}

#[test]
fn a_request_for_a_tool_is_refused_and_the_agent_is_not_given_one() {
    let rig = Rig::new("tools");
    let start = Start {
        source: "scripted".to_owned(),
        account: None,
        profile: None,
    };
    let Ok(mut conn) = Conn::start(rig.host.as_ref(), &start) else {
        panic!("the agent started")
    };
    let ask = Ask {
        text: "McpCall",
        model: None,
        effort: None,
    };
    // The stand-in asks to run a tool and, refused, says nothing: its answer is empty, which is no answer.
    let outcome = conn.complete(&ask, Instant::now() + LONG);
    assert_eq!(conn.refused, 1, "the permission request was refused");
    assert!(outcome.is_err(), "no text came of a refused tool");
    drop(conn);
    rig.running(0);
}

#[test]
fn a_session_nobody_asked_for_is_ended_when_it_has_been_idle_too_long() {
    let rig = Rig::new("idle");
    let pool = pool(Duration::from_millis(200), 2);
    let backend = rig.backend(&pool, "one", table("scripted"));
    assert!(ask(&backend, "Say a").is_ok());
    // Not idle long enough: it stays.
    pool.reap();
    assert_eq!(pool.live(), 1);
    std::thread::sleep(Duration::from_millis(400));
    backend.reap();
    assert_eq!(pool.live(), 0);
    rig.running(0);
    // The next request starts another.
    assert!(ask(&backend, "Say b").is_ok());
    assert_eq!(rig.opened(), 2);
    pool.close();
    rig.running(0);
}

#[test]
fn the_cap_ends_the_session_idle_longest_to_make_room_and_a_busy_pool_refuses_in_time() {
    let rig = Rig::new("cap");
    let pool = pool(Duration::from_secs(60), 1);
    let first = rig.backend(&pool, "first", table("scripted"));
    let mut other = table("scripted");
    other.account = Some("work".to_owned());
    let second = rig.backend(&pool, "second", other);
    assert!(ask(&first, "Say a").is_ok());
    assert!(ask(&second, "Say b").is_ok());
    assert_eq!(rig.opened(), 2);
    assert_eq!(pool.live(), 1, "never more than the cap");
    rig.running(1);
    // The one session is busy with a turn that waits; another harness cannot have a place and says so when its time is out.
    let busy = Arc::new(first);
    let waiting = {
        let busy = Arc::clone(&busy);
        std::thread::spawn(move || ask_within(&busy, "Wait", Duration::from_secs(4)))
    };
    std::thread::sleep(Duration::from_millis(1000));
    let why = ask_within(&second, "Say c", Duration::from_millis(500)).expect_err("no room");
    assert!(why.contains("busy"), "{why}");
    let ended = waiting.join().expect("no panic");
    assert!(ended.is_err(), "the turn that waited ran out of time");
    pool.close();
    rig.running(0);
}

#[test]
fn requests_for_one_harness_share_its_session_one_at_a_time() {
    let rig = Rig::new("share");
    let pool = pool(Duration::from_secs(60), 1);
    let backend = Arc::new(rig.backend(&pool, "one", table("scripted")));
    let threads: Vec<_> = (0..4)
        .map(|each| {
            let backend = Arc::clone(&backend);
            std::thread::spawn(move || ask(&backend, &format!("Say n{each}")))
        })
        .collect();
    for (each, thread) in threads.into_iter().enumerate() {
        assert_eq!(thread.join().expect("no panic"), Ok(format!("n{each}")));
    }
    assert_eq!(rig.opened(), 1);
    pool.close();
    rig.running(0);
}

#[test]
fn a_session_that_fails_or_answers_wrongly_is_ended_and_replaced() {
    let rig = Rig::new("replace");
    let pool = pool(Duration::from_secs(60), 2);
    let backend = rig.backend(&pool, "one", table("scripted"));
    assert!(ask(&backend, "Say fine").is_ok());
    assert_eq!(rig.opened(), 1);
    // An error from the agent: the session goes.
    let why = ask(&backend, "fail boom").expect_err("fails");
    assert!(why.contains("boom"), "{why}");
    assert_eq!(pool.live(), 0);
    rig.running(0);
    // A turn that ends in a refusal is a wrong answer: the session goes.
    assert!(ask(&backend, "Say again").is_ok());
    assert_eq!(rig.opened(), 2, "a new agent replaced the one that failed");
    let why = ask(&backend, "refuse nope").expect_err("wrong");
    assert!(why.contains("refusal"), "{why}");
    assert_eq!(pool.live(), 0);
    rig.running(0);
    // And an agent that died while it was idle is replaced without the request failing.
    assert!(ask(&backend, "Say once more").is_ok());
    assert_eq!(rig.opened(), 3);
    let Ok(Taken::Warm(conn)) = pool.take("scripted\u{1f}\u{1f}", Instant::now() + LONG) else {
        panic!("a warm session was left")
    };
    conn.link.close();
    pool.give("scripted\u{1f}\u{1f}", conn);
    rig.running(0);
    assert_eq!(ask(&backend, "Say recovered").as_deref(), Ok("recovered"));
    assert_eq!(rig.opened(), 4);
    pool.close();
    rig.running(0);
}

#[test]
fn closing_the_pool_ends_every_agent_and_no_later_request_starts_one() {
    let rig = Rig::new("close");
    let pool = pool(Duration::from_secs(60), 2);
    let backend = rig.backend(&pool, "one", table("scripted"));
    assert!(ask(&backend, "Say a").is_ok());
    backend.close();
    rig.running(0);
    let why = ask(&backend, "Say b").expect_err("closed");
    assert!(why.contains("stopping"), "{why}");
    assert_eq!(rig.opened(), 1);
}

#[test]
fn a_request_before_the_host_has_its_agents_fails_with_the_reason() {
    let pool = pool(Duration::from_secs(60), 1);
    let backend = HarnessBackend::new("early", &table("scripted"), Opener::default(), pool);
    let why = ask(&backend, "Say a").expect_err("not ready");
    assert!(why.contains("not started"), "{why}");
}

#[test]
fn a_harness_serves_a_completion_and_nothing_else() {
    let pool = pool(Duration::from_secs(60), 1);
    let backend = HarnessBackend::new("only", &table("scripted"), Opener::default(), pool);
    assert!(backend.serves(Operation::Complete));
    assert!(!backend.serves(Operation::Embed) && !backend.serves(Operation::Rerank));
    assert_eq!(backend.id(), "harness:only");
}

#[test]
fn the_instructions_of_a_request_go_first_in_the_user_turn() {
    let mut request = ChatRequest::ask("m", "the query");
    assert_eq!(prompt_of(&request), "the query");
    request.system = Some("Order these.".to_owned());
    assert_eq!(prompt_of(&request), "Order these.\n\nthe query");
}

#[test]
#[ignore = "needs Claude Code installed and signed in, and spends a few tokens of the subscription: set CANOA_TEST_CLAUDE to its program"]
fn a_real_claude_completes_a_prompt_through_a_warm_session() {
    // The harness is run as the person has it: the host opens `claude` the way `Local::open` does, which needs a whole host, so
    // this runs the harness's ACP adapter directly. It is left to the owner's machine.
    let program =
        std::env::var("CANOA_TEST_CLAUDE").expect("CANOA_TEST_CLAUDE names the ACP adapter");
    let rig = Rig::new("real");
    let host = Arc::new(Process {
        program: PathBuf::from(program),
        args: Vec::new(),
        opened: AtomicUsize::new(0),
        live: Arc::new(AtomicUsize::new(0)),
    });
    let opener = Opener::default();
    let _ = opener.set(host.clone() as Arc<dyn Host>);
    let pool = pool(Duration::from_secs(60), 1);
    let backend = HarnessBackend::new("real", &table("claude"), opener, pool);
    let said = ask(&backend, "Reply with the single word: ready").expect("answered");
    assert!(said.to_lowercase().contains("ready"), "{said}");
    assert_eq!(host.opened.load(Ordering::SeqCst), 1);
    drop(rig);
}
