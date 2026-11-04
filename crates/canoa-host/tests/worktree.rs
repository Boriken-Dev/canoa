//! A git worktree per session, made by the built `canoa-host` inside an entry's working roots, against repositories in a
//! temporary directory. Skipped, saying so, where no `git` runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use support::{Host, Scratch, example, quoted, with_link};

fn has_git() -> bool {
    let found = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipped: no git on PATH");
    }
    found
}

/// `git -C dir args`, which must succeed; its output.
fn git(dir: &Path, args: &[&str]) -> String {
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

/// A repository at `dir` with one commit.
fn repository(dir: &Path) {
    std::fs::create_dir_all(dir.join("sub")).expect("a directory");
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("README"), "hello\n").expect("a file");
    std::fs::write(dir.join("sub").join("keep"), "").expect("a file");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
}

fn entry(name: &str, roots: &[&Path], extra: &str) -> String {
    let roots: Vec<String> = roots
        .iter()
        .map(|root| quoted(&root.to_string_lossy()))
        .collect();
    format!(
        "[host.agents.{name}]\nprogram = {}\nworking_roots = [{}]\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy()),
        roots.join(", ")
    )
}

/// One worktree request on a link of its own, before any agent starts: `Ok(result)` or `Err(error)`.
fn ask(host: &Host, entry: &str, method: &str, params: Value) -> Result<Value, Value> {
    let client = support::key();
    host.pair(&client);
    with_link(host.url(entry), &client, host.peer, |rpc| {
        rpc.request(method, params)
    })
    .expect("a paired key is served")
}

fn message(error: &Value) -> String {
    error["message"].as_str().unwrap_or_default().to_owned()
}

/// What `canonicalize` makes of `path`, written as a drive path on Windows.
fn resolved(path: &Path) -> PathBuf {
    let path = std::fs::canonicalize(path).expect("it exists");
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(drive) => PathBuf::from(drive),
        None => path,
    }
}

#[test]
fn two_sessions_on_one_repository_each_work_in_a_worktree_of_their_own() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    let host = Host::start(&entry("fixture", &[&root], ""), 1);
    let client = support::key();
    host.pair(&client);

    let mut made = Vec::new();
    for (branch, cwd) in [
        ("canoa/first-a1b2c3", repo.clone()),
        ("canoa/second-d4e5f6", repo.join("sub")),
    ] {
        let opened = with_link(host.url("fixture"), &client, host.peer, |rpc| {
            let worktree = rpc
                .request("_canoa/worktree/new", json!({"cwd": cwd, "branch": branch}))
                .expect("a worktree is made");
            let path = worktree["path"].as_str().expect("a path").to_owned();
            rpc.initialize().expect("initialize is answered");
            let session = rpc
                .request("session/new", json!({"cwd": path, "mcpServers": []}))
                .expect("a session opens in the worktree");
            let late = rpc
                .request("_canoa/worktree/status", json!({"path": path}))
                .expect_err("once the agent runs, a worktree request is refused");
            assert_eq!(late["code"], -32600, "{late}");
            (worktree, session)
        })
        .expect("a paired key is served");
        made.push(opened);
    }
    let [(first, first_session), (second, second_session)] = <[_; 2]>::try_from(made).expect("two");
    let holder = resolved(&root).join("proj.worktrees");
    assert_eq!(
        first["path"],
        holder.join("first-a1b2c3").to_string_lossy().as_ref(),
        "{first}"
    );
    assert_eq!(
        second["path"],
        holder.join("second-d4e5f6").to_string_lossy().as_ref(),
        "{second}"
    );
    assert_eq!(first["repo"], resolved(&repo).to_string_lossy().as_ref());
    assert_eq!(first_session["received"]["cwd"], first["path"]);
    assert_eq!(second_session["received"]["cwd"], second["path"]);
    let branches = git(&repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        branches.contains("canoa/first-a1b2c3") && branches.contains("canoa/second-d4e5f6"),
        "{branches}"
    );
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(listed.matches("worktree ").count(), 3, "{listed}");

    // One holds a change and the other a commit: neither goes without the person's word.
    let first_path = PathBuf::from(first["path"].as_str().expect("a path"));
    let second_path = PathBuf::from(second["path"].as_str().expect("a path"));
    std::fs::write(first_path.join("draft"), "unsaved\n").expect("a file");
    std::fs::write(second_path.join("done"), "saved\n").expect("a file");
    git(&second_path, &["add", "done"]);
    git(&second_path, &["commit", "-q", "-m", "work"]);

    let status = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": first_path}),
    )
    .expect("a status");
    let picked = |status: &Value| {
        (
            status["exists"].clone(),
            status["branch"].clone(),
            status["detached"].clone(),
            status["changed"].clone(),
            status["commits"].clone(),
        )
    };
    assert_eq!(
        picked(&status),
        (
            json!(true),
            json!("canoa/first-a1b2c3"),
            json!(false),
            json!(1),
            json!(0)
        ),
        "{status}"
    );
    let status = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": second_path}),
    )
    .expect("a status");
    assert_eq!(
        picked(&status),
        (
            json!(true),
            json!("canoa/second-d4e5f6"),
            json!(false),
            json!(0),
            json!(1)
        ),
        "{status}"
    );

    let kept = ask(
        &host,
        "fixture",
        "_canoa/worktree/remove",
        json!({"path": first_path}),
    )
    .expect_err("a worktree holding a change is not removed without discard");
    assert_eq!(kept["code"], -32602, "{kept}");
    assert!(first_path.join("draft").exists());
    let removed = ask(
        &host,
        "fixture",
        "_canoa/worktree/remove",
        json!({"path": first_path, "discard": true}),
    )
    .expect("removed when the person discards");
    assert_eq!(
        removed,
        json!({"removed": true, "branch": "canoa/first-a1b2c3", "branchKept": false, "commits": 0})
    );
    assert!(!first_path.exists());

    let removed = ask(
        &host,
        "fixture",
        "_canoa/worktree/remove",
        json!({"path": second_path}),
    )
    .expect("a clean worktree is removed");
    assert_eq!(
        removed,
        json!({"removed": true, "branch": "canoa/second-d4e5f6", "branchKept": true, "commits": 1})
    );
    assert!(!second_path.exists());
    let branches = git(&repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.contains("canoa/first-a1b2c3"),
        "a branch with nothing of its own goes: {branches}"
    );
    assert!(
        branches.contains("canoa/second-d4e5f6"),
        "a branch with a commit of its own stays: {branches}"
    );

    let gone = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": first_path}),
    )
    .expect("a status");
    assert_eq!(gone, json!({"exists": false}));
}

