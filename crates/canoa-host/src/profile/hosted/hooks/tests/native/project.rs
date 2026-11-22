//! A project's context files through a host whose agent is a stand-in: which files reach the session and where, what a harness that
//! loads them itself is not given a second time, and what a directory the session enters adds.

use super::*;

/// The files of a project, each by its path below the project's folder, made under the test's own directory; the folder.
fn project(temp: &Temp, files: &[(&str, &str)]) -> PathBuf {
    let root = temp.0.join("proj");
    std::fs::create_dir_all(&root).expect("a folder");
    for (path, text) in files {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().expect("a parent")).expect("a folder");
        std::fs::write(at, text).expect("written");
    }
    root
}

/// A session opened in `cwd` on `harness`, whose agent takes a system prompt when `takes`: the request the host sends the agent,
/// and what the client was told when the agent answered it.
fn open<'a>(
    hosted: Hosted<'a, Client>,
    client: &'a Client,
    harness: &str,
    cwd: &Path,
    takes: bool,
) -> (Session<'a>, Value, Value) {
    send(
        client,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    );
    hosted
        .receive(Some(Duration::from_millis(10)))
        .expect("read");
    let capabilities = if takes {
        json!({"sessionCapabilities": {"systemPrompt": true}})
    } else {
        json!({})
    };
    let answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
        "agentCapabilities": capabilities, "agentInfo": {"name": harness}}});
    hosted
        .send(&serde_json::to_vec(&answer).expect("JSON"))
        .expect("sent");
    send(
        client,
        2,
        "session/new",
        json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
    );
    let request = hosted
        .receive(Some(Duration::from_millis(10)))
        .expect("read")
        .expect("the request goes on");
    let request: Value = serde_json::from_slice(&request).expect("JSON");
    let plugin = request
        .pointer("/params/_meta/claudeCode/options/plugins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|plugin| plugin["path"].as_str())
        .map(PathBuf::from)
        .find(|path| path.join("hooks").join("hooks.json").exists());
    let answer = json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}});
    hosted
        .send(&serde_json::to_vec(&answer).expect("JSON"))
        .expect("sent");
    let told = {
        let to = client.to.lock().expect("lock");
        serde_json::from_slice(to.last().expect("a message")).expect("JSON")
    };
    let session = Session {
        hosted,
        client,
        next: 3,
        plugin,
        id: "s1".to_owned(),
    };
    (session, request, told)
}

/// The texts of the content blocks of a request's system prompt, or of a prompt's blocks.
fn texts(blocks: &Value) -> Vec<String> {
    blocks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str().map(str::to_owned))
        .collect()
}

/// What the project's context adds to `texts`: the files' own text, with the label that comes before each left out, and the
/// paths the labels name.
fn files_in(texts: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut paths, mut bodies) = (Vec::new(), Vec::new());
    let mut iter = texts.iter();
    while let Some(text) = iter.next() {
        if let Some(rest) = text.strip_prefix("The project's context file ") {
            paths.push(
                rest.trim_end_matches(", not written by the user in this chat:")
                    .to_owned(),
            );
            bodies.push(iter.next().cloned().unwrap_or_default());
        }
    }
    (paths, bodies)
}

