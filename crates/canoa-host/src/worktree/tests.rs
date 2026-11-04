use super::*;

fn ask(worktrees: &Worktrees, method: &str, params: &Value) -> Value {
    let message = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
    let reply = worktrees
        .answer(message.to_string().as_bytes())
        .expect("a request is answered");
    let reply: Value = serde_json::from_slice(&reply).expect("JSON");
    assert_eq!(reply["id"], 7, "{reply}");
    reply
}

fn root() -> PathBuf {
    std::env::temp_dir()
}

#[test]
fn a_branch_is_canoa_then_plain_words() {
    for good in [
        "canoa/fix-tests-a1b2c3",
        "canoa/x",
        "canoa/0",
        "canoa/a-b-c",
    ] {
        assert!(safe_branch(good), "{good}");
    }
    let long = format!("canoa/{}", "a".repeat(MAX_BRANCH));
    for bad in [
        "",
        "canoa/",
        "main",
        "-b",
        "canoa/-x",
        "canoa/x-",
        "canoa/a--b",
        "canoa/a..b",
        "canoa/../x",
        "canoa/a/b",
        "canoa/A",
        "canoa/a b",
        "canoa/a\u{7}",
        "canoa/a\nb",
        "canoa/a@{1}",
        "canoa/a.lock",
        "canoa/é",
        "--canoa/x",
        long.as_str(),
    ] {
        assert!(!safe_branch(bad), "{bad:?}");
    }
}

#[test]
fn only_the_three_methods_are_worktree_requests() {
    for method in [NEW, STATUS, REMOVE] {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}});
        assert!(Worktrees::is_request(message.to_string().as_bytes()));
    }
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "session/new", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "_canoa/worktree/other", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 1, "result": {"method": NEW}}),
    ] {
        assert!(
            !Worktrees::is_request(message.to_string().as_bytes()),
            "{message}"
        );
    }
    assert!(!Worktrees::is_request(b"not json"));
}

#[test]
fn a_notification_is_dropped_unanswered() {
    let worktrees = Worktrees::new(vec![root()], true);
    let message =
        json!({"jsonrpc": "2.0", "method": NEW, "params": {"cwd": "/x", "branch": "canoa/x"}});
    assert_eq!(worktrees.answer(message.to_string().as_bytes()), None);
}

#[test]
fn switched_off_or_with_no_root_nothing_is_made() {
    let params = json!({"cwd": root(), "branch": "canoa/x"});
    let off = ask(&Worktrees::new(vec![root()], false), NEW, &params);
    assert_eq!(off["error"]["code"], INVALID_PARAMS, "{off}");
    assert!(
        off["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("worktrees = false")),
        "{off}"
    );
    let rootless = ask(&Worktrees::new(Vec::new(), true), NEW, &params);
    assert!(
        rootless["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("working roots")),
        "{rootless}"
    );
}

#[test]
fn what_a_client_names_is_checked_before_git_runs() {
    let inside = root().join("canoa-worktree-unit");
    let worktrees = Worktrees::new(vec![inside.clone()], true);
    let outside = root().to_string_lossy().into_owned();
    let dotted = format!(
        "{}{}..{}x",
        inside.display(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    for params in [
        json!({"cwd": outside, "branch": "canoa/x"}),
        json!({"cwd": dotted, "branch": "canoa/x"}),
        json!({"cwd": "relative", "branch": "canoa/x"}),
        json!({"cwd": inside, "branch": "-x"}),
        json!({"cwd": inside, "branch": "canoa/a..b"}),
        json!({"cwd": inside}),
        json!({"cwd": 3, "branch": "canoa/x"}),
    ] {
        let reply = ask(&worktrees, NEW, &params);
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{params}: {reply}");
    }
    for method in [STATUS, REMOVE] {
        let reply = ask(&worktrees, method, &json!({"path": outside}));
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{reply}");
    }
    let reply = ask(
        &worktrees,
        REMOVE,
        &json!({"path": inside, "discard": "yes"}),
    );
    assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{reply}");
    let message = json!({"jsonrpc": "2.0", "id": 7, "method": NEW, "params": ["/x", "canoa/x"]});
    let reply: Value = serde_json::from_slice(
        &worktrees
            .answer(message.to_string().as_bytes())
            .expect("answered"),
    )
    .expect("JSON");
    assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{reply}");
}

#[cfg(windows)]
#[test]
fn a_share_is_never_looked_at() {
    let worktrees = Worktrees::new(vec![PathBuf::from(r"\\server\share")], true);
    for path in [
        r"\\server\share\repo",
        r"\\?\UNC\server\share\repo",
        r"\\.\pipe\x",
    ] {
        let reply = ask(&worktrees, STATUS, &json!({"path": path}));
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{path}: {reply}");
    }
}
