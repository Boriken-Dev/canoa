//! `foundry/models` and `foundry/model`: the models of the person's Foundry Local, listed and moved through its own command line.
//!
//! The program is the Store's copy when there is one, else the first `foundry` on the `PATH`. Every call runs it as this user with
//! `-o json`; nothing is downloaded or loaded unless the key asked for that model by name, and what the program refuses is
//! shown as it said it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes and files are std's"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use canoa_foundry::models::{self, Listing, ModelAction};
use canoa_foundry::window;
use serde_json::{Value, json};

use super::files::Files;
use super::{REFUSED, bad, text_param};
use crate::spawn::{Command, Stdio};

type Outcome = Result<Value, (i32, String)>;

/// The most a program's answer is read to: a catalog of a few hundred models is a few hundred kilobytes.
const MOST_OUTPUT: u64 = 8 * 1024 * 1024;

/// How long each call is given before the program is ended: a download is the long one, and its size is the model's.
const LIST_TIME: Duration = Duration::from_secs(120);
const DOWNLOAD_TIME: Duration = Duration::from_secs(4 * 60 * 60);
const LOAD_TIME: Duration = Duration::from_secs(15 * 60);
const UNLOAD_TIME: Duration = Duration::from_secs(2 * 60);

/// What a finished program printed.
struct Done {
    code: Option<i32>,
    out: String,
    err: String,
}

/// The program calls start: the Store's copy of Foundry Local when it is as installed, else the first `foundry` on the `PATH`.
pub(crate) fn locate() -> Option<PathBuf> {
    if let Some((path, _)) = crate::sources::managed_program("foundry") {
        return Some(path);
    }
    let path = std::env::var_os("PATH")?;
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    super::registry::which_in(&path, &pathext, "foundry")
}

fn read_all(source: impl std::io::Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = source.take(MOST_OUTPUT).read_to_end(&mut bytes);
        bytes
    })
}

