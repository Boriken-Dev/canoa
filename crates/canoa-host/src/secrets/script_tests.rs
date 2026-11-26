//! The Bitwarden provider, a library written in JavaScript that ships in the host, run by the QuickJS built into it, against a stand-in
//! for `bw`. It proves the `secret_provider` kind is one a script can serve: its description and settings, the values it answers, the
//! flags that say what it needs, the errors it raises, and that every program it starts goes through the spawner it is handed.
//!
//! The stand-in is Rust here (a spawner that speaks the parts of `bw` the provider uses, as `bw` does); `end_to_end` also runs the real
//! process spawner against `tests/bw_standin.py` when `CANOA_TEST_PYTHON` names an interpreter.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::{Arc, Mutex};

use canoa::secret_provider::{Keeping, Named, SecretProvider};
use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    FileInfo, HarnessNotice, LinkState, ProcessExit, ProcessSpawner, READ_TIMED_OUT, SecretBytes,
    SpawnSpec, SpawnedProcess, SpawnerInfo, TerminalSize,
};
use serde_json::{Map, Value, json};

use super::catalog::Catalog;

pub(super) const GOOD_KEY: &str = "good-session-key-made-up";
pub(super) const VALUE: &str = "gh-pw-zq9-made-up";

/// A folder of its own for the bundled files, removed when dropped.
pub(super) struct Home(pub(super) std::path::PathBuf);

