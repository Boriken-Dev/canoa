use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::publish::web_address;
use super::tests::{Fixture, run_git};
use super::*;

/// A repository the tests commit in: its own identity and settings, so nothing the machine's own configuration says (signing,
/// automatic upstreams) reaches a test.
fn settled(fixture: &Fixture) {
    for (key, value) in [
        ("user.name", "Canoa Test"),
        ("user.email", "test@canoa.invalid"),
        ("commit.gpgsign", "false"),
        ("push.autoSetupRemote", "false"),
        ("push.default", "simple"),
        ("core.autocrlf", "false"),
    ] {
        run_git(&fixture.repo, &["config", key, value]);
    }
}

fn person(fixture: &Fixture) -> Worktrees {
    Worktrees::new(vec![fixture.scratch.join("src")], true).for_person()
}

fn ask(worktrees: &Worktrees, method: &str, params: &Value) -> Value {
    let message = json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
    let reply = worktrees
        .answer(message.to_string().as_bytes())
        .expect("a request is answered");
    serde_json::from_slice(&reply).expect("JSON")
}

fn words(reply: &Value) -> String {
    reply["error"]["message"].as_str().unwrap_or("").to_owned()
}

fn at(fixture: &Fixture) -> String {
    fixture.repo.to_string_lossy().into_owned()
}

/// A bare repository beside the fixture, as a remote.
fn bare_remote(fixture: &Fixture) -> PathBuf {
    let remote = fixture.scratch.join("remote.git");
    std::fs::create_dir_all(&remote).expect("a directory");
    run_git(&remote, &["init", "-q", "--bare"]);
    remote
}

fn branch(fixture: &Fixture) -> String {
    run_git(&fixture.repo, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_owned()
}

/// What the last commit holds, as paths.
fn last_commit_files(repo: &Path) -> Vec<String> {
    run_git(repo, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn only_the_person_commits_pushes_or_opens_a_pull_request() {
    let fixture = Fixture::new("publish-person");
    settled(&fixture);
    std::fs::write(fixture.repo.join("README"), "changed\n").expect("a file");
    let others = Worktrees::new(vec![fixture.scratch.join("src")], true);
    for (method, params) in [
        (COMMIT, json!({"path": at(&fixture), "message": "m"})),
        (PUSH, json!({"path": at(&fixture)})),
        (PULL_REQUEST, json!({"path": at(&fixture), "title": "t"})),
    ] {
        let reply = ask(&others, method, &params);
        assert!(words(&reply).contains("by the person"), "{method}: {reply}");
    }
    assert_eq!(
        run_git(&fixture.repo, &["log", "--format=%s"]).trim(),
        "first"
    );
}

#[test]
fn a_commit_takes_the_files_chosen_and_leaves_the_rest() {
    let fixture = Fixture::new("publish-chosen");
    settled(&fixture);
    let repo = &fixture.repo;
    std::fs::write(repo.join("README"), "changed\n").expect("a file");
    std::fs::write(repo.join("fresh.txt"), "new\n").expect("a file");
    std::fs::write(repo.join("other.txt"), "other\n").expect("a file");
    std::fs::remove_file(repo.join("sub").join("keep")).expect("removed");
    run_git(repo, &["add", "other.txt"]);
    let reply = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "chosen\n\nwith a body", "paths": ["fresh.txt", "sub/keep"]}),
    );
    assert_eq!(reply["result"]["summary"], "chosen", "{reply}");
    assert_eq!(reply["result"]["branch"], branch(&fixture), "{reply}");
    assert_eq!(
        reply["result"]["commit"],
        run_git(repo, &["rev-parse", "HEAD"]).trim(),
        "{reply}"
    );
    let mut files = last_commit_files(repo);
    files.sort();
    assert_eq!(files, ["fresh.txt", "sub/keep"]);
    // What was not chosen is still where it was: modified, and staged.
    let status = run_git(repo, &["status", "--porcelain"]);
    assert!(status.contains(" M README"), "{status}");
    assert!(status.contains("A  other.txt"), "{status}");
}

