//! A question a program install puts to the person, over the control channel: the form comes as the sign-in's does, sealed to the
//! key that asked, so a typed secret is in no line the link carried, for the application's own link and for a paired browser
//! key. The install still gets the value. The store is a made-up program under a made-up publisher.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use canoa::store::{
    ApplyOutcome, ApplyRequest, InstallPrompter, InstallTools, ProgramQuery, ProgramReport,
    ProgramStore, StoreInfo, StoreSettings,
};
use canoa_host::store::contract::{Fixture, Variant, WorldTransport};
use canoa_host::store::echo::{EchoFixture, EchoRecipe};
use canoa_host::store::engine::Engine;
use canoa_host::store::programs::{Options, Programs};
use canoa_host::store::shelf::Shelf;
use canoa_host::{Config, Home, Link, ServeOptions, Serving};
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Alloc, Value as Gv};
use coqui_acp_api::{LoginForm, ProcessSpawner};
use coqui_agentic_peer::auth::Identity;
use sealed::{App, Typed, found_in};
use serde_json::{Value, json};
use support::{Scratch, WAIT, example, host_binary, quoted};

/// What each install was given when it asked, as `user=… token=…`.
static GIVEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The echo store, but its install asks the person for a login first and refuses to go on without it.
#[derive(coqui_acp_api::guatiao::Provider, Debug)]
#[provider(
    canoa::store::ProgramStore,
    id = "asking_store",
    config = canoa::store::StoreSettings,
    new = AskingStore::unconfigured,
    crate = ::coqui_acp_api::guatiao
)]
pub struct AskingStore {
    engine: Engine<EchoRecipe>,
}

impl AskingStore {
    fn unconfigured() -> Self {
        Self::from(StoreSettings::default())
    }
}

impl From<StoreSettings> for AskingStore {
    fn from(settings: StoreSettings) -> Self {
        Self {
            engine: Engine::new(EchoRecipe, settings),
        }
    }
}

impl ProgramStore for AskingStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        self.engine.describe()
    }

    fn program(
        &self,
        query: ProgramQuery,
        spawner: Object<dyn ProcessSpawner>,
        tools: Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError> {
        self.engine.program(&query, &spawner, &tools)
    }

    fn latest(
        &self,
        query: canoa::store::LatestQuery,
        tools: Object<dyn InstallTools>,
    ) -> Result<canoa::store::LatestReport, ProviderError> {
        self.engine.latest(&query, &tools)
    }

    fn plan(
        &self,
        request: canoa::store::PlanRequest,
        tools: Object<dyn InstallTools>,
    ) -> Result<canoa::store::InstallPlan, ProviderError> {
        self.engine.plan(&request, &tools)
    }

    fn apply(
        &self,
        request: ApplyRequest,
        spawner: Object<dyn ProcessSpawner>,
        tools: Object<dyn InstallTools>,
        prompter: Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError> {
        let schema = json!({
            "type": "object",
            "required": ["token"],
            "properties": {
                "user": {"type": "string", "title": "User name"},
                "token": {"type": "string", "title": "Token", "writeOnly": true, "x-sensitive": true},
            },
        });
        let schema: Gv = guatiao_serde::adapt::to_value_in(&schema, Alloc::rust()).expect("schema");
        let answer = prompter.ask(LoginForm::new("Registry login", schema))?;
        let user = guatiao_serde::Serializable::from(&answer.values);
        let user = serde_json::to_value(user).unwrap_or(Value::Null);
        let token = answer
            .secrets
            .expose()
            .get("token")
            .unwrap_or("")
            .to_owned();
        GIVEN.lock().expect("not poisoned").push(format!(
            "user={} token={token}",
            user["user"].as_str().unwrap_or("")
        ));
        self.engine.apply(&request, &spawner, &tools, &prompter)
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "asking-stores",
    version = "0.0.0",
    providers = [AskingStore]
);

fn registry() -> Arc<Registry> {
    let mut registry = Registry::new("test", "0.0.0");
    registry
        .register_local("test", library)
        .expect("registered");
    Arc::new(registry)
}

fn programs(root: PathBuf, home: PathBuf) -> Programs {
    let fixture = EchoFixture {
        standin: example("version_standin", false),
        archive: false,
    };
    let transport = WorldTransport::new(fixture.world(Variant::Normal));
    let mut options = Options::default();
    options.home = Some(home);
    options.watch_registry = false;
    options.settle = Duration::from_millis(100);
    options.allow_elevated = true;
    options.environment = {
        let mut env = coqui_acp_api::Environment::new();
        for name in ["PATH", "Path", "SystemRoot"] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name, value);
            }
        }
        env
    };
    Programs::new(Shelf::unchecked(root), registry())
        .transport(transport)
        .options(options)
}