#[test]
fn nothing_is_made_or_removed_outside_the_working_roots() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    let outside = scratch.0.join("elsewhere");
    repository(&repo);
    repository(&outside);
    // An entry whose root is the repository itself has nowhere beside it to put a worktree.
    let config = entry("fixture", &[&root], "")
        + &entry("off", &[&root], "worktrees = false")
        + &entry("narrow", &[&repo], "");
    let host = Host::start(&config, 3);
    let new = |entry: &str, cwd: &Path, branch: &str| {
        ask(
            &host,
            entry,
            "_canoa/worktree/new",
            json!({"cwd": cwd, "branch": branch}),
        )
    };

    let refused = new("fixture", &outside, "canoa/x").expect_err("a repository outside the roots");
    assert_eq!(refused["code"], -32602, "{refused}");
    let refused = new("off", &repo, "canoa/x").expect_err("switched off");
    assert!(message(&refused).contains("worktrees = false"), "{refused}");
    let refused = new("narrow", &repo, "canoa/x").expect_err("nowhere beside the repository");
    assert!(
        message(&refused).contains("outside the working roots"),
        "{refused}"
    );
    for branch in [
        "-x",
        "--orphan",
        "canoa/a..b",
        "main",
        "canoa/x y",
        "canoa/x\u{1b}",
    ] {
        let refused = new("fixture", &repo, branch).expect_err("not a branch the host makes");
        assert_eq!(refused["code"], -32602, "{branch:?}: {refused}");
    }
    assert!(!root.join("proj.worktrees").exists(), "nothing was made");

    // Only a worktree the host would have made, where it would have put it, is the host's to remove.
    git(&repo, &["worktree", "add", "-q", "-b", "mine", "../mine"]);
    git(
        &repo,
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
        repo.clone(),
        root.join("mine"),
        root.join("hand-made"),
        outside.clone(),
    ] {
        let refused = ask(
            &host,
            "fixture",
            "_canoa/worktree/remove",
            json!({"path": path, "discard": true}),
        )
        .expect_err("not the host's to remove");
        assert_eq!(refused["code"], -32602, "{}: {refused}", path.display());
    }
    assert!(
        repo.join("README").exists()
            && root.join("mine").exists()
            && root.join("hand-made").exists()
    );
}

