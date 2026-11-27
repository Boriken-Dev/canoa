//! Roles over stand-in accounts: two Claude Code logins whose usage endpoint is a stand-in on loopback, so which member serves, and the
//! move of a session when its turn ends out of usage, run through the real providers' account code with nothing real spent. The harness
//! calls (export, import, send) are [`Hands`] that record what they were given.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use coqui_acp_api::Credentials;
use coqui_acp_history::{Document, LossReport};
use coqui_acp_pool::{AtTurn, HowEnded, Resume, Switchboard};
use serde_json::{Value, json};

use super::switch::{Hands, Idle, Imported, Mover, NOTICE_META, Switch, TurnEnd};
use super::{Gather, MEMORY, pool_of};
use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::{Config, RoleMember};
use crate::options::limits::Endpoints;
use crate::pools::gather::tests::{Dir, login_of, standin};

const NOW: i64 = 4_000_000_000;

const FILE: &str = "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"work\", \"work2\"]\n\
    [host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n\
    [host.accounts.work2]\nkind = \"claude-login\"\nid = \"id-work2\"\n\
    [host.roles.planner]\nresume_prompt = \"\"\nmembers = [\n\
      { agent = \"Claude\", account = \"work\" },\n\
      { agent = \"Claude\", account = \"work2\" },\n\
      { agent = \"Nowhere\" },\n\
    ]\n";

const OUT: fn() -> Value =
    || json!({"five_hour": {"utilization": 100.0, "resets_at": "2099-01-01T05:00:00Z"}});
const ROOM: fn() -> Value =
    || json!({"five_hour": {"utilization": 12.5, "resets_at": "2099-01-01T05:00:00Z"}});

/// What the host was asked to do to a session that moved.
#[derive(Default)]
struct Recorded {
    asked: RefCell<Vec<String>>,
    imported: RefCell<Option<Document>>,
    sent: RefCell<Vec<(String, Vec<Value>)>>,
}

/// Hands that export a fixed history and record the import and the prompt.
struct Fake {
    history: Document,
    recorded: Rc<Recorded>,
}

impl Hands for Fake {
    fn history(&self, from: &RoleMember, session: &str, _cwd: &str) -> Result<Document, String> {
        self.recorded
            .asked
            .borrow_mut()
            .push(format!("export {}:{session}", from.id()));
        Ok(self.history.clone())
    }

    fn import(&self, to: &RoleMember, history: &Document, _cwd: &str) -> Result<Imported, String> {
        self.recorded
            .asked
            .borrow_mut()
            .push(format!("import {}", to.id()));
        *self.recorded.imported.borrow_mut() = Some(history.clone());
        Ok(Imported {
            session: "moved-session".to_owned(),
            replayed: false,
            loss: LossReport::new(),
        })
    }

    fn send(
        &self,
        to: &RoleMember,
        session: &str,
        _cwd: &str,
        prompt: Vec<Value>,
    ) -> Result<String, String> {
        self.recorded
            .sent
            .borrow_mut()
            .push((format!("{}:{session}", to.id()), prompt));
        Ok("end_turn".to_owned())
    }
}