/// Runs `program` with `args` to its end, or ends it at `limit`.
fn run(
    program: &Path,
    env: &[(OsString, OsString)],
    args: &[String],
    limit: Duration,
) -> Result<Done, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().cloned())
        .stdin(Stdio::Null)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Foundry Local could not be started: {error}"))?;
    let out = child.stdout.take().map(read_all);
    let err = child.stderr.take().map(read_all);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "Foundry Local did not finish within {} minutes and was ended",
                    limit.as_secs().div_ceil(60)
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => break Err(format!("Foundry Local: {error}")),
        }
    };
    let text = |handle: Option<thread::JoinHandle<Vec<u8>>>| {
        handle
            .and_then(|handle| handle.join().ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    let (out, err) = (text(out), text(err));
    status.map(|status| Done {
        code: status.code(),
        out,
        err,
    })
}

/// A refusal in the program's own words.
fn refusal(what: &str, done: &Done) -> (i32, String) {
    let said = models::plain(&format!("{}\n{}", done.err, done.out), 600);
    let how = done
        .code
        .map_or_else(|| "was ended".to_owned(), |code| format!("exited {code}"));
    (
        REFUSED,
        if said.is_empty() {
            format!("{what}: Foundry Local {how}")
        } else {
            format!("{what}: {said}")
        },
    )
}

fn rows_of(done: &Done) -> Option<Vec<models::Row>> {
    serde_json::from_str::<Value>(done.out.trim())
        .ok()
        .map(|answer| models::rows(&answer))
}

/// Where Foundry Local keeps the models it downloads, under the person's home.
fn cache_of(home: &Path) -> PathBuf {
    home.join(".foundry").join("cache").join("models")
}

/// The window a downloaded model has (its own `genai_config.json`'s) and the one the person chose for it; null for the numbers of a
/// model that is not downloaded, or whose folder is not where Foundry Local keeps them.
fn window_json(
    model: &models::Model,
    home: Option<&Path>,
    chosen: &BTreeMap<String, u64>,
) -> Value {
    let own = home
        .filter(|_| model.cached)
        .and_then(|home| window::cached_folder(&cache_of(home), &model.row.id))
        .and_then(|folder| window::read(&folder).ok());
    json!({
        "context_length": own.and_then(|own| own.context_length),
        "max_length": own.and_then(|own| own.max_length),
        "chosen": chosen.get(&model.row.id),
    })
}

fn model_json(model: &models::Model, home: Option<&Path>, chosen: &BTreeMap<String, u64>) -> Value {
    json!({
        "id": model.row.id,
        "alias": model.row.alias,
        "task": model.row.task,
        "device": model.row.device,
        "size_mb": model.row.size_mb,
        "description": models::describe(&model.row),
        "cached": model.cached,
        "loaded": model.loaded,
        "window": window_json(model, home, chosen),
    })
}

/// `foundry/models` -> `{"installed", "program"?, "models": [{id, alias?, task?, device?, size_mb?, description, cached,
/// loaded, window: {context_length, max_length, chosen}}]}`: the catalog's variants with what is downloaded and what is in memory
/// marked, loaded first; each one's window is what its own configuration says (null for a model not downloaded) and what the
/// person chose in canoa.cfg's `[foundry.windows]`. Without Foundry Local it is `{"installed": false, "models": []}`.
pub(super) fn models(program: Option<PathBuf>, windows: &BTreeMap<String, u64>) -> Outcome {
    models_with(program, &[], std::env::home_dir().as_deref(), windows)
}

/// `home` is where the service's `.foundry/daemon.json` is: with no service running nothing is in memory, and listing what is
/// loaded would be a command that may start one, which listing never does.
fn models_with(
    program: Option<PathBuf>,
    env: &[(OsString, OsString)],
    home: Option<&Path>,
    windows: &BTreeMap<String, u64>,
) -> Outcome {
    let Some(program) = program else {
        return Ok(json!({"installed": false, "models": []}));
    };
    let ask = |which: Listing| -> Result<Done, (i32, String)> {
        run(&program, env, &models::list_args(which), LIST_TIME).map_err(|why| (REFUSED, why))
    };
    let all = ask(Listing::All)?;
    if all.code != Some(0) {
        return Err(refusal("the model catalog was not listed", &all));
    }
    let all = rows_of(&all).ok_or_else(|| {
        (
            REFUSED,
            "the model catalog was not listed: Foundry Local did not answer in JSON".to_owned(),
        )
    })?;
    let cached = ask(Listing::Cached)?;
    if cached.code != Some(0) {
        return Err(refusal("the downloaded models were not listed", &cached));
    }
    let cached = rows_of(&cached).unwrap_or_default();
    // Nothing is in memory when no service runs; a service that went away leaves its file and the program fails: also none.
    let serving = home.is_some_and(|home| canoa_foundry::running_endpoint(home).is_some());
    let loaded = if serving {
        let loaded = ask(Listing::Loaded)?;
        if loaded.code == Some(0) {
            rows_of(&loaded).unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let found = models::merge(all, &cached, &loaded);
    Ok(json!({
        "installed": true,
        "program": program.display().to_string(),
        "models": found
            .iter()
            .map(|model| model_json(model, home, windows))
            .collect::<Vec<_>>(),
    }))
}

/// `foundry/model` `{"model", "action"}` (`download`, `load` or `unload`) -> `{"ok": true, "model", "action", "note"?}`: the
/// program is asked to do that to that one model. A load keeps the model in memory until the person stops Foundry Local's
/// service, which the answer says.
pub(super) fn act(params: &Value, program: Option<PathBuf>) -> Outcome {
    act_with(params, program, &[], None)
}

fn act_with(
    params: &Value,
    program: Option<PathBuf>,
    env: &[(OsString, OsString)],
    limit: Option<Duration>,
) -> Outcome {
    let model = text_param(params, "model")?.ok_or_else(|| bad("model is needed"))?;
    let word = text_param(params, "action")?.ok_or_else(|| bad("action is needed"))?;
    let action =
        ModelAction::parse(word).ok_or_else(|| bad("action is download, load or unload"))?;
    let args = models::action_args(action, model).map_err(|why| bad(&why))?;
    let program = program.ok_or_else(|| {
        (
            REFUSED,
            "Foundry Local is not installed: install it from the Store's Runtimes".to_owned(),
        )
    })?;
    let limit = limit.unwrap_or(match action {
        ModelAction::Download => DOWNLOAD_TIME,
        ModelAction::Load => LOAD_TIME,
        ModelAction::Unload => UNLOAD_TIME,
    });
    let done = run(&program, env, &args, limit).map_err(|why| (REFUSED, why))?;
    if done.code != Some(0) {
        return Err(refusal(&format!("{word} {model}"), &done));
    }
    let note = match action {
        ModelAction::Load => Some(
            "The model stays in memory until Foundry Local's service is stopped (`foundry server stop`).",
        ),
        ModelAction::Download => Some("The model is in Foundry Local's cache."),
        ModelAction::Unload => None,
    };
    Ok(json!({"ok": true, "model": model, "action": word, "note": note}))
}

/// `foundry/window` `{"model", "window"}` -> `{"ok": true, "model", "window", "note"?, "applies"}`: keeps `window` tokens as the
/// window Foundry Local's model `model` is run with (`[foundry.windows]` in canoa.cfg), or takes the setting away when `window` is
/// null. A window above the model's own context length is lowered to it, and the answer says so. The setting is used when the
/// model is run by the SDK engine, which makes Canoa's own copy of the model's folder; the Foundry Local service runs the model
/// as it is cached, whatever is set here.
pub(super) fn window(params: &Value, files: &Files) -> Outcome {
    window_with(params, files, std::env::home_dir().as_deref())
}

fn window_with(params: &Value, files: &Files, home: Option<&Path>) -> Outcome {
    let model = text_param(params, "model")?.ok_or_else(|| bad("model is needed"))?;
    if !models::valid_model(model) {
        return Err(bad("a model is 1 to 128 letters, digits and . _ - : + /"));
    }
    let wanted = match params.get("window") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().filter(|tokens| *tokens > 0).ok_or_else(|| {
            bad("window is a whole number of tokens, at least 1, or null to remove the setting")
        })?),
    };
    let mut kept = wanted;
    let mut note = None;
    if let Some(tokens) = wanted {
        let limit = home
            .and_then(|home| window::cached_folder(&cache_of(home), model))
            .and_then(|folder| window::read(&folder).ok())
            .and_then(|own| own.limit());
        if let Some(limit) = limit {
            let (lowered, said) =
                window::clamp(tokens, limit).map_err(|why| bad(&why.to_string()))?;
            kept = Some(lowered);
            note = said;
        }
    }
    files.set_foundry_window(model, kept)?;
    Ok(json!({
        "ok": true,
        "model": model,
        "window": kept,
        "note": note,
        "applies": "Used when the model is run by the SDK engine, from the next time its gateway starts; the Foundry Local service runs the model as it is cached.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standin() -> PathBuf {
        let exe = std::env::current_exe().expect("test path");
        let dir = exe.parent().expect("deps").parent().expect("debug");
        let name = if cfg!(windows) {
            "foundry_standin.exe"
        } else {
            "foundry_standin"
        };
        let path = dir.join("examples").join(name);
        assert!(
            path.is_file(),
            "build the examples first: cargo build -p canoa-host --examples ({})",
            path.display()
        );
        path
    }

    fn state(name: &str) -> (PathBuf, Vec<(OsString, OsString)>) {
        let dir = std::env::temp_dir().join(format!("canoa-foundry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a state folder");
        let env = vec![(
            OsString::from("FOUNDRY_STANDIN_STATE"),
            dir.clone().into_os_string(),
        )];
        (dir, env)
    }

    fn marks(listed: &Value) -> Vec<(String, bool, bool)> {
        listed["models"]
            .as_array()
            .expect("models")
            .iter()
            .map(|m| {
                (
                    m["id"].as_str().expect("id").to_owned(),
                    m["cached"].as_bool().expect("cached"),
                    m["loaded"].as_bool().expect("loaded"),
                )
            })
            .collect()
    }

    #[test]
    fn a_key_that_may_only_read_the_store_cannot_run_foundry_local() {
        use super::super::{
            Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
        };
        let control = Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens");
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: [Grant::StoreRead].into_iter().collect(),
        };
        for method in ["foundry/models", "foundry/model"] {
            let line = serde_json::to_vec(
                &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {"model": "m", "action": "load"}}),
            )
            .expect("encodes");
            let answer: Value =
                serde_json::from_slice(&control.handle(&caller, &line).expect("answered"))
                    .expect("JSON");
            assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        }
    }

    #[test]
    fn without_foundry_local_nothing_is_listed_and_nothing_can_be_moved() {
        let listed = models(None, &BTreeMap::new()).expect("an answer");
        assert_eq!(listed, json!({"installed": false, "models": []}));
        let refused = act(&json!({"model": "m", "action": "load"}), None).expect_err("refused");
        assert_eq!(refused.0, REFUSED);
        assert!(refused.1.contains("not installed"), "{}", refused.1);
    }

    #[test]
    fn a_model_is_downloaded_then_loaded_then_unloaded_and_the_listing_follows() {
        let (dir, env) = state("flow");
        let program = Some(standin());
        let home = dir.join("home");
        std::fs::create_dir_all(home.join(".foundry")).expect("a home");
        let id = "qwen3-14b-generic-gpu:2";
        let listed =
            models_with(program.clone(), &env, Some(&home), &BTreeMap::new()).expect("listed");
        assert_eq!(listed["installed"], true);
        assert_eq!(marks(&listed).len(), 3);
        assert!(
            marks(&listed)
                .iter()
                .all(|(_, cached, loaded)| !cached && !loaded)
        );
        let first = listed["models"]
            .as_array()
            .and_then(|all| all.iter().find(|m| m["id"] == id))
            .expect("the model");
        assert_eq!(first["alias"], "qwen3-14b");
        assert_eq!(first["description"], "chat, gpu, 9.9 GB");

        // A model that is not downloaded is refused with the program's own words, and stays unloaded.
        let early = act_with(
            &json!({"model": id, "action": "load"}),
            program.clone(),
            &env,
            None,
        )
        .expect_err("not downloaded");
        assert_eq!(early.0, REFUSED);
        assert!(early.1.contains("is not downloaded"), "{}", early.1);

        let done = act_with(
            &json!({"model": id, "action": "download"}),
            program.clone(),
            &env,
            None,
        )
        .expect("downloaded");
        assert_eq!(done["ok"], true);
        let listed =
            models_with(program.clone(), &env, Some(&home), &BTreeMap::new()).expect("listed");
        assert!(marks(&listed).contains(&(id.to_owned(), true, false)));

        let done = act_with(
            &json!({"model": id, "action": "load"}),
            program.clone(),
            &env,
            None,
        )
        .expect("loaded");
        assert!(
            done["note"]
                .as_str()
                .is_some_and(|n| n.contains("server stop")),
            "{done}"
        );
        let listed =
            models_with(program.clone(), &env, Some(&home), &BTreeMap::new()).expect("listed");
        assert!(
            marks(&listed).contains(&(id.to_owned(), true, false)),
            "with no service running nothing is asked to be in memory"
        );
        std::fs::write(
            home.join(".foundry").join("daemon.json"),
            r#"{"web_urls": ["http://127.0.0.1:60570"]}"#,
        )
        .expect("a daemon file");
        let listed =
            models_with(program.clone(), &env, Some(&home), &BTreeMap::new()).expect("listed");
        assert_eq!(
            marks(&listed)[0],
            (id.to_owned(), true, true),
            "loaded models come first"
        );

        act_with(
            &json!({"model": id, "action": "unload"}),
            program.clone(),
            &env,
            None,
        )
        .expect("unloaded");
        let listed = models_with(program, &env, Some(&home), &BTreeMap::new()).expect("listed");
        assert!(marks(&listed).contains(&(id.to_owned(), true, false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_that_could_be_an_option_or_an_action_that_is_not_one_never_reaches_the_program() {
        let (dir, env) = state("names");
        for params in [
            json!({"model": "--help", "action": "load"}),
            json!({"model": "a b", "action": "load"}),
            json!({"model": "qwen3-14b-generic-gpu:2", "action": "register"}),
            json!({"model": "qwen3-14b-generic-gpu:2"}),
            json!({"action": "load"}),
        ] {
            let refused = act_with(&params, Some(standin()), &env, None).expect_err("refused");
            assert_eq!(refused.0, super::super::BAD_PARAMS, "{params}");
        }
        assert!(!dir.join("cached").exists() && !dir.join("loaded").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_catalog_is_shown_in_the_programs_words_without_its_escapes() {
        let (dir, mut env) = state("fail");
        env.push((OsString::from("FOUNDRY_STANDIN_FAIL"), OsString::from("1")));
        let refused =
            models_with(Some(standin()), &env, None, &BTreeMap::new()).expect_err("refused");
        assert_eq!(refused.0, REFUSED);
        assert_eq!(
            refused.1,
            "the model catalog was not listed: error: the catalog is not reachable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_that_does_not_finish_in_time_is_ended() {
        let (dir, mut env) = state("slow");
        env.push((
            OsString::from("FOUNDRY_STANDIN_SLEEP_MS"),
            OsString::from("20000"),
        ));
        let started = Instant::now();
        let refused = act_with(
            &json!({"model": "qwen3-14b-generic-gpu:2", "action": "download"}),
            Some(standin()),
            &env,
            Some(Duration::from_millis(300)),
        )
        .expect_err("ended");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(refused.0, REFUSED);
        assert!(refused.1.contains("was ended"), "{}", refused.1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn cached_model(home: &Path, id: &str, context: u64, max: u64) {
        let (name, version) = id.rsplit_once(':').expect("a variant id");
        let folder = cache_of(home)
            .join("Microsoft")
            .join(format!("{name}-{version}"))
            .join(format!("v{version}"));
        std::fs::create_dir_all(&folder).expect("a model folder");
        std::fs::write(
            folder.join("genai_config.json"),
            format!(
                "{{\"model\": {{\"context_length\": {context}}}, \"search\": {{\"max_length\": {max}}}}}"
            ),
        )
        .expect("a configuration");
    }

    fn files_in(dir: &Path) -> Files {
        Files::new(
            Some(dir.to_path_buf()),
            Vec::new(),
            Some(dir.join("canoa.cfg")),
            None,
            super::super::UploadLimits::default(),
        )
    }

    #[test]
    fn a_downloaded_model_lists_its_own_window_and_the_one_chosen() {
        let (dir, env) = state("windows");
        let program = Some(standin());
        let home = dir.join("home");
        std::fs::create_dir_all(home.join(".foundry")).expect("a home");
        let id = "qwen3-14b-generic-gpu:2";
        cached_model(&home, id, 40960, 8192);
        let chosen = BTreeMap::from([(id.to_owned(), 16384_u64)]);

        // Not downloaded as far as the program says: no numbers, but the person's choice is still shown.
        let listed = models_with(program.clone(), &env, Some(&home), &chosen).expect("listed");
        let row = |listed: &Value| {
            listed["models"]
                .as_array()
                .and_then(|all| all.iter().find(|m| m["id"] == id))
                .expect("the model")
                .clone()
        };
        assert_eq!(
            row(&listed)["window"],
            json!({"context_length": null, "max_length": null, "chosen": 16384})
        );

        act_with(
            &json!({"model": id, "action": "download"}),
            program.clone(),
            &env,
            None,
        )
        .expect("downloaded");
        let listed = models_with(program, &env, Some(&home), &chosen).expect("listed");
        assert_eq!(
            row(&listed)["window"],
            json!({"context_length": 40960, "max_length": 8192, "chosen": 16384})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_is_kept_in_canoa_cfg_per_model_capped_at_the_models_own_and_taken_away_with_null() {
        let (dir, _) = state("keep");
        let home = dir.join("home");
        let id = "qwen3-14b-generic-gpu:2";
        cached_model(&home, id, 40960, 8192);
        let files = files_in(&dir);

        let kept =
            window_with(&json!({"model": id, "window": 16384}), &files, Some(&home)).expect("kept");
        assert_eq!(
            (kept["window"].clone(), kept["note"].clone()),
            (json!(16384), Value::Null)
        );
        assert_eq!(files.foundry_windows().get(id), Some(&16384));
        let text = std::fs::read_to_string(dir.join("canoa.cfg")).expect("written");
        assert!(text.contains("windows") && text.contains("16384"), "{text}");

        let capped = window_with(
            &json!({"model": id, "window": 100_000}),
            &files,
            Some(&home),
        )
        .expect("kept");
        assert_eq!(
            capped["window"], 40960,
            "the model's own context length is the most"
        );
        assert!(
            capped["note"]
                .as_str()
                .is_some_and(|n| n.contains("100000")),
            "{capped}"
        );
        assert_eq!(files.foundry_windows().get(id), Some(&40960));

        // A model that is not downloaded has no limit to check against, so the number is kept as given.
        window_with(
            &json!({"model": "other:1", "window": 5000}),
            &files,
            Some(&home),
        )
        .expect("kept");
        assert_eq!(files.foundry_windows().len(), 2);

        let removed = window_with(&json!({"model": id, "window": null}), &files, Some(&home))
            .expect("removed");
        assert_eq!(removed["window"], Value::Null);
        assert_eq!(
            files.foundry_windows(),
            BTreeMap::from([("other:1".to_owned(), 5000)])
        );
        window_with(
            &json!({"model": "other:1", "window": null}),
            &files,
            Some(&home),
        )
        .expect("removed");
        assert!(files.foundry_windows().is_empty());
        let text = std::fs::read_to_string(dir.join("canoa.cfg")).expect("read");
        assert!(
            !text.contains("foundry"),
            "an empty table is not left behind: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_that_is_not_a_number_of_tokens_or_a_name_that_is_not_a_model_is_refused() {
        let (dir, _) = state("badwindow");
        let files = files_in(&dir);
        for params in [
            json!({"model": "m", "window": 0}),
            json!({"model": "m", "window": -4}),
            json!({"model": "m", "window": "big"}),
            json!({"model": "m", "window": 1.5}),
            json!({"model": "--help", "window": 4096}),
            json!({"window": 4096}),
        ] {
            let refused = window_with(&params, &files, None).expect_err("refused");
            assert_eq!(refused.0, super::super::BAD_PARAMS, "{params}");
        }
        assert!(files.foundry_windows().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_that_may_only_read_the_store_cannot_set_a_window() {
        use super::super::{
            Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
        };
        let control = Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens");
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: [Grant::StoreRead].into_iter().collect(),
        };
        let line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "foundry/window", "params": {"model": "m", "window": 4096}}),
        )
        .expect("encodes");
        let answer: Value =
            serde_json::from_slice(&control.handle(&caller, &line).expect("answered"))
                .expect("JSON");
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
}
