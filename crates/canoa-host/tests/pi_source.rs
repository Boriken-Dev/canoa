//! The host's Pi source: Pi runs through coqui-acp-pi's provider inside the host, with no Node and no adapter. It is ready
//! when a `pi` is found on this computer or the Store holds a managed copy, and a managed copy runs as its Node and script.
//! Here `pi` is a stand-in that speaks Pi's RPC. One test in this file, since it sets the process's environment.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use std::path::Path;

use canoa::store::Launch;
use canoa_host::store::shelf::{MadeBy, Shelf, record_tree};
use coqui_acp_api::guatiao::{Map, Value as Setting};
use linked::{Hosted, ask, open_session, said};
use serde_json::{Value, json};
use support::{Scratch, cwd, example};

fn env(name: &str, value: &Path) {
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var(name, value) };
}

/// One turn of the source `Pi`, with `text` as the prompt: the answer and what the agent said.
fn turn(host: &Hosted, text: &str) -> (Value, Vec<Value>) {
    let link = host.local.open("Pi").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    ask(
        &link,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
    )
    .expect("the turn is answered")
}

/// The arguments of each start the stand-in logged.
fn starts(file: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            line.get("args")?.as_array().map(|args| {
                args.iter()
                    .filter_map(|arg| arg.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .collect()
}

#[test]
fn pi_runs_through_the_hosts_own_provider_from_a_pi_on_the_path_or_a_managed_copy() {
    let root = Scratch::new();
    let bin = root.0.join("bin");
    let empty = root.0.join("empty");
    let home = root.0.join("home");
    let data = root.0.join("data");
    for dir in [&bin, &empty, &home, &data] {
        std::fs::create_dir_all(dir).expect("made");
    }
    // Neither a real Pi nor a real Store copy is ever looked at, and no Node is on the path.
    env("USERPROFILE", &home);
    env("HOME", &home);
    env("LOCALAPPDATA", &data);
    env("XDG_DATA_HOME", &data);
    env("PI_CODING_AGENT_DIR", &home.join("pi-agent"));
    let log = root.0.join("fake-pi.log");
    env("COQUI_ACP_FAKE_PI_LOG", &log);
    let standin = example("fake_pi", false);
    let suffix = std::env::consts::EXE_SUFFIX;

    // 1. Nothing found: Pi is served and says it misses `pi` and nothing else.
    env("PATH", &empty);
    let host = Hosted::start_with("", &["--local-sources"]);
    let line = host.line_of("Pi").expect("printed");
    assert_eq!(line["missing"], json!(["pi"]));
    assert_eq!(line["harness"], "pi");
    assert!(line.get("url").is_none(), "no address: {line}");
    host.stop();

    // 2. Only a `pi` on the path, and the person's protected list: ready, and a turn runs through the provider.
    std::fs::copy(&standin, bin.join(format!("pi{suffix}"))).expect("copied");
    env("PATH", &bin);
    let config = "[permissions]\nprotected = [\"secret\"]\n";
    let host = Hosted::start_with(config, &["--local-sources"]);
    assert_eq!(host.line_of("Pi").expect("printed")["missing"], json!([]));
    let work = Path::new(&cwd(&host.scratch)).to_path_buf();
    std::fs::create_dir_all(work.join("secret")).expect("made");
    std::fs::write(work.join("secret").join("notes.txt"), "private").expect("written");
    std::fs::write(work.join("plain.txt"), "public").expect("written");
    let (answer, updates) = turn(&host, "hello");
    assert_eq!(answer["stopReason"], "end_turn");
    assert_eq!(said(&updates), "echo: hello");
    let started = starts(&log);
    assert_eq!(started.len(), 1, "{} starts", started.len());
    assert!(started[0].contains(&"rpc".to_owned()), "{:?}", started[0]);
    assert!(
        !started[0].iter().any(|arg| arg.ends_with("cli.js")),
        "a pi on the path is run as itself"
    );
    // The person's list reaches the provider: a read under `secret` is refused, one beside it is not.
    let refused = turn(&host, "[[read:secret/notes.txt]]").1;
    let allowed = turn(&host, "[[read:plain.txt]]").1;
    let failed = |updates: &[Value]| {
        updates.iter().any(|update| {
            update["params"]["update"]["status"] == "failed"
                || update["params"]["update"]["rawOutput"]
                    .to_string()
                    .contains("protected")
        })
    };
    assert!(failed(&refused), "{} updates, none refused", refused.len());
    assert!(!failed(&allowed), "a read beside it was refused");
    host.stop();

    // 3. A managed copy, no `pi` anywhere: it runs as its Node and script, never through a program looked up.
    env("PATH", &empty);
    let shelf = Shelf::unchecked(data.join("Canoa").join("programs"));
    let program = shelf.program("pi").expect("a name");
    let locked = program.lock().expect("locked");
    let staging = locked.stage("1.5.2").expect("staged");
    let node = staging.join(format!("node{suffix}"));
    let script = staging.join("cli.js");
    std::fs::copy(&standin, &node).expect("copied");
    std::fs::write(&script, "// Pi's CLI\n").expect("written");
    let mut launch = Launch::new(node.to_string_lossy().into_owned());
    launch.settings = settings_of(&node, &script);
    let mut files = Vec::new();
    record_tree(&staging, &mut files).expect("recorded");
    locked
        .commit(
            &staging,
            "1.5.2",
            &launch,
            files,
            MadeBy {
                provider: "pi_store".to_owned(),
                version: "0".to_owned(),
            },
        )
        .expect("committed");
    drop(locked);
    std::fs::remove_file(&log).expect("removed");
    let host = Hosted::start_with("", &["--local-sources"]);
    assert_eq!(host.line_of("Pi").expect("printed")["missing"], json!([]));
    let (answer, updates) = turn(&host, "again");
    assert_eq!(answer["stopReason"], "end_turn");
    assert_eq!(said(&updates), "echo: again");
    let started = starts(&log);
    let first = started
        .first()
        .and_then(|args| args.first())
        .expect("a start");
    assert_eq!(Path::new(first), script, "the script is the first argument");
    host.stop();
}

fn settings_of(node: &Path, script: &Path) -> Setting {
    let mut settings = Map::new();
    settings.set("node", Setting::from(node.to_string_lossy().as_ref()));
    settings.set("script", Setting::from(script.to_string_lossy().as_ref()));
    Setting::from(settings)
}