/// The history of a session whose last turn a limit ended, with a notice of the host's in it.
fn history(limit: &str) -> Document {
    let notice = format!("{{\"example\":true,\"{NOTICE_META}\":true}}");
    let lines = [
        r#"{"format":"coqui-acp-history","version":1,"id":"5f0f6d6e-3f3c-4f1a-8a52-1c1e6f0b9a77","origin":{"harness":"claude"}}"#.to_owned(),
        r#"{"method":"session/prompt","params":{"prompt":[{"text":"first question","type":"text"}]}}"#.to_owned(),
        r#"{"method":"session/update","params":{"update":{"content":{"text":"first answer","type":"text"},"sessionUpdate":"agent_message_chunk"}}}"#.to_owned(),
        r#"{"method":"history/turn_end","params":{"stopReason":"end_turn"}}"#.to_owned(),
        format!(r#"{{"method":"session/update","params":{{"update":{{"content":{{"text":"planner moved","type":"text"}},"sessionUpdate":"agent_message_chunk","_meta":{notice}}}}}}}"#),
        r#"{"method":"session/prompt","params":{"prompt":[{"text":"fix the build","type":"text"}]}}"#.to_owned(),
        r#"{"method":"session/update","params":{"update":{"content":{"text":"Looking at the build.","type":"text"},"sessionUpdate":"agent_message_chunk"}}}"#.to_owned(),
        r#"{"method":"session/update","params":{"update":{"kind":"read","sessionUpdate":"tool_call","status":"pending","title":"Read a","toolCallId":"t1"}}}"#.to_owned(),
        r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","status":"completed","toolCallId":"t1"}}}"#.to_owned(),
        format!(r#"{{"method":"session/update","params":{{"update":{{"content":{{"text":"{limit}","type":"text"}},"sessionUpdate":"agent_message_chunk"}}}}}}"#),
        r#"{"method":"history/turn_end","params":{"error":"limit","stopReason":"error"}}"#.to_owned(),
    ];
    format!("{}\n", lines.join("\n"))
        .parse()
        .expect("a history")
}

struct World {
    _dir: Dir,
    config: Config,
    store: Arc<Accounts>,
    endpoints: Endpoints,
}

/// The two accounts as the stand-in usage endpoint reports them.
fn world(work: Value, work2: Value) -> World {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    for (id, token) in [("id-work", "tok-work"), ("id-work2", "tok-work2")] {
        store
            .store_login(
                id,
                &Credentials::from_iter([("credentials_file", login_of(token))]),
                None,
            )
            .expect("stored");
    }
    let address = standin(vec![("tok-work", work), ("tok-work2", work2)]);
    World {
        _dir: dir,
        config: Config::from_str_in(FILE, std::path::Path::new("")).expect("parses"),
        store,
        endpoints: Endpoints::default().with("claude", address),
    }
}

#[test]
fn a_role_serves_its_first_member_with_usage_left_and_says_why_it_left_the_others() {
    crate::pools::gather::forget_reports();
    let world = world(OUT(), ROOM());
    let role = &world.config.roles["planner"];
    let pool = pool_of(&world.config, "planner", role);
    let gather = Gather::new(&world.config, &world.store, &world.endpoints, role);
    let starter = Mover::new(&Idle, "", &[]);
    let board = Switchboard::new(&pool, &*MEMORY, &gather, &starter).usage(&gather);
    let chosen = board
        .resolve(&coqui_acp_pool::Ask::new(NOW))
        .expect("a member has room");
    assert_eq!(chosen.member, "Claude/work2///", "{chosen:?}");
    assert_eq!(chosen.why, coqui_acp_pool::Why::Start);
    assert!(chosen.usage_known, "the account's usage was reported");
    let skipped: Vec<(&str, String)> = chosen
        .skipped
        .iter()
        .map(|each| (each.member.as_str(), each.reason.text()))
        .collect();
    assert_eq!(skipped[0].0, "Claude/work///");
    assert!(skipped[0].1.contains("is used up"), "{skipped:?}");
    // An agent the file names and the host does not have is a member that cannot serve, said by name, and no more.
    assert_eq!(
        skipped[1],
        ("Nowhere////", "no agent called Nowhere".to_owned())
    );
    // With room on both, the first serves: the strategy is the listed order.
    crate::pools::gather::forget_reports();
    let world = self::world(ROOM(), ROOM());
    let role = &world.config.roles["planner"];
    let gather = Gather::new(&world.config, &world.store, &world.endpoints, role);
    let board = Switchboard::new(&pool, &*MEMORY, &gather, &starter).usage(&gather);
    assert_eq!(
        board
            .resolve(&coqui_acp_pool::Ask::new(NOW))
            .expect("room")
            .member,
        "Claude/work///"
    );
}

#[test]
fn a_session_whose_turn_a_limit_ended_moves_to_the_next_member_cleaned_and_goes_on_by_itself() {
    crate::pools::gather::forget_reports();
    let world = world(OUT(), ROOM());
    let recorded = Rc::new(Recorded::default());
    let hands = Fake {
        history: history("You have hit your usage limit. Please wait for it to reset."),
        recorded: Rc::clone(&recorded),
    };
    let role = &world.config.roles["planner"];
    let switch = Switch {
        config: &world.config,
        accounts: &world.store,
        endpoints: &world.endpoints,
        hands: &hands,
        hold: None,
        landed: None,
    };
    let (answer, carried) = {
        switch.ended(
            ("planner", role),
            &TurnEnd {
                session: "s-1",
                cwd: "/work",
                member: "Claude/work///",
                how: HowEnded::Limit { until: None },
                now: NOW,
                model: None,
                report: None,
                record: coqui_acp_history::TurnRecord::new(NOW),
            },
        )
    };
    let AtTurn::Moved(moved) = answer else {
        panic!("the session should have moved: {answer:?}");
    };
    assert_eq!(moved.from, "Claude/work///");
    assert_eq!(moved.chosen.member, "Claude/work2///");
    assert!(
        moved.chosen.same_group,
        "another account of the same harness"
    );
    assert_eq!(carried.as_deref(), Some("moved-session"));
    assert_eq!(
        *recorded.asked.borrow(),
        ["export Claude/work///:s-1", "import Claude/work2///"]
    );
    // The carried session lost the failed turn's end, the limit's words and the host's own notice, and kept the work.
    let imported = recorded.imported.borrow().clone().expect("imported");
    let text = imported.to_string().expect("written");
    assert!(!text.contains("usage limit"), "{text}");
    assert!(!text.contains("planner moved"), "{text}");
    assert!(!text.contains("\"stopReason\":\"error\""), "{text}");
    assert!(text.contains("Looking at the build."), "{text}");
    assert!(text.contains("first answer"), "{text}");
    // The next agent is told one line, to the session the import made, and nothing else.
    let sent = recorded.sent.borrow();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, "Claude/work2///:moved-session");
    assert_eq!(
        sent[0].1,
        [json!({"type": "text", "text": coqui_acp_pool::DEFAULT_CONTINUATION})]
    );
    assert!(matches!(moved.resume, Resume::Continue(_)));
}

#[test]
fn a_turn_that_ended_for_another_reason_or_ended_well_with_usage_left_moves_nothing() {
    crate::pools::gather::forget_reports();
    let world = world(ROOM(), ROOM());
    let recorded = Rc::new(Recorded::default());
    let hands = Fake {
        history: history("anything"),
        recorded: Rc::clone(&recorded),
    };
    let role = &world.config.roles["planner"];
    let switch = Switch {
        config: &world.config,
        accounts: &world.store,
        endpoints: &world.endpoints,
        hands: &hands,
        hold: None,
        landed: None,
    };
    for how in [HowEnded::Other, HowEnded::Finished] {
        let (answer, carried) = switch.ended(
            ("planner", role),
            &TurnEnd {
                session: "s-1",
                cwd: "/work",
                member: "Claude/work///",
                how,
                now: NOW,
                model: None,
                report: None,
                record: coqui_acp_history::TurnRecord::new(NOW),
            },
        );
        assert_eq!(answer, AtTurn::Stayed, "{how:?}");
        assert!(carried.is_none());
    }
    assert!(recorded.asked.borrow().is_empty(), "nothing was exported");
}

mod hub {
    use std::sync::Mutex;

    use coqui_acp_history::Document;

    use super::*;
    use crate::delegate::{ChildContext, Principal, Source};
    use crate::history::Store;
    use crate::local::Link;
    use crate::roles::hub::{Backing, Binding, Hub};
    use crate::serve::LocalError;
    use crate::usage_splice::TurnEnded;

    /// Two accounts of one key each, which report no usage, so nothing is read from anywhere.
    const FILE: &str = "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"one\", \"two\"]\n\
        [host.accounts.one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
        [host.accounts.two]\nkind = \"anthropic-api-key\"\nid = \"id-two\"\n\
        [host.roles.planner]\nmembers = [\n\
          { agent = \"Claude\", account = \"one\" },\n\
          { agent = \"Claude\", account = \"two\" },\n\
        ]\n";

    struct Opener;

    impl crate::delegate::Host for Opener {
        fn sources(&self) -> Vec<Source> {
            Vec::new()
        }

        fn may_use(&self, _who: &Principal, _source: &str) -> bool {
            false
        }

        fn kind(&self, _source: &str) -> Option<String> {
            None
        }

        fn open(&self, _source: &str, _context: ChildContext) -> Result<Link, LocalError> {
            Err(LocalError::NoEntry)
        }

        fn prover(
            &self,
            _peer: &coqui_peer_webcrypto::PeerId,
        ) -> Result<cabuya::auth::Proof, String> {
            Err("no peer".to_owned())
        }
    }

    #[derive(Default)]
    struct Told {
        asked: Mutex<Vec<String>>,
        sent: Mutex<Vec<(String, Vec<Value>)>>,
    }

    struct Hands {
        history: Document,
        told: Arc<Told>,
    }

    impl super::Hands for Hands {
        fn history(
            &self,
            from: &RoleMember,
            session: &str,
            _cwd: &str,
        ) -> Result<Document, String> {
            self.told
                .asked
                .lock()
                .expect("told")
                .push(format!("export {}:{session}", from.id()));
            Ok(self.history.clone())
        }

        fn import(&self, to: &RoleMember, _h: &Document, _cwd: &str) -> Result<Imported, String> {
            self.told
                .asked
                .lock()
                .expect("told")
                .push(format!("import {}", to.id()));
            Ok(Imported {
                session: "moved-session".to_owned(),
                replayed: false,
                loss: LossReport::new(),
            })
        }

        fn send(
            &self,
            to: &RoleMember,
            session: &str,
            _cwd: &str,
            prompt: Vec<Value>,
        ) -> Result<String, String> {
            self.told
                .sent
                .lock()
                .expect("told")
                .push((format!("{}:{session}", to.id()), prompt));
            Ok("end_turn".to_owned())
        }
    }

    /// A role whose second member runs another harness, and which waits for the person before a move to it.
    const ASKING: &str = "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"one\"]\n\
        [host.accounts.one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
        [host.agents.Codex]\nharness = \"codex\"\n\
        [host.roles.planner]\nhandover = \"ask\"\nmembers = [\n\
          { agent = \"Claude\", account = \"one\" },\n\
          { agent = \"Codex\" },\n\
        ]\n";

    /// The same, with both members on one harness: it never waits.
    const ASKING_WITHIN: &str = "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"one\", \"two\"]\n\
        [host.accounts.one]\nkind = \"anthropic-api-key\"\nid = \"id-one\"\n\
        [host.accounts.two]\nkind = \"anthropic-api-key\"\nid = \"id-two\"\n\
        [host.roles.planner]\nhandover = \"ask\"\nmembers = [\n\
          { agent = \"Claude\", account = \"one\" },\n\
          { agent = \"Claude\", account = \"two\" },\n\
        ]\n";

    struct Backed {
        store: Arc<Accounts>,
        history: Arc<Store>,
        told: Arc<Told>,
        file: &'static str,
    }

    impl Backing for Backed {
        fn config(&self) -> Result<Config, String> {
            Config::from_str_in(self.file, std::path::Path::new(""))
                .map_err(|error| error.to_string())
        }

        fn accounts(&self, _config: &Config) -> Result<Arc<Accounts>, String> {
            Ok(Arc::clone(&self.store))
        }

        fn hands(
            &self,
            _opener: Arc<dyn crate::delegate::Host>,
        ) -> Arc<dyn super::Hands + Send + Sync> {
            Arc::new(Hands {
                history: history("You have hit your usage limit."),
                told: Arc::clone(&self.told),
            })
        }

        fn store(&self) -> Option<Arc<Store>> {
            Some(Arc::clone(&self.history))
        }
    }

    fn ready() -> (Arc<Hub>, Arc<Told>, Arc<Store>, Dir) {
        ready_with(FILE)
    }

    fn ready_with(file: &'static str) -> (Arc<Hub>, Arc<Told>, Arc<Store>, Dir) {
        let dir = Dir::new();
        let root = dir.0.join("accounts");
        let store =
            Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
        let history = Arc::new(Store::open(
            Some(dir.0.join("history")),
            crate::config::HistoryConfig::default(),
        ));
        let told = Arc::new(Told::default());
        let hub = Hub::new();
        hub.attach(Arc::new(Backed {
            store,
            history: Arc::clone(&history),
            told: Arc::clone(&told),
            file,
        }));
        hub.opener(Arc::new(Opener));
        // The session as the host recorded it: two turns, the second one a limit ended.
        history
            .seed(
                "Claude",
                "s-1",
                &self::history("You have hit your usage limit."),
            )
            .expect("seeded");
        hub.bind(
            "Claude",
            "s-1",
            Binding {
                role: "planner".to_owned(),
                member: "Claude/one///".to_owned(),
                cwd: "/work".to_owned(),
                moved_from: None,
            },
        );
        (hub, told, history, dir)
    }

    fn ended(failed: bool, said: &str) -> TurnEnded {
        TurnEnded {
            session: "s-1".to_owned(),
            failed,
            stop: Some(if failed { "error" } else { "end_turn" }.to_owned()),
            said: said.to_owned(),
            input: Some(120),
            output: Some(45),
            ..TurnEnded::default()
        }
    }

    #[test]
    fn a_bound_session_whose_turn_a_limit_ended_moves_by_itself_and_keeps_both_turns_properties() {
        let (hub, told, history, _dir) = ready();
        let answer = hub
            .run("Claude", &ended(true, "You have hit your usage limit."))
            .expect("a bound session");
        assert!(matches!(answer, AtTurn::Moved(_)), "{answer:?}");
        assert_eq!(
            *told.asked.lock().expect("told"),
            ["export Claude/one///:s-1", "import Claude/two///"]
        );
        let sent = told.sent.lock().expect("told");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "Claude/two///:moved-session");
        // The session is the other member's now, under the id the import made, and says where it came from.
        assert!(hub.binding("Claude", "s-1").is_none());
        let now = hub.binding("Claude", "moved-session").expect("rebound");
        assert_eq!(now.member, "Claude/two///");
        assert_eq!(now.moved_from.as_deref(), Some("Claude/one///"));
        let moves = hub.moves_of("Claude", "s-1");
        assert_eq!(moves.len(), 1);
        assert_eq!(
            moves[0].to_session,
            ("Claude".to_owned(), "moved-session".to_owned())
        );
        // The turn the limit ended is recorded under the member it ran on, with what it used and that it moved on.
        let turns = history.served("Claude", "s-1").expect("read");
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert_eq!(turns[0]["no"], 2);
        assert_eq!(turns[0]["role"], "planner");
        assert_eq!(turns[0]["member"], "Claude/one///");
        assert_eq!(turns[0]["limited"], true);
        assert_eq!(turns[0]["moved_on"], true);
    }

    #[test]
    fn a_turn_that_ended_well_moves_nothing_and_is_recorded_under_the_member_that_ran_it() {
        let (hub, told, history, _dir) = ready();
        let answer = hub.run("Claude", &ended(false, "all done"));
        assert!(answer.is_none(), "{answer:?}");
        assert!(told.asked.lock().expect("told").is_empty());
        assert!(hub.binding("Claude", "s-1").is_some());
        let turns = history.served("Claude", "s-1").expect("read");
        assert_eq!(turns[0]["member"], "Claude/one///");
        assert_eq!(turns[0]["why"], "kept");
        assert_eq!(turns[0]["limited"], false);
    }

    #[test]
    fn a_session_no_role_serves_is_left_alone() {
        let (hub, told, _history, _dir) = ready();
        assert!(hub.unbind("Claude", "s-1"));
        assert!(
            hub.run("Claude", &ended(true, "You have hit your usage limit."))
                .is_none()
        );
        assert!(told.asked.lock().expect("told").is_empty());
    }

    fn reported(hit: &coqui_acp_api::LimitHit) -> Value {
        json!({ coqui_acp_api::LIMIT_META_KEY: serde_json::to_value(hit).expect("a hit is JSON") })
    }

    #[test]
    fn a_limit_the_agent_says_no_other_member_helps_with_leaves_the_session_on_its_member() {
        let (hub, told, history, _dir) = ready();
        // The agent's own words name no allowance, so only what it reported says this is a limit, and that it follows the work.
        let mut turn = ended(false, "The session's budget is used up.");
        turn.refused = true;
        turn.stop = Some("refusal".to_owned());
        turn.meta = Some(reported(
            &coqui_acp_api::LimitHit::new(1).continuable(false),
        ));
        let answer = hub.run("Claude", &turn).expect("a bound session");
        assert!(matches!(answer, AtTurn::NotContinuable(_)), "{answer:?}");
        assert!(told.asked.lock().expect("told").is_empty());
        assert!(hub.binding("Claude", "s-1").is_some());
        let turns = history.served("Claude", "s-1").expect("read");
        assert_eq!(turns[0]["limited"], true, "{turns:?}");
        assert_eq!(turns[0]["moved_on"], false, "{turns:?}");
        // The same turn of a limit that follows the member moves, though its words name nothing either.
        let mut again = turn.clone();
        again.meta = Some(reported(&coqui_acp_api::LimitHit::new(1)));
        let answer = hub.run("Claude", &again).expect("a bound session");
        assert!(matches!(answer, AtTurn::Moved(_)), "{answer:?}");
    }

    /// Runs the turn on a thread, since a move that waits for the person does not return.
    fn running(hub: &Arc<Hub>) -> std::thread::JoinHandle<Option<AtTurn>> {
        let hub = Arc::clone(hub);
        std::thread::spawn(move || {
            hub.run("Claude", &ended(true, "You have hit your usage limit."))
        })
    }

    fn told_of(watching: &std::sync::mpsc::Receiver<Value>, method: &str) -> Option<Value> {
        told_within(watching, method, 30)
    }

    fn told_within(
        watching: &std::sync::mpsc::Receiver<Value>,
        method: &str,
        seconds: u64,
    ) -> Option<Value> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        while std::time::Instant::now() < until {
            if let Ok(note) = watching.recv_timeout(std::time::Duration::from_millis(100))
                && note["method"] == method
            {
                return Some(note);
            }
        }
        None
    }

    #[test]
    fn a_move_to_another_harness_waits_for_the_persons_yes_and_tells_the_link_when_it_is_made() {
        let (hub, told, _history, _dir) = ready_with(ASKING);
        let watching = hub.watch();
        let turn = running(&hub);
        let waiting = told_of(&watching, "roles/ask").expect("the link is told a move waits");
        assert_eq!(waiting["params"]["role"], "planner");
        assert_eq!(waiting["params"]["session"], "s-1");
        assert_eq!(
            waiting["params"]["from"],
            json!({"entry": "Claude", "session": "s-1"})
        );
        assert_eq!(waiting["params"]["to"], json!({"entry": "Codex"}));
        assert!(
            waiting["params"]["reason"]
                .as_str()
                .is_some_and(|why| why.contains("out of usage"))
        );
        // Nothing was exported, imported or sent while it waits, and no one is told it moved.
        assert!(!turn.is_finished());
        assert!(told.asked.lock().expect("told").is_empty());
        assert!(!hub.go(None, "another-session"));
        assert!(hub.go(None, "s-1"));
        let answer = turn.join().expect("the move ends").expect("a session");
        assert!(matches!(answer, AtTurn::Moved(_)), "{answer:?}");
        assert_eq!(
            *told.asked.lock().expect("told"),
            ["export Claude/one///:s-1", "import Codex////"]
        );
        let moved = told_of(&watching, "roles/moved").expect("the link is told it moved");
        assert_eq!(
            moved["params"],
            json!({"role": "planner",
                "from": {"entry": "Claude", "session": "s-1"},
                "to": {"entry": "Codex", "session": "moved-session"},
                "member": {"entry": "Codex"},
                "replayed": false, "losses": []})
        );
    }

    #[test]
    fn a_move_that_waits_ends_when_the_session_stops_being_the_roles_and_moves_nothing() {
        let (hub, told, _history, _dir) = ready_with(ASKING);
        let watching = hub.watch();
        let turn = running(&hub);
        told_of(&watching, "roles/ask").expect("a move waits");
        assert!(hub.unbind("Claude", "s-1"));
        let answer = turn.join().expect("the move ends").expect("a session");
        assert!(!matches!(answer, AtTurn::Moved(_)), "{answer:?}");
        assert!(told.asked.lock().expect("told").is_empty());
        assert!(told_within(&watching, "roles/moved", 1).is_none());
    }

    #[test]
    fn a_move_within_the_harness_never_waits_even_when_the_role_asks() {
        let (hub, told, _history, _dir) = ready_with(ASKING_WITHIN);
        let watching = hub.watch();
        let answer = running(&hub).join().expect("ends").expect("a session");
        assert!(matches!(answer, AtTurn::Moved(_)), "{answer:?}");
        assert_eq!(told.asked.lock().expect("told").len(), 2);
        assert!(told_of(&watching, "roles/moved").is_some());
    }
}
