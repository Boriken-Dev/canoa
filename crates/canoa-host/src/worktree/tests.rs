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
fn only_the_four_methods_are_worktree_requests() {
    for method in [NEW, STATUS, REMOVE, DIFF] {
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
fn a_device_or_a_share_outside_the_roots_is_never_looked_at() {
    let worktrees = Worktrees::new(
        vec![PathBuf::from(r"C:\work"), PathBuf::from(r"\\server\share")],
        true,
    );
    for path in [
        r"\\.\pipe\x",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x",
        r"\\other\share\repo",
        r"\\server\other\repo",
        r"\\server\share\..\other",
    ] {
        let reply = ask(&worktrees, STATUS, &json!({"path": path}));
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{path}: {reply}");
        let reply = ask(&worktrees, NEW, &json!({"cwd": path, "branch": "canoa/x"}));
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{path}: {reply}");
    }
}

/// A repository on a share under a root passes this host's checks like any other. Git itself still asks for
/// `safe.directory` on a share it does not take as local, and says so; where that is set, a worktree is made there and
/// every path answered is written plain.
#[cfg(windows)]
#[test]
fn a_repository_on_a_share_under_a_root_passes_the_hosts_checks() {
    let Some(fixture) = Fixture::new("on-share") else {
        return;
    };
    let on_share = |path: &Path| {
        let text = path.to_string_lossy();
        let (drive, rest) = text.split_at(2);
        PathBuf::from(format!(r"\\localhost\{}${rest}", &drive[..1]))
    };
    let repo = on_share(&fixture.repo);
    let worktrees = Worktrees::new(vec![on_share(&fixture.scratch.join("src"))], true);
    let made = match worktrees.make(&repo.to_string_lossy(), "canoa/share-a1b2c3") {
        Ok(made) => made,
        Err(refusal) => {
            assert_eq!(refusal.code, INTERNAL_ERROR, "{refusal:?}");
            assert!(refusal.message.starts_with("git failed: "), "{refusal:?}");
            return;
        }
    };
    let path = PathBuf::from(made["path"].as_str().expect("a path"));
    assert!(
        path.to_string_lossy().starts_with(r"\\localhost\"),
        "the worktree is written plain, on the share: {}",
        path.display()
    );
    assert!(path.join("README").exists());
    let removed = worktrees
        .remove(&path.to_string_lossy(), false)
        .expect("removed");
    assert_eq!(removed["branchKept"], false, "{removed}");
    assert!(!path.exists());
}

/// `git -C dir args` for a test, which must succeed.
fn run_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Canoa Test",
            "-c",
            "user.email=test@canoa.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_commit_made_after_the_count_keeps_its_branch() {
    if Command::new("git").arg("--version").output().is_err() {
        return eprintln!("skipped: no git on PATH");
    }
    let scratch = std::env::temp_dir().join(format!("canoa-worktree-race-{}", std::process::id()));
    let root = scratch.join("src");
    let repo = root.join("proj");
    std::fs::create_dir_all(&repo).expect("a directory");
    run_git(&repo, &["init", "-q"]);
    run_git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let worktrees = Worktrees::new(vec![root.clone()], true);
    let made = worktrees
        .make(&repo.to_string_lossy(), "canoa/race-a1b2c3")
        .expect("made");
    let path = PathBuf::from(made["path"].as_str().expect("a path"));
    let removed = worktrees
        .remove_after(&path.to_string_lossy(), false, || {
            run_git(&path, &["commit", "-q", "--allow-empty", "-m", "late"]);
        })
        .expect("removed");
    assert_eq!(removed["branchKept"], true, "{removed}");
    assert!(run_git(&repo, &["branch", "--format=%(refname:short)"]).contains("canoa/race-a1b2c3"));
    let _ = std::fs::remove_dir_all(&scratch);
}

#[cfg(windows)]
#[test]
fn a_git_file_naming_a_share_is_refused_before_git_runs() {
    let scratch = std::env::temp_dir().join(format!("canoa-worktree-share-{}", std::process::id()));
    let repo = scratch.join("src").join("proj");
    std::fs::create_dir_all(&repo).expect("a directory");
    std::fs::write(repo.join(".git"), "gitdir: //server.invalid/share/x\n").expect("a .git file");
    let worktrees = Worktrees::new(vec![scratch.join("src")], true);
    let reply = ask(&worktrees, NEW, &json!({"cwd": repo, "branch": "canoa/x"}));
    assert!(
        reply["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("does not reach")),
        "{reply}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

/// A repository `src/proj` with one commit under a scratch directory, and the worktree rule over `src`.
struct Fixture {
    scratch: PathBuf,
    repo: PathBuf,
    worktrees: Worktrees,
}

impl Fixture {
    fn new(name: &str) -> Option<Self> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipped: no git on PATH");
            return None;
        }
        let scratch =
            std::env::temp_dir().join(format!("canoa-worktree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let repo = scratch.join("src").join("proj");
        std::fs::create_dir_all(repo.join("sub")).expect("a directory");
        run_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README"), "hello\n").expect("a file");
        std::fs::write(repo.join("sub").join("keep"), "").expect("a file");
        run_git(&repo, &["add", "."]);
        run_git(&repo, &["commit", "-q", "-m", "first"]);
        let worktrees = Worktrees::new(vec![scratch.join("src")], true);
        Some(Self {
            scratch,
            repo,
            worktrees,
        })
    }

    fn make(&self, cwd: &Path, branch: &str) -> Result<PathBuf, Refusal> {
        let made = self.worktrees.make(&cwd.to_string_lossy(), branch)?;
        Ok(PathBuf::from(made["path"].as_str().expect("a path")))
    }

    fn status(&self, path: &Path) -> Result<Value, Refusal> {
        self.worktrees.status(&path.to_string_lossy())
    }

    fn remove(&self, path: &Path, discard: bool) -> Result<Value, Refusal> {
        self.worktrees.remove(&path.to_string_lossy(), discard)
    }

    fn branches(&self) -> String {
        run_git(&self.repo, &["branch", "--format=%(refname:short)"])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

#[test]
fn two_sessions_on_one_repository_each_work_in_a_worktree_of_their_own() {
    let Some(fixture) = Fixture::new("two") else {
        return;
    };
    let first = fixture
        .make(&fixture.repo, "canoa/first-a1b2c3")
        .expect("first");
    let second = fixture
        .make(&fixture.repo.join("sub"), "canoa/second-d4e5f6")
        .expect("second");
    assert_ne!(first, second);
    assert!(first.ends_with("first-a1b2c3") && second.ends_with("second-d4e5f6"));
    assert!(first.join("README").exists() && second.join("README").exists());
    let list = run_git(&fixture.repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(list.matches("worktree ").count(), 3, "{list}");

    std::fs::write(first.join("draft"), "unsaved\n").expect("a file");
    std::fs::write(second.join("done"), "saved\n").expect("a file");
    run_git(&second, &["add", "done"]);
    run_git(&second, &["commit", "-q", "-m", "work"]);
    let state = fixture.status(&first).expect("status");
    assert_eq!(state["exists"], true, "{state}");
    assert_eq!(
        (&state["changed"], &state["commits"]),
        (&json!(1), &json!(0))
    );
    let state = fixture.status(&second).expect("status");
    assert_eq!(
        (&state["changed"], &state["commits"]),
        (&json!(0), &json!(1))
    );
    assert!(!second.join("draft").exists());

    // A change is not deleted without discard; a commit of its own keeps the branch.
    assert!(fixture.remove(&first, false).is_err());
    assert!(first.join("draft").exists());
    let discarded = fixture.remove(&first, true).expect("discarded");
    assert_eq!(discarded["branchKept"], false, "{discarded}");
    assert!(!first.exists());
    let removed = fixture.remove(&second, false).expect("removed");
    assert_eq!(removed["branchKept"], true, "{removed}");
    assert_eq!(removed["commits"], 1, "{removed}");
    let branches = fixture.branches();
    assert!(
        !branches.contains("canoa/first-a1b2c3") && branches.contains("canoa/second-d4e5f6"),
        "{branches}"
    );
    assert_eq!(fixture.status(&first).expect("status")["exists"], false);
}

#[test]
fn what_is_not_canoas_is_neither_made_over_nor_removed() {
    let Some(fixture) = Fixture::new("notmine") else {
        return;
    };
    assert!(fixture.make(&fixture.repo, "--orphan").is_err());
    assert!(
        fixture
            .make(&fixture.scratch.join("src").join("none"), "canoa/x")
            .is_err()
    );
    let plain = fixture.scratch.join("src").join("plain");
    std::fs::create_dir_all(&plain).expect("a directory");
    assert!(fixture.make(&plain, "canoa/x").is_err());
    run_git(
        &fixture.repo,
        &["worktree", "add", "-q", "-b", "mine", "../mine"],
    );
    run_git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "canoa/hand-made",
            "../hand-made",
        ],
    );
    for path in [
        fixture.repo.clone(),
        fixture.scratch.join("src").join("mine"),
        fixture.scratch.join("src").join("hand-made"),
    ] {
        assert!(
            fixture
                .status(&path)
                .map_or(true, |state| state["exists"] == false),
            "{path:?}"
        );
        assert!(fixture.remove(&path, true).is_err(), "{path:?}");
        assert!(path.exists(), "{path:?}");
    }
}

#[test]
fn a_worktrees_directory_that_is_a_link_is_not_used() {
    let Some(fixture) = Fixture::new("linked") else {
        return;
    };
    let elsewhere = fixture.scratch.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory");
    let link = fixture.scratch.join("src").join("proj.worktrees");
    if cfg!(windows) {
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&elsewhere)
            .output()
            .expect("cmd runs");
        assert!(made.status.success());
    } else {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, &link).expect("a link");
    }
    assert!(fixture.make(&fixture.repo, "canoa/x").is_err());
    assert_eq!(std::fs::read_dir(&elsewhere).expect("listed").count(), 0);
    assert!(!fixture.branches().contains("canoa/x"));
    if cfg!(windows) {
        let _ = std::fs::remove_dir(&link);
    }
}

#[test]
fn files_git_ignores_are_counted_and_only_discard_removes_them() {
    let Some(fixture) = Fixture::new("ignored") else {
        return;
    };
    std::fs::write(fixture.repo.join(".gitignore"), ".env\nbuild/\n").expect("a file");
    run_git(&fixture.repo, &["add", ".gitignore"]);
    run_git(&fixture.repo, &["commit", "-q", "-m", "ignore"]);
    let made = fixture
        .make(&fixture.repo, "canoa/ignored-a1b2c3")
        .expect("made");
    std::fs::write(made.join(".env"), "TOKEN=x\n").expect("a file");
    std::fs::create_dir_all(made.join("build")).expect("a directory");
    std::fs::write(made.join("build").join("out"), "").expect("a file");
    assert_eq!(fixture.status(&made).expect("status")["changed"], 2);
    assert!(fixture.remove(&made, false).is_err());
    assert!(made.join(".env").exists());
    fixture.remove(&made, true).expect("discarded");
    assert!(!made.exists());
}

#[test]
fn a_repository_configured_with_a_filter_or_an_include_is_refused_and_no_filter_runs() {
    let Some(fixture) = Fixture::new("filter") else {
        return;
    };
    std::fs::write(fixture.repo.join("a.txt"), "text\n").expect("a file");
    run_git(&fixture.repo, &["add", "a.txt"]);
    run_git(&fixture.repo, &["commit", "-q", "-m", "text"]);
    let made = fixture
        .make(&fixture.repo, "canoa/before-a1b2c3")
        .expect("made");
    let marker = fixture.scratch.join("filter-ran");
    std::fs::write(
        fixture.repo.join(".git").join("info").join("attributes"),
        "*.txt filter=x\n",
    )
    .expect("attributes");
    let command = format!(
        "sh -c 'echo ran > \"{}\"; cat'",
        marker.to_string_lossy().replace('\\', "/")
    );
    for key in ["filter.x.smudge", "filter.x.clean"] {
        run_git(&fixture.repo, &["config", key, &command]);
    }
    assert!(fixture.make(&fixture.repo, "canoa/after-d4e5f6").is_err());
    std::fs::write(made.join("a.txt"), "changed\n").expect("a file");
    assert!(fixture.status(&made).is_err());
    assert!(fixture.remove(&made, true).is_err());
    assert!(!marker.exists(), "a filter ran");
    run_git(&fixture.repo, &["config", "--remove-section", "filter.x"]);
    run_git(&fixture.repo, &["config", "include.path", "elsewhere.cfg"]);
    assert!(fixture.make(&fixture.repo, "canoa/after-d4e5f6").is_err());
}

#[test]
fn a_submodule_gets_no_worktree_inside_its_superprojects_git() {
    let Some(fixture) = Fixture::new("submodule") else {
        return;
    };
    let library = fixture.scratch.join("src").join("library");
    std::fs::create_dir_all(&library).expect("a directory");
    run_git(&library, &["init", "-q"]);
    run_git(&library, &["commit", "-q", "--allow-empty", "-m", "first"]);
    run_git(
        &fixture.repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "../library",
            "lib",
        ],
    );
    run_git(&fixture.repo, &["commit", "-q", "-m", "submodule"]);
    assert!(
        fixture
            .make(&fixture.repo.join("lib"), "canoa/sub-a1b2c3")
            .is_err()
    );
    assert!(
        !fixture
            .repo
            .join(".git")
            .join("modules")
            .join("lib.worktrees")
            .exists()
    );
}

#[test]
fn a_detached_head_with_commits_of_its_own_is_still_found_and_not_removed() {
    let Some(fixture) = Fixture::new("detached") else {
        return;
    };
    let made = fixture
        .make(&fixture.repo, "canoa/detached-a1b2c3")
        .expect("made");
    run_git(&made, &["checkout", "-q", "--detach"]);
    std::fs::write(made.join("work"), "x\n").expect("a file");
    run_git(&made, &["add", "work"]);
    run_git(&made, &["commit", "-q", "-m", "detached work"]);
    let state = fixture.status(&made).expect("status");
    assert_eq!(state["exists"], true, "{state}");
    assert_eq!(state["detached"], true, "{state}");
    assert_eq!(state["commits"], 1, "{state}");
    for discard in [false, true] {
        assert!(fixture.remove(&made, discard).is_err());
    }
    assert!(made.join("work").exists());
}

#[test]
fn a_status_tells_one_set_of_changes_from_another_of_the_same_size() {
    let Some(fixture) = Fixture::new("listing") else {
        return;
    };
    let made = fixture
        .make(&fixture.repo, "canoa/listing-a1b2c3")
        .expect("made");
    std::fs::write(made.join("one"), "").expect("a file");
    let first = fixture.status(&made).expect("status");
    std::fs::remove_file(made.join("one")).expect("removed");
    std::fs::write(made.join("two"), "").expect("a file");
    let second = fixture.status(&made).expect("status");
    assert_eq!(first["changed"], second["changed"]);
    assert_ne!(first["listing"], second["listing"]);
}

#[test]
fn a_diff_lists_tracked_and_untracked_files_against_a_commit_and_writes_nothing() {
    let Some(fixture) = Fixture::new("diff") else {
        return;
    };
    let repo = &fixture.repo;
    std::fs::write(repo.join("README"), "hello\nworld\n").expect("a file");
    std::fs::write(repo.join("fresh.txt"), "one\ntwo\n").expect("a file");
    std::fs::write(repo.join("blob.bin"), [0u8, 1, 2]).expect("a file");
    std::fs::remove_file(repo.join("sub").join("keep")).expect("removed");
    let before = run_git(repo, &["status", "--porcelain=v1", "--untracked-files=all"]);
    let asked = fixture
        .worktrees
        .diff(&repo.to_string_lossy(), None)
        .expect("a diff");
    assert_eq!(asked["exists"], true, "{asked}");
    let head = run_git(repo, &["rev-parse", "HEAD"]);
    assert_eq!(asked["head"], head.trim(), "{asked}");
    let files = asked["files"].as_array().expect("files");
    let by = |name: &str| {
        files
            .iter()
            .find(|file| file["path"] == name)
            .unwrap_or_else(|| panic!("{name} in {asked}"))
    };
    assert_eq!(
        (
            by("README")["added"].as_u64(),
            by("README")["removed"].as_u64()
        ),
        (Some(1), Some(0))
    );
    assert!(
        by("README")["patch"]
            .as_str()
            .expect("a patch")
            .starts_with("@@ "),
        "{asked}"
    );
    assert_eq!(by("sub/keep")["status"], "D");
    assert_eq!(by("fresh.txt")["status"], "A");
    assert_eq!(by("fresh.txt")["added"], 2);
    assert_eq!(by("blob.bin")["binary"], true);
    assert!(by("blob.bin")["patch"].is_null());
    assert_eq!(asked["added"], 3, "{asked}");
    // A folder limits it, with paths relative to it.
    let inner = fixture
        .worktrees
        .diff(&repo.join("sub").to_string_lossy(), None)
        .expect("a diff");
    assert_eq!(
        inner["files"].as_array().expect("files").len(),
        1,
        "{inner}"
    );
    assert_eq!(inner["files"][0]["path"], "keep");
    // Against an earlier commit, what a later commit made counts too.
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-q", "-m", "second"]);
    let since = fixture
        .worktrees
        .diff(&repo.to_string_lossy(), Some(head.trim()))
        .expect("a diff");
    assert_eq!(since["added"], 3, "{since}");
    let now = fixture
        .worktrees
        .diff(&repo.to_string_lossy(), None)
        .expect("a diff");
    assert_eq!(now["files"].as_array().expect("files").len(), 0, "{now}");
    // The same change has the same print, another has another.
    assert_ne!(
        since["files"][0]["print"], since["files"][1]["print"],
        "{since}"
    );
    let _ = before;
    for bad in ["nothex", "abc", &"0".repeat(40)] {
        let reply = fixture.worktrees.diff(&repo.to_string_lossy(), Some(bad));
        assert!(reply.is_err(), "{bad}");
    }
    assert_eq!(
        fixture
            .worktrees
            .diff(&repo.join("nothing-here").to_string_lossy(), None)
            .expect("answered")["exists"],
        false
    );
}

#[test]
fn a_diff_changes_nothing_in_the_repository() {
    let Some(fixture) = Fixture::new("diffreads") else {
        return;
    };
    std::fs::write(fixture.repo.join("README"), "changed\n").expect("a file");
    std::fs::write(fixture.repo.join("new.txt"), "x\n").expect("a file");
    let status = || {
        run_git(
            &fixture.repo,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )
    };
    let index = || std::fs::read(fixture.repo.join(".git").join("index")).expect("an index");
    let (state, before) = (status(), index());
    fixture
        .worktrees
        .diff(&fixture.repo.to_string_lossy(), None)
        .expect("a diff");
    assert_eq!(status(), state);
    assert_eq!(index(), before, "the index is not rewritten");
}