/// Makes `link` lead to the directory `target`: a symbolic link on Unix, a junction on Windows, which needs no privilege.
fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("a link");
    #[cfg(windows)]
    {
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("cmd runs");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
    }
}

#[test]
fn a_link_never_leads_a_worktree_out_of_the_roots() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let outside = scratch.0.join("elsewhere");
    let repo = root.join("proj");
    repository(&repo);
    repository(&outside.join("theirs"));
    let host = Host::start(&entry("fixture", &[&root], ""), 1);

    // A directory inside the root that leads to a repository outside it.
    link_dir(&outside.join("theirs"), &root.join("theirs"));
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": root.join("theirs"), "branch": "canoa/x"}),
    )
    .expect_err("the link leads out");
    assert!(message(&refused).contains("link"), "{refused}");

    // The directory worktrees go in, leading out.
    std::fs::create_dir_all(outside.join("holder")).expect("a directory");
    link_dir(&outside.join("holder"), &root.join("proj.worktrees"));
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": "canoa/x"}),
    )
    .expect_err("the .worktrees directory leads out");
    assert!(message(&refused).contains("link"), "{refused}");
    assert_eq!(
        std::fs::read_dir(outside.join("holder"))
            .expect("readable")
            .count(),
        0,
        "nothing was made there"
    );
    let branches = git(&repo, &["branch", "--format=%(refname:short)"]);
    assert!(!branches.contains("canoa/x"), "{branches}");
}

#[test]
fn hooks_in_the_repository_do_not_run() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    let marker = scratch.0.join("hook-ran");
    let hook = format!(
        "#!/bin/sh\necho ran > '{}'\n",
        marker.to_string_lossy().replace('\\', "/")
    );
    let hook_path = repo.join(".git").join("hooks").join("post-checkout");
    std::fs::write(&hook_path, hook).expect("a hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
    }
    let host = Host::start(&entry("fixture", &[&root], ""), 1);
    let made = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": "canoa/hooked"}),
    )
    .expect("a worktree is made");
    assert!(
        PathBuf::from(made["path"].as_str().expect("a path"))
            .join("README")
            .exists()
    );
    assert!(!marker.exists(), "the repository's post-checkout hook ran");
    // The same hook runs for git itself, so its absence above is the host's doing.
    git(&repo, &["worktree", "add", "-q", "-b", "plain", "../plain"]);
    assert!(marker.exists(), "the hook runs for a plain git");
}

/// A host serving `root`, and a worktree it made of `repo` on `branch`.
fn host_with_worktree(root: &Path, repo: &Path, branch: &str) -> (Host, PathBuf) {
    let host = Host::start(&entry("fixture", &[root], ""), 1);
    let made = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": branch}),
    )
    .expect("a worktree is made");
    let path = PathBuf::from(made["path"].as_str().expect("a path"));
    (host, path)
}

#[test]
fn files_git_ignores_are_counted_and_never_removed_unasked() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    std::fs::write(repo.join(".gitignore"), ".env\nbuild/\n").expect("a file");
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-q", "-m", "ignore"]);
    let (host, path) = host_with_worktree(&root, &repo, "canoa/ignored-a1b2c3");
    std::fs::write(path.join(".env"), "TOKEN=x\n").expect("a file");
    std::fs::create_dir_all(path.join("build")).expect("a directory");
    std::fs::write(path.join("build").join("out"), "").expect("a file");
    let status = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": path}),
    )
    .expect("a status");
    assert_eq!(status["changed"], 2, "{status}");
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/remove",
        json!({"path": path}),
    )
    .expect_err("files git ignores are not removed without discard");
    assert_eq!(refused["code"], -32602, "{refused}");
    assert!(path.join(".env").exists());
    ask(
        &host,
        "fixture",
        "_canoa/worktree/remove",
        json!({"path": path, "discard": true}),
    )
    .expect("removed with discard");
    assert!(!path.exists());
}

