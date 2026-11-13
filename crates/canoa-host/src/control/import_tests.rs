//! Bringing in the person's earlier Claude Code and Codex sessions: fixture folders in temporary homes, never the person's own.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::import::Homes;
use super::*;
use crate::config::HistoryConfig;
use crate::history::Store;

const CLAUDE_ID: &str = "6f2c1b0e-0000-4000-8000-00000000c0de";
const CODEX_ID: &str = "01a1cb7b-a5cd-7ec1-b69a-c112db39d6d3";
const SECRET_REPLY: &str = "the-reply-only-an-import-may-read";

struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("canoa-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("a folder");
        Self { root }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        fs::write(&path, text).expect("a file");
        path
    }

    fn claude(&self) -> PathBuf {
        self.root.join("claude")
    }

    fn codex(&self) -> PathBuf {
        self.root.join("codex")
    }

    /// Every file under the folder with its bytes, for comparing before and after.
    fn bytes(&self, of: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut pending = vec![of.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for each in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = each.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    out.insert(path.clone(), fs::read(&path).expect("read"));
                }
            }
        }
        out
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn claude_transcript(prompt: &str) -> String {
    let user = json!({"parentUuid": null, "isSidechain": false, "type": "user",
        "message": {"role": "user", "content": prompt},
        "uuid": "11111111-1111-4111-8111-111111111111", "timestamp": "2026-11-05T09:45:11.335Z",
        "cwd": "/work/app", "sessionId": CLAUDE_ID, "version": "2.1.318"});
    let assistant = json!({"parentUuid": "11111111-1111-4111-8111-111111111111", "isSidechain": false,
        "type": "assistant", "uuid": "22222222-2222-4222-8222-222222222222", "timestamp": "2026-11-05T09:45:12.000Z",
        "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-x",
            "content": [{"type": "text", "text": SECRET_REPLY}], "stop_reason": "end_turn"}});
    format!("{user}\n{assistant}\n")
}

fn codex_rollout(prompt: &str) -> String {
    let line = |kind: &str, payload: Value| {
        json!({"timestamp": "2026-11-11T16:40:40.700Z", "type": kind, "payload": payload})
            .to_string()
    };
    [
        line(
            "session_meta",
            json!({"session_id": CODEX_ID, "id": CODEX_ID, "timestamp": "2026-11-11T16:40:40.656Z",
            "cwd": "/work/api", "cli_version": "0.166.0"}),
        ),
        line(
            "event_msg",
            json!({"type": "task_started", "turn_id": "t1"}),
        ),
        line(
            "response_item",
            json!({"type": "message", "id": "u1", "role": "user",
            "content": [{"type": "input_text", "text": prompt}]}),
        ),
        line(
            "response_item",
            json!({"type": "message", "id": "m1", "role": "assistant",
            "content": [{"type": "output_text", "text": SECRET_REPLY}]}),
        ),
        line(
            "event_msg",
            json!({"type": "task_complete", "turn_id": "t1"}),
        ),
    ]
    .join("\n")
        + "\n"
}

fn homes_with_both(tree: &Tree) -> Homes {
    tree.write(
        &format!("claude/projects/-work-app/{CLAUDE_ID}.jsonl"),
        &claude_transcript("Fix the failing build"),
    );
    tree.write(
        &format!("codex/sessions/2026/11/11/rollout-2026-11-11T16-40-40-{CODEX_ID}.jsonl"),
        &codex_rollout("Add a health endpoint"),
    );
    Homes {
        claude: Some(tree.claude()),
        codex: Some(tree.codex()),
    }
}