/// A host serving the asking store, with `key` paired and allowed to read and install on `@control`.
struct Hosted {
    _lock: Box<dyn std::any::Any>,
    _scratch: Scratch,
    serving: Option<std::thread::JoinHandle<()>>,
    stopper: canoa_host::Stopper,
    local: canoa_host::Local,
    url: String,
    peer: coqui_peer_webcrypto::PeerId,
    host_spki: Vec<u8>,
}

fn host(key: &Identity) -> Hosted {
    let scratch = Scratch::new();
    let home = Home::at(scratch.0.join("home"));
    let lock = home.lock("a test").expect("a free home");
    std::fs::create_dir_all(scratch.0.join("user")).expect("a home for the installs");
    let config = Config::from_str_in(
        &format!(
            "[host.agents.fixture]\nprogram = {}\n\n[host.control]\nlisten = \"127.0.0.1:0\"\ngrants = {{ \"{}\" = [\"store.read\", \"store.programs\"] }}\n",
            quoted(&example("stdio_agent", false).to_string_lossy()),
            key.peer_id()
        ),
        &scratch.0,
    )
    .expect("parses");
    let identity = home.identity(Some(support::method())).expect("an identity");
    let host_spki = identity.key_material().bytes.clone();
    let store = home.store().expect("a store");
    store
        .accept_key(&key.key_material())
        .expect("the key is paired");
    let serving = Serving::bind(
        &config,
        identity,
        store,
        &ServeOptions::new(host_binary()).with_programs(
            Arc::new(programs(scratch.0.join("programs"), scratch.0.join("user"))),
            vec!["asking_store".to_owned()],
        ),
    )
    .expect("it binds");
    let url = serving
        .listening()
        .iter()
        .find(|entry| entry.name == "@control")
        .map(|entry| entry.url.clone())
        .expect("@control listens");
    let (local, stopper, peer) = (serving.local(), serving.stopper(), serving.peer());
    let run = std::thread::spawn(move || serving.run());
    Hosted {
        _lock: Box::new(lock),
        _scratch: scratch,
        serving: Some(run),
        stopper,
        local,
        url,
        peer,
        host_spki,
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        self.stopper.stop();
        if let Some(run) = self.serving.take() {
            let _ = run.join();
        }
    }
}

const FORM_USER: &str = "ada";

/// What the person types: a token that is searched for in everything the link carried.
fn typed(token: &'static str) -> Typed {
    // The fields are static text; each test names its own token.
    Typed::Fills(Box::leak(
        vec![("user", FORM_USER), ("token", token)].into_boxed_slice(),
    ))
}

/// Plays one `store/apply` on a connection: the host's questions are answered by `app` as `typed` says. Gives what the apply ended
/// with, and every line either way is kept in `app.traffic`.
fn play(
    app: &mut App,
    typed: &Typed,
    send: &mut dyn FnMut(&Value),
    next: &mut dyn FnMut() -> Result<Value, String>,
    plan: &Value,
) -> Result<Value, Value> {
    let line = json!({"jsonrpc": "2.0", "id": 90, "method": "store/apply", "params": {"provider": "asking_store", "plan": plan}});
    app.traffic.push(line.to_string());
    send(&line);
    loop {
        let message = match next() {
            Ok(message) => message,
            Err(why) => return Err(json!(why)),
        };
        app.traffic.push(message.to_string());
        if message["method"] == "_coqui_acp/secret_request" {
            let Some(result) = app.answer(&message["params"], typed) else {
                return Err(json!("the person left"));
            };
            let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": result});
            app.traffic.push(reply.to_string());
            send(&reply);
        } else if message["method"] == "store/ask" {
            // A host that asks in the clear is answered in the clear, so the secret shows in the lines the link carried.
            let reply = json!({"jsonrpc": "2.0", "id": message["id"], "result": {
                "values": {"user": FORM_USER}, "secrets": {"token": token_of(typed)}}});
            app.traffic.push(reply.to_string());
            send(&reply);
        } else if message["id"] == 90 && message.get("method").is_none() {
            return match message.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(message["result"].clone()),
            };
        }
    }
}

fn token_of(typed: &Typed) -> &'static str {
    match typed {
        Typed::Fills(values) | Typed::Forged(values) => values
            .iter()
            .find(|(name, _)| *name == "token")
            .map_or("", |(_, value)| value),
        _ => "",
    }
}

fn given_with(token: &str) -> Vec<String> {
    GIVEN
        .lock()
        .expect("not poisoned")
        .iter()
        .filter(|line| line.contains(token))
        .cloned()
        .collect()
}

fn local_call(link: &Link, id: u64, method: &str, params: Value) -> Value {
    linked::ask(link, id, method, params).expect("answered").0
}