#[test]
fn a_repository_whose_own_configuration_runs_programs_is_refused() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    std::fs::write(repo.join("a.txt"), "text\n").expect("a file");
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "text"]);
    let (host, path) = host_with_worktree(&root, &repo, "canoa/before-a1b2c3");
    let marker = scratch.0.join("filter-ran");
    let command = format!(
        "sh -c 'echo ran > \"{}\"; cat'",
        marker.to_string_lossy().replace('\\', "/")
    );
    std::fs::write(
        repo.join(".git").join("info").join("attributes"),
        "*.txt filter=x\n",
    )
    .expect("attributes");
    git(&repo, &["config", "filter.x.smudge", &command]);
    git(&repo, &["config", "filter.x.clean", &command]);
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": "canoa/after-d4e5f6"}),
    )
    .expect_err("a filter in the repository's configuration");
    assert!(message(&refused).contains("configuration"), "{refused}");
    std::fs::write(path.join("a.txt"), "changed\n").expect("a file");
    for method in ["_canoa/worktree/status", "_canoa/worktree/remove"] {
        let refused = ask(&host, "fixture", method, json!({"path": path})).expect_err("refused");
        assert!(
            message(&refused).contains("configuration"),
            "{method}: {refused}"
        );
    }
    git(&repo, &["config", "--unset", "filter.x.smudge"]);
    git(&repo, &["config", "--unset", "filter.x.clean"]);
    git(&repo, &["config", "include.path", "elsewhere.cfg"]);
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": "canoa/after-d4e5f6"}),
    )
    .expect_err("an include in the repository's configuration");
    assert!(message(&refused).contains("configuration"), "{refused}");
    assert!(!marker.exists(), "a filter ran");
}

#[test]
fn a_submodule_or_a_repository_with_alternates_gets_no_worktree() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    let library = root.join("library");
    repository(&repo);
    repository(&library);
    git(
        &repo,
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
    git(&repo, &["commit", "-q", "-m", "submodule"]);
    let host = Host::start(&entry("fixture", &[&root], ""), 1);
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo.join("lib"), "branch": "canoa/sub-a1b2c3"}),
    )
    .expect_err("a submodule's worktree would go in the superproject's .git");
    assert_eq!(refused["code"], -32602, "{refused}");
    assert!(
        !repo
            .join(".git")
            .join("modules")
            .join("lib.worktrees")
            .exists()
    );

    let shared = root.join("shared");
    git(&root, &["clone", "-q", "--shared", "proj", "shared"]);
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": shared, "branch": "canoa/alt-a1b2c3"}),
    )
    .expect_err("objects from another repository");
    assert!(message(&refused).contains("alternates"), "{refused}");
}

#[test]
fn a_detached_head_with_commits_of_its_own_is_found_and_kept() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    let (host, path) = host_with_worktree(&root, &repo, "canoa/detached-a1b2c3");
    git(&path, &["checkout", "-q", "--detach"]);
    std::fs::write(path.join("work"), "x\n").expect("a file");
    git(&path, &["add", "work"]);
    git(&path, &["commit", "-q", "-m", "detached work"]);
    let status = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": path}),
    )
    .expect("still found");
    assert_eq!(status["exists"], true, "{status}");
    assert_eq!(status["detached"], true, "{status}");
    assert_eq!(status["commits"], 1, "{status}");
    for discard in [false, true] {
        let refused = ask(
            &host,
            "fixture",
            "_canoa/worktree/remove",
            json!({"path": path, "discard": discard}),
        )
        .expect_err("a commit only HEAD reaches would be lost");
        assert_eq!(refused["code"], -32602, "{refused}");
    }
    assert!(path.join("work").exists());
}

#[test]
fn a_status_tells_one_set_of_changes_from_another_of_the_same_size() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    let (host, path) = host_with_worktree(&root, &repo, "canoa/listing-a1b2c3");
    std::fs::write(path.join("one"), "").expect("a file");
    let first = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": path}),
    )
    .expect("a status");
    std::fs::remove_file(path.join("one")).expect("removed");
    std::fs::write(path.join("two"), "").expect("a file");
    let second = ask(
        &host,
        "fixture",
        "_canoa/worktree/status",
        json!({"path": path}),
    )
    .expect("a status");
    assert_eq!(first["changed"], second["changed"]);
    assert!(first["listing"].is_string(), "{first}");
    assert_ne!(first["listing"], second["listing"]);
}

#[test]
fn a_repository_has_at_most_so_many_worktrees_from_the_host() {
    if !has_git() {
        return;
    }
    let scratch = Scratch::new();
    let root = scratch.0.join("src");
    let repo = root.join("proj");
    repository(&repo);
    for index in 0..32 {
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                &format!("../proj.worktrees/w{index}"),
            ],
        );
    }
    let host = Host::start(&entry("fixture", &[&root], ""), 1);
    let refused = ask(
        &host,
        "fixture",
        "_canoa/worktree/new",
        json!({"cwd": repo, "branch": "canoa/one-more"}),
    )
    .expect_err("past the bound");
    assert!(message(&refused).contains("32"), "{refused}");
}