#[test]
fn a_commit_with_no_files_named_takes_everything_under_the_folder() {
    let fixture = Fixture::new("publish-all");
    settled(&fixture);
    let repo = &fixture.repo;
    std::fs::write(repo.join("README"), "changed\n").expect("a file");
    std::fs::write(repo.join("fresh.txt"), "new\n").expect("a file");
    let reply = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "everything"}),
    );
    assert_eq!(reply["result"]["summary"], "everything", "{reply}");
    let mut files = last_commit_files(repo);
    files.sort();
    assert_eq!(files, ["README", "fresh.txt"]);
    assert_eq!(run_git(repo, &["status", "--porcelain"]).trim(), "");
}

#[test]
fn a_file_name_is_taken_literally_and_never_as_an_option() {
    let fixture = Fixture::new("publish-literal");
    settled(&fixture);
    let repo = &fixture.repo;
    std::fs::write(repo.join("a.txt"), "a\n").expect("a file");
    std::fs::write(repo.join("b.txt"), "b\n").expect("a file");
    std::fs::write(repo.join("--all"), "x\n").expect("a file");
    let reply = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "literal", "paths": ["--all", "*.txt"]}),
    );
    // Neither a pattern nor an option: the named file goes in, and `*.txt` names no file.
    assert!(words(&reply).contains("*.txt"), "{reply}");
    let reply = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "literal", "paths": ["--all"]}),
    );
    assert_eq!(last_commit_files(repo), ["--all"], "{reply}");
    assert!(run_git(repo, &["status", "--porcelain"]).contains("?? a.txt"));
}

#[test]
fn git_says_why_a_commit_is_not_made() {
    let fixture = Fixture::new("publish-refused");
    settled(&fixture);
    let nothing = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "m"}),
    );
    assert!(words(&nothing).contains("nothing to commit"), "{nothing}");
    std::fs::write(fixture.repo.join("README"), "changed\n").expect("a file");
    let empty = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": ""}),
    );
    assert!(words(&empty).contains("empty commit message"), "{empty}");
    let none = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "m", "paths": []}),
    );
    assert!(words(&none).contains("no file is chosen"), "{none}");
    assert_eq!(
        run_git(&fixture.repo, &["log", "--format=%s"]).trim(),
        "first"
    );
}

#[test]
fn the_repositorys_own_hook_runs_for_the_person_and_its_words_come_back() {
    let fixture = Fixture::new("publish-hook");
    settled(&fixture);
    let hooks = fixture.repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).expect("a directory");
    run_git(
        &fixture.repo,
        &["config", "core.hooksPath", &hooks.to_string_lossy()],
    );
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'lint says no' >&2\nexit 1\n").expect("a hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("mode");
    }
    std::fs::write(fixture.repo.join("README"), "changed\n").expect("a file");
    let reply = ask(
        &person(&fixture),
        COMMIT,
        &json!({"path": at(&fixture), "message": "m"}),
    );
    assert!(words(&reply).contains("lint says no"), "{reply}");
    assert_eq!(
        run_git(&fixture.repo, &["log", "--format=%s"]).trim(),
        "first"
    );
}