const CANARY_OWN: &str = "tok-OWN-CANARY/+=q9\"\\z-0123456789";
const CANARY_KEY: &str = "tok-KEY-CANARY/+=q9\"\\z-9876543210";

#[test]
fn a_form_an_install_asks_the_applications_own_link_is_sealed_and_its_secret_is_in_no_line() {
    let key = support::key();
    let host = host(&key);
    let link = host.local.open("@control").expect("a link");
    let secret = host.local.link_secret().expect("the host minted a secret");
    let mut app = App::new(&secret[..]);
    let plan = local_call(
        &link,
        1,
        "store/plan",
        json!({"provider": "asking_store", "action": "install", "version": "1.0.0"}),
    );
    let mut send = |message: &Value| linked::send(&link, message);
    let mut next = || linked::next(&link);
    let done = play(&mut app, &typed(CANARY_OWN), &mut send, &mut next, &plan)
        .expect("the install goes on");
    assert_eq!(done["applied"], true, "{done}");

    assert!(
        app.traffic
            .iter()
            .any(|line| line.contains("_coqui_acp/secret_request")),
        "the form was put to the app sealed"
    );
    assert!(
        !app.traffic.iter().any(|line| line.contains("store/ask")),
        "nothing was asked in the clear"
    );
    let places: Vec<(String, Vec<u8>)> = app
        .traffic
        .iter()
        .enumerate()
        .map(|(index, line)| (format!("link line {index}"), line.as_bytes().to_vec()))
        .collect();
    let hits = found_in(CANARY_OWN, &places);
    assert!(hits.is_empty(), "{hits:?}");
    // The plain field stays plain, and the install was given both.
    assert!(
        app.traffic.iter().any(|line| line.contains(FORM_USER)),
        "the plain field travels as plain"
    );
    assert_eq!(
        given_with(CANARY_OWN),
        [format!("user={FORM_USER} token={CANARY_OWN}")]
    );
}

#[test]
fn a_form_an_install_asks_a_paired_browser_key_is_sealed_to_that_key_and_its_secret_is_in_no_line()
{
    let key = support::key();
    let host = host(&key);
    let mut app = App::for_key(&key, &host.host_spki);
    support::with_link(&host.url, &key, host.peer, |rpc| {
        let plan = rpc
            .request(
                "store/plan",
                json!({"provider": "asking_store", "action": "install", "version": "1.0.0"}),
            )
            .expect("a plan");
        let done = play(
            &mut app,
            &typed(CANARY_KEY),
            &mut |message| rpc.send(message).expect("sent"),
            &mut || rpc.next_message(WAIT),
            &plan,
        )
        .expect("the install goes on");
        assert_eq!(done["applied"], true, "{done}");
    })
    .expect("a link");

    assert!(
        app.traffic
            .iter()
            .any(|line| line.contains("_coqui_acp/secret_request")),
        "the form was put to the key sealed"
    );
    assert!(
        !app.traffic.iter().any(|line| line.contains("store/ask")),
        "nothing was asked in the clear"
    );
    let places: Vec<(String, Vec<u8>)> = app
        .traffic
        .iter()
        .enumerate()
        .map(|(index, line)| (format!("link line {index}"), line.as_bytes().to_vec()))
        .collect();
    let hits = found_in(CANARY_KEY, &places);
    assert!(hits.is_empty(), "{hits:?}");
    assert_eq!(
        given_with(CANARY_KEY),
        [format!("user={FORM_USER} token={CANARY_KEY}")]
    );
}

#[test]
fn a_declined_form_and_an_answer_bound_to_another_secret_give_the_install_nothing() {
    const DECLINED: &str = "tok-DECLINED-CANARY";
    const FORGED: &str = "tok-FORGED-CANARY";
    let key = support::key();
    let host = host(&key);
    let link = host.local.open("@control").expect("a link");
    let secret = host.local.link_secret().expect("the host minted a secret");
    for (id, token, typed) in [
        (10, DECLINED, Typed::Declines),
        (
            20,
            FORGED,
            Typed::Forged(Box::leak(
                vec![("user", FORM_USER), ("token", FORGED)].into_boxed_slice(),
            )),
        ),
    ] {
        let mut app = App::new(&secret[..]);
        let plan = local_call(
            &link,
            id,
            "store/plan",
            json!({"provider": "asking_store", "action": "install", "version": "1.0.0"}),
        );
        let mut send = |message: &Value| linked::send(&link, message);
        let mut next = || linked::next(&link);
        let refused = play(&mut app, &typed, &mut send, &mut next, &plan)
            .expect_err("an install that was not answered does not go on");
        assert!(refused["message"].is_string(), "{refused}");
        assert!(given_with(token).is_empty(), "{token} reached the install");
    }
}