/// The items of the report the client was told.
fn items(told: &Value) -> Vec<Value> {
    told["result"]["_meta"]["dev.boriken.canoa/profile"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Runs `then` on a host with no overlay and a session of `harness` opened in the project's folder.
fn run(
    files: &[(&str, &str)],
    harness: &str,
    takes: bool,
    then: impl FnOnce(&Temp, &Path, Session<'_>, Value, Value),
) {
    let temp = Temp::new();
    let root = project(&temp, files);
    let connection = connect_overlays(&temp, &[], &[], &real).expect("prepared");
    let client = Client::default();
    let scope = Scope::new(Vec::new());
    let hosted = Hosted::in_process(&client, &scope, &connection);
    let (session, request, told) = open(hosted, &client, harness, &root, takes);
    then(&temp, &root, session, request, told);
}

#[test]
fn the_files_of_a_project_reach_a_session_that_takes_a_system_prompt_in_the_standard_order() {
    run(
        &[
            ("AGENTS.local.md", "mine"),
            ("AGENTS.md", "root"),
            (".agents/AGENTS.md", "private"),
            ("sub/AGENTS.md", "deeper, not where the session is"),
        ],
        CLAUDE,
        true,
        |_, _, mut session, request, told| {
            let (paths, bodies) = files_in(&texts(&request["params"]["systemPrompt"]["content"]));
            assert_eq!(bodies, ["root", "private", "mine"]);
            let ends: Vec<&str> = paths
                .iter()
                .map(|path| path.rsplit("proj/").next().unwrap_or(""))
                .collect();
            assert_eq!(ends, ["AGENTS.md", ".agents/AGENTS.md", "AGENTS.local.md"]);
            // Handed over once: the first prompt carries none of them again.
            let prompt = session.prompt("hello");
            assert!(files_in(&texts(&prompt["params"]["prompt"])).0.is_empty());
            let reported = items(&told);
            let applied: Vec<&Value> = reported
                .iter()
                .filter(|item| {
                    item["key"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("project_context."))
                })
                .collect();
            assert_eq!(applied.len(), 3, "{reported:?}");
            assert!(applied.iter().all(|item| item["state"] == "applied"));
        },
    );
}

#[test]
fn a_session_whose_agent_takes_no_system_prompt_is_given_them_with_its_first_prompt_only() {
    run(
        &[("AGENTS.md", "root"), (".agents/AGENTS.md", "private")],
        CLAUDE,
        false,
        |_, _, mut session, request, told| {
            assert!(request["params"].get("systemPrompt").is_none(), "{request}");
            let first = session.prompt("hello");
            let blocks = texts(&first["params"]["prompt"]);
            assert_eq!(blocks[0], "hello", "the user's own words come first");
            assert_eq!(files_in(&blocks).1, ["root", "private"]);
            let second = session.prompt("again");
            assert!(files_in(&texts(&second["params"]["prompt"])).0.is_empty());
            assert!(
                items(&told).iter().any(|item| item["how"]
                    .as_str()
                    .is_some_and(|how| how.contains("first prompt"))),
                "{told}"
            );
        },
    );
}

#[test]
fn a_harness_that_loads_agents_md_itself_is_not_given_it_twice() {
    run(
        &[("AGENTS.md", "root"), (".agents/AGENTS.md", "private")],
        "coqui-acp-codex",
        true,
        |_, _, _, request, told| {
            let (paths, bodies) = files_in(&texts(&request["params"]["systemPrompt"]["content"]));
            assert_eq!(bodies, ["private"], "{paths:?}");
            let reported = items(&told);
            let left = reported
                .iter()
                .find(|item| item["key"] == "project_context.AGENTS.md")
                .expect("reported");
            assert_eq!(left["state"], "off", "{left}");
        },
    );
}

#[test]
fn a_project_with_no_context_file_gets_nothing_and_no_plugin() {
    run(
        &[
            ("README.md", "not context"),
            ("CLAUDE.md", "not the standard"),
        ],
        CLAUDE,
        true,
        |_, _, session, request, told| {
            assert!(request["params"].get("systemPrompt").is_none(), "{request}");
            assert!(session.plugin.is_none(), "no plugin to follow a directory");
            assert!(
                items(&told).iter().all(|item| !item["key"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("project_context")),
                "{told}"
            );
        },
    );
}

#[test]
fn a_directory_the_session_enters_brings_its_files_with_the_next_prompt_once() {
    run(
        &[
            ("AGENTS.md", "root"),
            ("a/AGENTS.md", "a"),
            ("a/b/.agents/AGENTS.md", "b"),
            ("c/AGENTS.md", "elsewhere"),
        ],
        CLAUDE,
        true,
        |_, root, mut session, request, _| {
            assert_eq!(
                files_in(&texts(&request["params"]["systemPrompt"]["content"])).1,
                ["root"]
            );
            assert!(
                session.plugin.is_some(),
                "the harness's hooks report where it works"
            );
            let deeper = root.join("a").join("b");
            session
                .report("batch", json!({"cwd": deeper.to_string_lossy()}))
                .expect("answered");
            let prompt = session.prompt("next");
            let blocks = texts(&prompt["params"]["prompt"]);
            assert_eq!(blocks[0], "next");
            assert_eq!(
                files_in(&blocks).1,
                ["a", "b"],
                "broad first, root not again"
            );
            session
                .report("batch", json!({"cwd": root.join("a").to_string_lossy()}))
                .expect("answered");
            session
                .report("batch", json!({"cwd": deeper.to_string_lossy()}))
                .expect("answered");
            let again = session.prompt("more");
            assert!(
                files_in(&texts(&again["params"]["prompt"])).0.is_empty(),
                "going back is nothing new"
            );
            // A directory beside, not below, gives only its own.
            session
                .report("batch", json!({"cwd": root.join("c").to_string_lossy()}))
                .expect("answered");
            let beside = session.prompt("beside");
            assert_eq!(
                files_in(&texts(&beside["params"]["prompt"])).1,
                ["elsewhere"]
            );
        },
    );
}