#[test]
fn a_push_with_no_upstream_or_no_remote_says_what_git_says() {
    let fixture = Fixture::new("publish-no-upstream");
    settled(&fixture);
    let nowhere = ask(&person(&fixture), PUSH, &json!({"path": at(&fixture)}));
    assert!(
        words(&nowhere).contains("No configured push destination"),
        "{nowhere}"
    );
    let remote = bare_remote(&fixture);
    run_git(
        &fixture.repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    let none = ask(&person(&fixture), PUSH, &json!({"path": at(&fixture)}));
    assert!(words(&none).contains("no upstream branch"), "{none}");
    assert_eq!(run_git(&remote, &["branch", "--list"]).trim(), "");
}

#[test]
fn a_push_sets_the_upstream_when_asked_and_then_pushes_new_commits_plainly() {
    let fixture = Fixture::new("publish-push");
    settled(&fixture);
    let remote = bare_remote(&fixture);
    run_git(
        &fixture.repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    let name = branch(&fixture);
    let first = ask(
        &person(&fixture),
        PUSH,
        &json!({"path": at(&fixture), "setUpstream": true}),
    );
    assert_eq!(first["result"]["pushed"], true, "{first}");
    assert_eq!(first["result"]["branch"], name.as_str(), "{first}");
    assert_eq!(first["result"]["remote"], "origin", "{first}");
    assert_eq!(
        run_git(&remote, &["rev-parse", &name]),
        run_git(&fixture.repo, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        run_git(&fixture.repo, &["rev-parse", "--abbrev-ref", "@{u}"]).trim(),
        format!("origin/{name}")
    );
    std::fs::write(fixture.repo.join("README"), "later\n").expect("a file");
    run_git(&fixture.repo, &["commit", "-q", "-am", "later"]);
    let second = ask(&person(&fixture), PUSH, &json!({"path": at(&fixture)}));
    assert_eq!(second["result"]["pushed"], true, "{second}");
    assert_eq!(
        run_git(&remote, &["rev-parse", &name]),
        run_git(&fixture.repo, &["rev-parse", "HEAD"])
    );
}

#[test]
fn a_rejected_push_comes_back_in_gits_words_and_changes_nothing() {
    let fixture = Fixture::new("publish-rejected");
    settled(&fixture);
    let remote = bare_remote(&fixture);
    run_git(
        &fixture.repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    let name = branch(&fixture);
    let first = ask(
        &person(&fixture),
        PUSH,
        &json!({"path": at(&fixture), "setUpstream": true}),
    );
    assert_eq!(first["result"]["pushed"], true, "{first}");
    // Another clone moves the branch on.
    let other = fixture.scratch.join("other");
    run_git(
        &fixture.scratch,
        &["clone", "-q", &remote.to_string_lossy(), "other"],
    );
    std::fs::write(other.join("theirs"), "t\n").expect("a file");
    run_git(&other, &["add", "theirs"]);
    run_git(&other, &["commit", "-q", "-m", "theirs"]);
    run_git(&other, &["push", "-q", "origin", &name]);
    let theirs = run_git(&remote, &["rev-parse", &name]);
    std::fs::write(fixture.repo.join("README"), "ours\n").expect("a file");
    run_git(&fixture.repo, &["commit", "-q", "-am", "ours"]);
    let refused = ask(&person(&fixture), PUSH, &json!({"path": at(&fixture)}));
    let said = words(&refused);
    assert!(said.contains("rejected"), "{refused}");
    assert!(said.contains(&format!("{name} -> {name}")), "{refused}");
    assert_eq!(run_git(&remote, &["rev-parse", &name]), theirs);
}

#[test]
fn a_remote_is_never_read_as_an_option() {
    let fixture = Fixture::new("publish-option");
    settled(&fixture);
    let reply = ask(
        &person(&fixture),
        PUSH,
        &json!({"path": at(&fixture), "remote": "--mirror"}),
    );
    assert!(words(&reply).contains("not an option"), "{reply}");
}

/// A copy of `examples/version_standin` named `gh` in a new folder, printing `line` and ending with `exit`.
fn standin_gh(fixture: &Fixture, line: &str, exit: u8) -> std::ffi::OsString {
    let exe = std::env::current_exe().expect("test path");
    let debug = exe.parent().expect("deps").parent().expect("debug");
    let name = if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    };
    let source = debug.join("examples").join(name);
    assert!(
        source.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    let folder = fixture.scratch.join(format!("tools-{exit}-{}", line.len()));
    std::fs::create_dir_all(&folder).expect("a directory");
    let mut program = std::fs::read(&source).expect("the stand-in");
    program.extend_from_slice(b"\nCOQUI-STANDIN:");
    program.extend_from_slice(json!({"line": line, "exit": exit}).to_string().as_bytes());
    program.push(b'\n');
    let target = folder.join(if cfg!(windows) { "gh.exe" } else { "gh" });
    std::fs::write(&target, program).expect("the program");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("mode");
    }
    folder.into_os_string()
}

#[test]
fn a_pull_request_is_made_by_gh_in_the_folder() {
    let fixture = Fixture::new("publish-gh");
    settled(&fixture);
    let tools = standin_gh(&fixture, "https://forge.invalid/acme/widgets/pull/7", 0);
    let reply = ask(
        &person(&fixture).with_tools_path(tools),
        PULL_REQUEST,
        &json!({"path": at(&fixture), "title": "A title", "body": "Why", "base": "main", "draft": true}),
    );
    assert_eq!(reply["result"]["via"], "gh", "{reply}");
    assert_eq!(
        reply["result"]["url"], "https://forge.invalid/acme/widgets/pull/7",
        "{reply}"
    );
    assert_eq!(
        reply["result"]["branch"],
        branch(&fixture).as_str(),
        "{reply}"
    );
}

#[test]
fn ghs_own_refusal_is_the_answer_when_gh_is_there() {
    let fixture = Fixture::new("publish-gh-refused");
    settled(&fixture);
    let tools = standin_gh(&fixture, "gh: not logged in; run gh auth login", 1);
    let reply = ask(
        &person(&fixture).with_tools_path(tools),
        PULL_REQUEST,
        &json!({"path": at(&fixture), "title": "t"}),
    );
    assert!(words(&reply).contains("not logged in"), "{reply}");
    assert!(reply.get("result").is_none(), "{reply}");
}

#[test]
fn with_no_gh_the_forges_web_page_opens_the_pull_request() {
    let fixture = Fixture::new("publish-web");
    settled(&fixture);
    let nothing = fixture.scratch.join("nothing-here");
    std::fs::create_dir_all(&nothing).expect("a directory");
    run_git(
        &fixture.repo,
        &["remote", "add", "origin", "git@github.com:acme/widgets.git"],
    );
    let name = branch(&fixture);
    let rule = || person(&fixture).with_tools_path(nothing.clone().into_os_string());
    let reply = ask(
        &rule(),
        PULL_REQUEST,
        &json!({"path": at(&fixture), "title": "t"}),
    );
    assert_eq!(reply["result"]["via"], "web", "{reply}");
    assert_eq!(
        reply["result"]["url"],
        format!("https://github.com/acme/widgets/compare/{name}?expand=1"),
        "{reply}"
    );
    // A remote with no web address is said so, not guessed.
    run_git(
        &fixture.repo,
        &[
            "remote",
            "set-url",
            "origin",
            &fixture.scratch.to_string_lossy(),
        ],
    );
    let local = ask(
        &rule(),
        PULL_REQUEST,
        &json!({"path": at(&fixture), "title": "t"}),
    );
    assert!(words(&local).contains("no web address"), "{local}");
}

#[test]
fn a_remotes_address_becomes_the_page_that_starts_a_pull_request() {
    let page = |address, branch, base| web_address(address, branch, base);
    assert_eq!(
        page("https://github.com/acme/widgets.git", "feature/x y", None).as_deref(),
        Some("https://github.com/acme/widgets/compare/feature/x%20y?expand=1")
    );
    assert_eq!(
        page("ssh://git@github.com:22/acme/widgets", "b", Some("main")).as_deref(),
        Some("https://github.com/acme/widgets/compare/main...b?expand=1")
    );
    assert_eq!(
        page(
            "https://token@git.example.com:8443/team/sub/repo.git/",
            "b",
            None
        )
        .as_deref(),
        Some("https://git.example.com:8443/team/sub/repo/compare/b?expand=1")
    );
    assert_eq!(
        page("git@gitlab.com:team/sub/repo.git", "fix/a", Some("dev")).as_deref(),
        Some(
            "https://gitlab.com/team/sub/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix%2Fa&merge_request%5Btarget_branch%5D=dev"
        )
    );
    assert_eq!(
        page("https://bitbucket.org/acme/widgets.git", "b", None).as_deref(),
        Some("https://bitbucket.org/acme/widgets/pull-requests/new?source=b")
    );
    for local in [
        "/srv/git/widgets.git",
        "C:\\repos\\widgets",
        "C:/repos/widgets",
        "../widgets",
        "file:///srv/git/widgets.git",
        "https://github.com/widgets",
    ] {
        assert_eq!(page(local, "b", None), None, "{local}");
    }
}