impl Home {
    pub(super) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-bundled-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the stand-in was asked: the words after the program, and whether the session key was in the environment.
#[derive(Clone, Debug)]
pub(super) struct Call {
    pub(super) program: String,
    pub(super) words: Vec<String>,
    pub(super) session_in_env: bool,
    pub(super) key_on_command_line: bool,
}

/// A process that has finished: what it printed and how it ended.
struct Canned {
    out: Mutex<Vec<u8>>,
    err: Mutex<Vec<u8>>,
    code: i64,
    hangs: bool,
}

fn take(from: &Mutex<Vec<u8>>, into: &mut [u8]) -> i64 {
    let mut have = from.lock().unwrap();
    let n = have.len().min(into.len());
    into[..n].copy_from_slice(&have[..n]);
    have.drain(..n);
    i64::try_from(n).unwrap()
}

impl SpawnedProcess for Canned {
    fn write(&self, _: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn close_input(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    fn read(&self, dst: &mut [u8], _: u64) -> Result<i64, ProviderError> {
        if self.hangs {
            return Ok(READ_TIMED_OUT);
        }
        Ok(take(&self.out, dst))
    }
    fn read_error(&self, dst: &mut [u8], _: u64) -> Result<i64, ProviderError> {
        Ok(take(&self.err, dst))
    }
    fn resize(&self, _: TerminalSize) -> Result<(), ProviderError> {
        Err(canoa::status::unsupported("no terminal"))
    }
    fn wait(&self, _: u64) -> Result<ProcessExit, ProviderError> {
        Ok(if self.hangs {
            ProcessExit::running()
        } else {
            ProcessExit::ended(Some(self.code), None)
        })
    }
    fn kill(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    fn link(&self) -> Result<LinkState, ProviderError> {
        Ok(LinkState::up())
    }
}

/// `bw` as the tests' spawner runs it, in the words `bw` uses, with made-up items.
pub(super) struct StandIn {
    pub(super) calls: Arc<Mutex<Vec<Call>>>,
}

impl StandIn {
    pub(super) fn new() -> (Self, Arc<Mutex<Vec<Call>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }

    fn item(name: &str, part: &str) -> Option<&'static str> {
        Some(match (name, part) {
            ("github", "password") => "gh-pw-zq9-made-up\n",
            ("github", "username") => "ana",
            ("github", "notes") => "a note",
            ("github", "totp") => "123456",
            ("github", "uri") => "https://github.com",
            ("empty", "password") => "",
            ("two lines", "password") => "line one\nline two",
            _ => return None,
        })
    }
}

fn failed(why: &str) -> Canned {
    Canned {
        out: Mutex::new(Vec::new()),
        err: Mutex::new(why.as_bytes().to_vec()),
        code: 1,
        hangs: false,
    }
}

impl ProcessSpawner for StandIn {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        Ok(SpawnerInfo::new("test", "test", true))
    }

    fn spawn(&self, spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        let environment = spec.environment.expose();
        let key = environment.get("BW_SESSION").map(str::to_owned);
        let words: Vec<String> = spec
            .args
            .iter()
            .filter(|word| *word != "--nointeraction")
            .cloned()
            .collect();
        self.calls.lock().unwrap().push(Call {
            program: spec.program.clone(),
            words: spec.args.clone(),
            session_in_env: key.is_some(),
            key_on_command_line: spec.args.iter().any(|word| word.contains(GOOD_KEY)),
        });
        if spec.program == "no-such-program-bw" {
            return Err(ProviderError::new(
                coqui_acp_api::guatiao::Status::GUATIAO_ERR_NOT_FOUND,
                "no such program",
            ));
        }
        if spec.program == "slow-bw" {
            return Ok(Canned {
                out: Mutex::new(Vec::new()),
                err: Mutex::new(Vec::new()),
                code: 0,
                hangs: true,
            }
            .into_object());
        }
        let asked_nothing = spec.args.iter().any(|word| word == "--nointeraction")
            && environment.get("BW_NOINTERACTION") == Some("true");
        let canned = if !asked_nothing {
            failed("would have asked a question\n")
        } else if key.as_deref() != Some(GOOD_KEY) {
            failed("Vault is locked.\n")
        } else if words.len() >= 2 && words[0] == "list" && words[1] == "items" {
            let items: Vec<Value> = ["github", "empty", "two lines", "github"]
                .iter()
                .map(|name| json!({"name": name, "login": {"password": "x"}}))
                .collect();
            Canned {
                out: Mutex::new(Value::Array(items).to_string().into_bytes()),
                err: Mutex::new(Vec::new()),
                code: 0,
                hangs: false,
            }
        } else if words.len() == 3 && words[0] == "get" {
            match Self::item(&words[2], &words[1]) {
                Some(text) => {
                    let mut printed = text.as_bytes().to_vec();
                    if !text.ends_with('\n') {
                        printed.push(b'\n');
                    }
                    Canned {
                        out: Mutex::new(printed),
                        err: Mutex::new(Vec::new()),
                        code: 0,
                        hangs: false,
                    }
                }
                None => failed("Not found.\n"),
            }
        } else {
            failed("unknown command\n")
        };
        Ok(canned.into_object())
    }

    fn write_file(&self, _: &str, _: Secret<SecretBytes>, _: bool) -> Result<(), ProviderError> {
        Err(canoa::status::unsupported("no files"))
    }
    fn read_file(&self, _: &str, _: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        Err(canoa::status::unsupported("no files"))
    }
    fn file_info(&self, _: &str) -> Result<Option<FileInfo>, ProviderError> {
        Err(canoa::status::unsupported("no files"))
    }
    fn remove_file(&self, _: &str) -> Result<(), ProviderError> {
        Err(canoa::status::unsupported("no files"))
    }
    fn notice(&self, _: HarnessNotice) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// The Bitwarden provider of the bundled catalog, configured with `settings`.
fn provider(home: &Home, settings: &Value) -> Arc<dyn SecretProvider> {
    let catalog = Catalog::built_in_in(&home.0);
    let config: Map<String, Value> = settings.as_object().cloned().unwrap_or_default();
    catalog
        .instance("bitwarden", &config)
        .unwrap_or_else(|why| panic!("the provider is made: {why}"))
}

fn standin() -> (Object<dyn ProcessSpawner>, Arc<Mutex<Vec<Call>>>) {
    let (standin, calls) = StandIn::new();
    (standin.into_object(), calls)
}

fn good() -> Value {
    json!({"command": "bw", "session": GOOD_KEY})
}

fn value_of(provider: &Arc<dyn SecretProvider>, name: &str) -> Option<String> {
    let (spawner, _) = standin();
    provider
        .get(Named::new(name), spawner)
        .expect("an answer")
        .value
        .map(|value| value.expose().clone())
}

fn provider_settings(part: &str) -> Value {
    json!({"command": "bw", "session": GOOD_KEY, "part": part})
}

#[test]
fn the_bundled_library_is_offered_as_a_built_in_provider_with_the_settings_it_declares() {
    let home = Home::new("offer");
    let catalog = Catalog::built_in_in(&home.0);
    let listed = catalog.list();
    let ids: Vec<&str> = listed.iter().map(|each| each.id.as_str()).collect();
    assert_eq!(ids, ["file", "form", "system", "bitwarden"], "{ids:?}");
    let bitwarden = &listed[3];
    assert_eq!(bitwarden.origin, "built-in");
    assert_eq!(bitwarden.info.title, "Bitwarden");
    assert!(bitwarden.info.can_list && !bitwarden.info.can_store && !bitwarden.info.needs_person);
    let schema = bitwarden.schema.as_ref().expect("a schema");
    let session = &schema["properties"]["session"];
    assert!(
        session["writeOnly"] == true && session["x-sensitive"] == true,
        "the session key is a secret, so a form hides it: {session}"
    );
    assert_eq!(
        session["x-from-secret"], true,
        "and it may be taken from another provider"
    );
    assert!(
        schema
            .get("required")
            .is_none_or(|required| required == &json!([])),
        "every setting has a default, so a description needs none"
    );
    let form = bitwarden.form.as_ref().expect("a form");
    assert_eq!(form["sections"][0]["id"], "vault");
    // A setting the script does not declare is refused by the host before the script is asked.
    let mut surprise = Map::new();
    surprise.insert("surprise".to_owned(), json!(1));
    assert!(catalog.instance("bitwarden", &surprise).is_err());
}

#[test]
fn the_files_that_ship_are_rewritten_when_they_differ() {
    let home = Home::new("rewrite");
    let folder = super::bundled::keep(&home.0).expect("kept");
    let file = folder.join("bitwarden/bitwarden.js");
    let shipped = std::fs::read(&file).expect("shipped");
    std::fs::write(&file, "export const planted = 1;\n").expect("changed");
    super::bundled::keep(&home.0).expect("kept again");
    assert_eq!(std::fs::read(&file).expect("shipped again"), shipped);
}

#[test]
fn a_part_of_an_item_is_the_value_and_the_line_break_bw_adds_is_not() {
    let home = Home::new("parts");
    let password = provider(&home, &provider_settings("password"));
    assert_eq!(value_of(&password, "github").as_deref(), Some(VALUE));
    assert_eq!(
        value_of(&password, "two lines").as_deref(),
        Some("line one\nline two")
    );
    let username = provider(&home, &provider_settings("username"));
    assert_eq!(value_of(&username, "github").as_deref(), Some("ana"));
}

#[test]
fn an_item_that_is_not_there_or_has_no_such_part_is_no_value_and_no_failure() {
    let home = Home::new("missing");
    let password = provider(&home, &provider_settings("password"));
    assert_eq!(value_of(&password, "ghost"), None);
    assert_eq!(value_of(&password, "empty"), None);
    let totp = provider(&home, &provider_settings("totp"));
    assert_eq!(value_of(&totp, "empty"), None);
    let (spawner, _) = standin();
    let held = password.get(Named::new("github"), spawner).expect("held");
    assert_eq!(
        held.expires_in, None,
        "a Bitwarden item has no end of its own"
    );
}

#[test]
fn names_are_listed_once_each_and_nothing_else() {
    let home = Home::new("names");
    let provider = provider(&home, &good());
    let (spawner, _) = standin();
    let names = provider.names(spawner).expect("names").names;
    assert_eq!(names, ["empty", "github", "two lines"]);
}

#[test]
fn it_describes_a_reader_that_lists_and_needs_no_person() {
    let home = Home::new("describe");
    let info = provider(&home, &good()).describe().expect("described");
    assert_eq!(info.title, "Bitwarden");
    assert!(info.can_list && !info.can_store && !info.needs_person);
}

#[test]
fn a_wrong_or_missing_session_key_is_gone_and_the_message_holds_no_key() {
    let home = Home::new("locked");
    for settings in [
        json!({"command": "bw", "session": "not-the-key-zq9"}),
        json!({"command": "bw"}),
    ] {
        let provider = provider(&home, &settings);
        let (spawner, _) = standin();
        let error = provider
            .get(Named::new("github"), spawner)
            .expect_err("locked");
        assert_eq!(
            error.status,
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_GONE
        );
        assert!(error.message().contains("locked"), "{}", error.message());
        assert!(!error.message().contains("not-the-key-zq9"));
    }
}

#[test]
fn a_program_that_is_not_there_is_gone_and_says_what_is_needed() {
    let home = Home::new("absent");
    let provider = provider(
        &home,
        &json!({"command": "no-such-program-bw", "session": GOOD_KEY}),
    );
    let (spawner, _) = standin();
    let error = provider
        .get(Named::new("github"), spawner)
        .expect_err("gone");
    assert_eq!(
        error.status,
        coqui_acp_api::guatiao::Status::GUATIAO_ERR_GONE
    );
    assert!(
        error
            .message()
            .contains("Bitwarden's command line is needed"),
        "{}",
        error.message()
    );
}

#[test]
fn a_command_that_does_not_answer_in_time_is_gone() {
    let home = Home::new("slow");
    let provider = provider(
        &home,
        &json!({"command": "slow-bw", "session": GOOD_KEY, "timeout_seconds": 1}),
    );
    let (spawner, _) = standin();
    let started = std::time::Instant::now();
    let error = provider
        .get(Named::new("github"), spawner)
        .expect_err("gone");
    assert!(error.message().contains("in time"), "{}", error.message());
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn the_session_key_goes_in_the_environment_and_never_on_a_command_line() {
    let home = Home::new("env");
    let provider = provider(&home, &good());
    let (standin, calls) = StandIn::new();
    provider
        .get(Named::new("github"), standin.into_object())
        .expect("held");
    let (standin, _) = (
        StandIn {
            calls: Arc::clone(&calls),
        },
        (),
    );
    provider.names(standin.into_object()).expect("names");
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    for call in calls.iter() {
        assert_eq!(call.program, "bw");
        assert!(call.session_in_env, "{call:?}");
        assert!(!call.key_on_command_line, "{call:?}");
        assert!(call.words.iter().any(|word| word == "--nointeraction"));
    }
}

#[test]
fn a_request_for_a_name_that_is_no_name_is_refused_before_bw_is_run() {
    let home = Home::new("names_refused");
    let provider = provider(&home, &good());
    for name in ["", "a\nb"] {
        let (standin, calls) = StandIn::new();
        let error = provider
            .get(Named::new(name), standin.into_object())
            .expect_err("refused");
        assert_eq!(
            error.status,
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_BAD_VALUE
        );
        assert!(calls.lock().unwrap().is_empty(), "{name:?}");
    }
    // A long name is no refusal of the host's: it is asked of `bw` like any other.
    let (standin, calls) = StandIn::new();
    provider
        .get(Named::new("x".repeat(300)), standin.into_object())
        .expect("a long name is asked");
    assert!(!calls.lock().unwrap().is_empty());
}

#[test]
fn it_keeps_nothing_and_forgets_nothing() {
    let home = Home::new("unsupported");
    let provider = provider(&home, &good());
    let (spawner, _) = standin();
    let error = provider
        .put(Keeping::new("a", "v"), spawner)
        .expect_err("unsupported");
    assert_eq!(
        error.status,
        coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED
    );
    let (spawner, _) = standin();
    let error = provider
        .forget(Named::new("a"), spawner)
        .expect_err("unsupported");
    assert_eq!(
        error.status,
        coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED
    );
}

/// A shared buffer a subscriber writes its lines to, so a test can read what the host logged.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Log {
    type Writer = Log;
    fn make_writer(&'a self) -> Log {
        self.clone()
    }
}

/// A vault whose `[secrets]` is `section`, over the bundled providers, with the stand-in for `bw` as the only program anything runs.
fn vault(home: &Home, section: Value) -> (Arc<super::vault::Vault>, Arc<Mutex<Vec<Call>>>) {
    let (standin, calls) = StandIn::new();
    let shared = Arc::new(standin);
    let vault = super::vault::Vault::with_clock(
        Catalog::built_in_in(&home.0),
        Arc::new(move || Ok(Some(section.clone()))),
        Arc::new(|| 1_000_000),
    );
    let vault = vault.with_spawners(Arc::new(move || {
        StandIn {
            calls: Arc::clone(&shared.calls),
        }
        .into_object()
    }));
    (vault, calls)
}

fn recalled(vault: &super::vault::Vault, name: &str) -> Option<String> {
    vault
        .recall("s1", name)
        .map(|value| String::from_utf8(value.to_vec()).expect("text"))
}

#[test]
fn a_vault_opens_bitwarden_with_a_key_taken_from_another_provider_and_logs_no_value() {
    let home = Home::new("from");
    let (vault, calls) = vault(
        &home,
        json!({
            "names": {"github": "bw", "ghost": "bw"},
            "providers": {"bw": {
                "provider": "bitwarden", "keep": "close", "command": "bw",
                "from": {"session": {"entry": "form", "name": "bw_session"}}
            }}
        }),
    );
    let log = Log::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        // Locked: the key is not held, so the vault is not even opened and the person is asked for the secret itself.
        assert_eq!(recalled(&vault, "github"), None);
        assert!(calls.lock().unwrap().is_empty(), "bw was run without a key");
        // A wrong key is the provider's `GONE`, which the host reads as nothing held.
        vault
            .remember(None, "bw_session", b"not-the-key-zq9", "close")
            .unwrap();
        assert_eq!(recalled(&vault, "github"), None);
        assert_eq!(calls.lock().unwrap().len(), 1);
        // The right key, which another provider (the form) gave, opens it; an item that is not there is no value.
        vault
            .remember(None, "bw_session", GOOD_KEY.as_bytes(), "close")
            .unwrap();
        assert_eq!(recalled(&vault, "github").as_deref(), Some(VALUE));
        assert_eq!(recalled(&vault, "ghost"), None);
    });
    let logged = String::from_utf8(log.0.lock().unwrap().clone()).expect("text");
    assert!(!logged.is_empty(), "the host logs what it could not do");
    for secret in [VALUE, GOOD_KEY, "not-the-key-zq9"] {
        assert!(!logged.contains(secret), "the log holds a value: {logged}");
    }
    let rows = vault.rows();
    let row = rows
        .iter()
        .find(|row| row.name == "github")
        .expect("what the script gave is in the table");
    assert_eq!(row.provider, "bw");
    assert_eq!(row.expires_in, None);
    // The key went to bw in its environment, never on a command line.
    for call in calls.lock().unwrap().iter() {
        assert!(call.session_in_env && !call.key_on_command_line, "{call:?}");
    }
}

#[test]
fn a_setting_is_taken_from_a_secret_only_when_its_provider_says_it_may_be() {
    let home = Home::new("from_refused");
    let (vault, calls) = vault(
        &home,
        json!({
            "names": {"github": "bw"},
            "providers": {"bw": {
                "provider": "bitwarden", "command": "bw",
                "from": {"part": {"entry": "form", "name": "which_part"}}
            }}
        }),
    );
    vault
        .remember(None, "which_part", b"username", "close")
        .unwrap();
    // `part` is a setting but not one that may be a secret, so the provider is not made and not asked.
    assert_eq!(recalled(&vault, "github"), None);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_setting_that_names_a_secret_through_a_loop_of_providers_is_not_followed_for_ever() {
    let home = Home::new("loop");
    let (vault, calls) = vault(
        &home,
        json!({
            "names": {"github": "a"},
            "providers": {
                "a": {"provider": "bitwarden", "from": {"session": {"entry": "b", "name": "x"}}},
                "b": {"provider": "bitwarden", "from": {"session": {"entry": "a", "name": "y"}}}
            }
        }),
    );
    assert_eq!(recalled(&vault, "github"), None);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn an_entry_gives_a_setting_once_and_names_only_entries_that_exist() {
    use super::settings::Settings;

    let read = |section: Value| Settings::read(Some(&section));
    let twice = read(json!({"providers": {"bw": {
        "provider": "bitwarden", "session": "x",
        "from": {"session": {"entry": "form", "name": "n"}}
    }}}));
    assert!(twice.is_err_and(|why| why.contains("twice")));
    let ghost = read(json!({"providers": {"bw": {
        "provider": "bitwarden", "from": {"session": {"entry": "ghost", "name": "n"}}
    }}}));
    assert!(ghost.is_err_and(|why| why.contains("ghost")));
    let shapeless = read(json!({"providers": {"bw": {
        "provider": "bitwarden", "from": {"session": "form"}
    }}}));
    assert!(shapeless.is_err());
    let fine = read(json!({"providers": {"bw": {
        "provider": "bitwarden", "from": {"session": {"entry": "form", "name": "n"}}
    }}}))
    .expect("an entry taking its key from the form");
    assert_eq!(fine.entries["bw"].from["session"].name, "n");
    assert!(
        fine.entries["bw"].config.is_empty(),
        "from is not a setting"
    );
}

/// The real process spawner and a real program standing in for `bw` (`tests/bw_standin.py`, run by the interpreter `CANOA_TEST_PYTHON`
/// names): what the provider asks of a spawner is what a spawner really does with a child's pipes, environment and exit status.
#[cfg(canoa_test_python)]
#[test]
fn end_to_end_the_provider_runs_a_real_program_through_the_real_spawner() {
    let home = Home::new("e2e");
    let standin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/canoa-bitwarden/tests/bw_standin.py");
    let section = json!({
        "names": {"github": "bw", "ghost": "bw"},
        "providers": {"bw": {
            "provider": "bitwarden", "keep": "close",
            "command": env!("CANOA_TEST_PYTHON"), "arguments": [standin.to_string_lossy()],
            "from": {"session": {"entry": "form", "name": "bw_session"}}
        }}
    });
    let vault = super::vault::Vault::with_clock(
        Catalog::built_in_in(&home.0),
        Arc::new(move || Ok(Some(section.clone()))),
        Arc::new(|| 1_000_000),
    );
    assert_eq!(recalled(&vault, "github"), None, "locked");
    vault
        .remember(None, "bw_session", b"not-the-key-zq9", "close")
        .unwrap();
    assert_eq!(recalled(&vault, "github"), None, "a wrong key");
    vault
        .remember(None, "bw_session", GOOD_KEY.as_bytes(), "close")
        .unwrap();
    assert_eq!(recalled(&vault, "github").as_deref(), Some(VALUE));
    assert_eq!(recalled(&vault, "ghost"), None);
}

#[cfg(not(canoa_test_python))]
#[test]
#[ignore = "NOT RUN: set CANOA_TEST_PYTHON to a Python 3 that runs the stand-in for bw"]
fn end_to_end_the_provider_runs_a_real_program_through_the_real_spawner() {
    panic!("CANOA_TEST_PYTHON names no Python");
}