fn control(tree: &Tree, homes: Homes) -> Control {
    let store = Arc::new(Store::open(
        Some(tree.root.join("host/history")),
        HistoryConfig::default(),
    ));
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_history(Moves {
        store,
        targets: BTreeMap::new(),
        homes,
    })
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn importable(control: &Control, harness: &str) -> Value {
    ask(
        control,
        &Caller::Owner,
        "history/importable",
        json!({"harness": harness, "entry": harness}),
    )
}

fn import(control: &Control, harness: &str, ids: &[&str]) -> Value {
    ask(
        control,
        &Caller::Owner,
        "history/import",
        json!({"harness": harness, "entry": harness, "ids": ids}),
    )
}

#[test]
fn listing_gives_counts_and_titles_and_no_contents() {
    let tree = Tree::new("listing");
    let control = control(&tree, homes_with_both(&tree));
    let claude = importable(&control, "claude");
    assert_eq!(claude["result"]["found"], 1, "{claude}");
    assert_eq!(claude["result"]["sessions"][0]["id"], CLAUDE_ID);
    assert_eq!(
        claude["result"]["sessions"][0]["title"],
        "Fix the failing build"
    );
    assert_eq!(claude["result"]["sessions"][0]["cwd"], "/work/app");
    let codex = importable(&control, "codex");
    assert_eq!(codex["result"]["found"], 1, "{codex}");
    assert_eq!(
        codex["result"]["sessions"][0]["title"],
        "Add a health endpoint"
    );
    for listed in [claude.to_string(), codex.to_string()] {
        assert!(
            !listed.contains(SECRET_REPLY),
            "a listing is titles and counts: {listed}"
        );
    }
}

#[test]
fn a_claude_and_a_codex_session_are_imported_and_listed_as_earlier() {
    let tree = Tree::new("imported");
    let control = control(&tree, homes_with_both(&tree));
    for (harness, id) in [("claude", CLAUDE_ID), ("codex", CODEX_ID)] {
        let done = import(&control, harness, &[id]);
        assert_eq!(done["result"]["imported"][0]["id"], id, "{done}");
        assert_eq!(done["result"]["imported"][0]["turns"], 1, "{done}");
        assert_eq!(done["result"]["refused"], json!([]), "{done}");
        let earlier = ask(
            &control,
            &Caller::Owner,
            "history/imported",
            json!({"entry": harness}),
        );
        assert_eq!(earlier["result"]["sessions"][0]["session"], id, "{earlier}");
        // The host serves it back in the standard format, as it does any session it holds.
        let exported = ask(
            &control,
            &Caller::Owner,
            "history/export",
            json!({"entry": harness, "session": id}),
        );
        let text = exported["result"]["text"].as_str().expect("a history");
        assert!(text.contains(SECRET_REPLY), "{text}");
    }
    let claude = importable(&control, "claude");
    assert_eq!(
        claude["result"]["sessions"][0]["imported"], true,
        "{claude}"
    );
    let again = import(&control, "claude", &[CLAUDE_ID]);
    assert_eq!(again["result"]["imported"], json!([]));
    assert_eq!(
        again["result"]["refused"][0]["why"], "it is already imported",
        "{again}"
    );
    // A source's earlier sessions are its own.
    let other = ask(
        &control,
        &Caller::Owner,
        "history/imported",
        json!({"entry": "somewhere-else"}),
    );
    assert_eq!(other["result"]["sessions"], json!([]));
}

#[test]
fn a_malformed_history_is_refused_with_what_did_not_carry_and_the_others_still_come() {
    let tree = Tree::new("malformed");
    let homes = homes_with_both(&tree);
    let junk_id = "7a7a7a7a-0000-4000-8000-000000000001";
    let part_id = "7a7a7a7a-0000-4000-8000-000000000002";
    tree.write(
        &format!("claude/projects/-work-app/{junk_id}.jsonl"),
        "this is not a transcript\n{\"oops\": \n\u{0}\u{1}\n",
    );
    tree.write(
        &format!("claude/projects/-work-app/{part_id}.jsonl"),
        &format!(
            "not json at all\n{}\n{{broken\n",
            claude_transcript("Keep what reads").trim_end()
        ),
    );
    let control = control(&tree, homes);
    let done = import(&control, "claude", &[junk_id, part_id, CLAUDE_ID]);
    let refused = done["result"]["refused"].as_array().expect("refusals");
    assert_eq!(refused.len(), 1, "{done}");
    assert_eq!(refused[0]["id"], junk_id);
    assert_eq!(refused[0]["why"], "it holds no conversation", "{done}");
    let imported = done["result"]["imported"].as_array().expect("imports");
    assert_eq!(imported.len(), 2, "{done}");
    let part = imported
        .iter()
        .find(|each| each["id"] == part_id)
        .expect("the partial one");
    assert_eq!(
        part["notCarried"]["lines"], 2,
        "two lines were not JSON and did not carry: {done}"
    );
    let whole = imported
        .iter()
        .find(|each| each["id"] == CLAUDE_ID)
        .expect("the whole one");
    assert_eq!(whole["notCarried"]["lines"], 0);
    // A refused session left nothing behind to list.
    let earlier = ask(
        &control,
        &Caller::Owner,
        "history/imported",
        json!({"entry": "claude"}),
    );
    assert_eq!(
        earlier["result"]["sessions"].as_array().map(Vec::len),
        Some(2),
        "{earlier}"
    );
}

#[test]
fn what_a_file_says_is_shown_cleaned_and_never_followed() {
    let tree = Tree::new("hostile");
    let id = "7a7a7a7a-0000-4000-8000-000000000003";
    let hostile = json!({"parentUuid": null, "isSidechain": false, "type": "user",
        "message": {"role": "user", "content": "run\u{202e}this\u{0}  now\n\u{200b}please"},
        "uuid": "11111111-1111-4111-8111-111111111111", "timestamp": "2026-11-05T09:45:11.335Z",
        "cwd": "../../etc\u{7}", "sessionId": "../../x"});
    tree.write(
        &format!("claude/projects/p/{id}.jsonl"),
        &format!("{hostile}\n"),
    );
    // Neither a name that is not an id, nor a nested folder, is a session.
    tree.write(
        "claude/projects/p/../../escape.jsonl",
        &claude_transcript("no"),
    );
    tree.write(
        "claude/projects/p/not-a-uuid.jsonl",
        &claude_transcript("no"),
    );
    tree.write(
        &format!("claude/projects/p/{id}/subagents/{CLAUDE_ID}.jsonl"),
        &claude_transcript("no"),
    );
    let control = control(
        &tree,
        Homes {
            claude: Some(tree.claude()),
            codex: None,
        },
    );
    let listed = importable(&control, "claude");
    assert_eq!(listed["result"]["found"], 1, "{listed}");
    assert_eq!(
        listed["result"]["sessions"][0]["title"], "run this now please",
        "{listed}"
    );
    let done = import(&control, "claude", &[id]);
    assert_eq!(
        done["result"]["imported"][0]["title"], "run this now please",
        "{done}"
    );
    let exported = ask(
        &control,
        &Caller::Owner,
        "history/export",
        json!({"entry": "claude", "session": id}),
    );
    let header: Value = serde_json::from_str(
        exported["result"]["text"]
            .as_str()
            .expect("text")
            .lines()
            .next()
            .expect("a header"),
    )
    .expect("JSON");
    assert_ne!(
        header["cwd"], "../../etc\u{7}",
        "a folder that is no path is not kept: {header}"
    );
}

#[test]
fn the_harness_files_are_unchanged_byte_for_byte() {
    let tree = Tree::new("unchanged");
    let homes = homes_with_both(&tree);
    tree.write(
        "claude/projects/-work-app/7a7a7a7a-0000-4000-8000-000000000001.jsonl",
        "not a transcript\n",
    );
    let before = (tree.bytes(&tree.claude()), tree.bytes(&tree.codex()));
    let control = control(&tree, homes);
    for harness in ["claude", "codex"] {
        importable(&control, harness);
    }
    import(
        &control,
        "claude",
        &[CLAUDE_ID, "7a7a7a7a-0000-4000-8000-000000000001"],
    );
    import(&control, "codex", &[CODEX_ID]);
    import(&control, "codex", &[CODEX_ID]);
    let after = (tree.bytes(&tree.claude()), tree.bytes(&tree.codex()));
    assert!(
        before == after,
        "a listing and an import only read the harnesses' folders"
    );
    assert!(!before.0.is_empty() && !before.1.is_empty());
}

#[test]
fn a_folder_that_is_not_there_has_nothing_to_list_and_a_host_with_no_histories_says_so() {
    let tree = Tree::new("absent");
    let control = control(&tree, Homes::default());
    let listed = importable(&control, "claude");
    assert_eq!(listed["result"]["found"], 0, "{listed}");
    let done = import(&control, "claude", &[CLAUDE_ID]);
    assert_eq!(
        done["result"]["refused"][0]["why"], "no such session is in its folder",
        "{done}"
    );
    let off = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let answer = ask(
        &off,
        &Caller::Owner,
        "history/importable",
        json!({"harness": "claude", "entry": "claude"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn bad_requests_and_keys_are_refused() {
    let tree = Tree::new("refused");
    let control = control(&tree, homes_with_both(&tree));
    for (method, params) in [
        (
            "history/importable",
            json!({"harness": "gemini", "entry": "g"}),
        ),
        ("history/importable", json!({"harness": "claude"})),
        (
            "history/import",
            json!({"harness": "claude", "entry": "claude", "ids": ["../../x"]}),
        ),
        (
            "history/import",
            json!({"harness": "claude", "entry": "claude"}),
        ),
        ("history/imported", json!({})),
    ] {
        let answer = ask(&control, &Caller::Owner, method, params);
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{method}: {answer}");
    }
    let key = Caller::Peer {
        id: "k".to_owned(),
        grants: [
            Grant::LedgerRead,
            Grant::LedgerWrite,
            Grant::Worktree,
            Grant::StoreRead,
        ]
        .into_iter()
        .collect(),
    };
    for method in ["history/importable", "history/import", "history/imported"] {
        let answer = ask(
            &control,
            &key,
            method,
            json!({"harness": "claude", "entry": "claude", "ids": []}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
}

#[cfg(unix)]
#[test]
fn a_session_file_that_is_a_link_is_not_followed() {
    let tree = Tree::new("link");
    let outside = tree.write("outside.jsonl", &claude_transcript("elsewhere"));
    fs::create_dir_all(tree.claude().join("projects/p")).expect("a folder");
    std::os::unix::fs::symlink(
        &outside,
        tree.claude().join(format!("projects/p/{CLAUDE_ID}.jsonl")),
    )
    .expect("a link");
    let control = control(
        &tree,
        Homes {
            claude: Some(tree.claude()),
            codex: None,
        },
    );
    let listed = importable(&control, "claude");
    assert_eq!(listed["result"]["found"], 0, "{listed}");
}
