use std::path::{Path, PathBuf};
use std::process::Command;

use super::files::{Files, UploadLimits};
use super::overlays::{OverlayLimits, Overlays};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-ov-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    // Where the path is written the way git and the source string name it.
    fs::canonicalize(&dir)
        .map(|resolved| {
            let text = resolved.to_string_lossy().into_owned();
            PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
        })
        .expect("resolves")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(["-c", "core.autocrlf=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repository on `main` holding `files`, committed.
fn repo(root: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let repo = root.join(name);
    fs::create_dir_all(&repo).expect("repo");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    change(&repo, files, "first");
    repo
}

fn change(repo: &Path, files: &[(&str, &str)], message: &str) {
    for (path, text) in files {
        let file = repo.join(path);
        fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
        fs::write(file, text).expect("file");
    }
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "--quiet", "-m", message]);
}

/// The source for `repo`'s `main`, naming the folder `path` in it: what `overlay/check` and `catalog/fetch` are given.
fn source(repo: &Path, path: &str) -> Value {
    let text = repo.to_string_lossy().replace('\\', "/");
    let url = if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    };
    json!({"repo": url, "ref": "main", "path": path})
}

/// `base` with the members of `add`.
fn plus(mut base: Value, add: Value) -> Value {
    if let (Some(base), Some(add)) = (base.as_object_mut(), add.as_object()) {
        base.extend(add.clone());
    }
    base
}

struct Setup {
    tmp: PathBuf,
    control: Control,
    overlays: PathBuf,
}

fn setup(grants_home: Option<&Path>, limits: OverlayLimits) -> Setup {
    setup_with(grants_home, Vec::new(), None, limits)
}

fn setup_with(
    grants_home: Option<&Path>,
    roots: Vec<EntryRoots>,
    config: Option<PathBuf>,
    limits: OverlayLimits,
) -> Setup {
    let tmp = dir("setup");
    let overlays = tmp.join("cfg").join("overlays");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        grants_home.map(Path::to_path_buf),
        roots,
        config,
        None,
        UploadLimits::default(),
    ))
    .with_overlays(Overlays::new(Some(overlays.clone()), limits));
    Setup {
        tmp,
        control,
        overlays,
    }
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn key(id: &str, grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

const BOTH: [Grant; 2] = [Grant::OverlayRead, Grant::OverlayWrite];

fn check(s: &Setup, who: &Caller, name: &str, source: &Value) -> Value {
    ask(
        &s.control,
        who,
        "overlay/check",
        plus(json!({"name": name}), source.clone()),
    )
}

fn installed_digest(dir: &Path) -> String {
    coqui_aos::tree_digest(&coqui_aos::StdFs, dir, &coqui_aos::Settings::default())
        .expect("digest")
        .digest
        .to_string()
}

#[test]
fn an_overlay_is_checked_installed_with_the_digest_of_what_was_written_and_listed() {
    let s = setup(None, OverlayLimits::default());
    let upstream = repo(
        &s.tmp,
        "up",
        &[
            ("team/OVERLAY.md", "# team\n"),
            ("team/context/a.md", "alpha\n"),
            ("team/node_modules/x/index.js", "skipped\n"),
            ("other/ignored.txt", "not this overlay\n"),
        ],
    );
    let who = Caller::Owner;
    let checked = check(&s, &who, "team", &source(&upstream, "team"));
    let result = &checked["result"];
    assert!(
        result["token"]
            .as_str()
            .is_some_and(|t| t.starts_with("ov-")),
        "{checked}"
    );
    let files: Vec<&String> = result["files"].as_object().expect("files").keys().collect();
    assert_eq!(files, ["OVERLAY.md", "context/a.md"], "{checked}");
    assert_eq!(result["skipped"], json!(["node_modules/x/index.js"]));
    assert_eq!(result["added"], json!(["OVERLAY.md", "context/a.md"]));
    assert_eq!(result["manifest"], "# team\n");
    assert_eq!(result["from"], Value::Null);
    // Checking installs nothing.
    assert!(!s.overlays.join("team").exists());

    let applied = ask(
        &s.control,
        &who,
        "overlay/apply",
        json!({"token": result["token"], "catalog": "mine"}),
    );
    assert_eq!(applied["result"]["digest"], result["digest"], "{applied}");
    assert_eq!(
        fs::read_to_string(s.overlays.join("team/context/a.md")).expect("file"),
        "alpha\n"
    );
    assert!(!s.overlays.join("team/node_modules").exists());
    // The digest in the record is the digest of the folder as written.
    assert_eq!(installed_digest(&s.overlays.join("team")), result["digest"]);
    let listed = ask(&s.control, &who, "overlay/installed", json!({}));
    assert_eq!(
        listed["result"]["overlays"]["team"]["catalog"], "mine",
        "{listed}"
    );
    assert_eq!(
        listed["result"]["overlays"]["team"]["commit"],
        result["commit"]
    );
    // A plan is used once.
    let again = ask(
        &s.control,
        &who,
        "overlay/apply",
        json!({"token": result["token"]}),
    );
    assert_eq!(again["error"]["code"], REFUSED, "{again}");

    // An update shows what changed and replaces the folder whole.
    change(
        &upstream,
        &[
            ("team/context/a.md", "beta\n"),
            ("team/context/b.md", "new\n"),
        ],
        "second",
    );
    fs::remove_file(upstream.join("team/OVERLAY.md")).expect("removed");
    fs::write(upstream.join("team/OVERLAY.md"), "# team 2\n").expect("rewritten");
    git(&upstream, &["add", "-A"]);
    git(&upstream, &["commit", "--quiet", "-m", "third"]);
    let next = check(&s, &who, "team", &source(&upstream, "team"));
    assert_eq!(
        next["result"]["changed"],
        json!(["OVERLAY.md", "context/a.md"]),
        "{next}"
    );
    assert_eq!(next["result"]["added"], json!(["context/b.md"]));
    assert_eq!(next["result"]["from"], result["commit"]);
    fs::write(s.overlays.join("team/edited-by-hand.md"), "mine").expect("edit");
    let applied = ask(
        &s.control,
        &who,
        "overlay/apply",
        json!({"token": next["result"]["token"]}),
    );
    assert!(applied["result"].is_object(), "{applied}");
    assert!(
        !s.overlays.join("team/edited-by-hand.md").exists(),
        "the folder is replaced whole"
    );
    assert_eq!(
        fs::read_to_string(s.overlays.join("team/context/b.md")).expect("file"),
        "new\n"
    );
    // The catalog a store installed it from is kept by the same source.
    let listed = ask(
        &s.control,
        &who,
        "overlay/installed",
        json!({"name": "team"}),
    );
    assert_eq!(
        listed["result"]["overlays"]["team"]["catalog"], "mine",
        "{listed}"
    );

    // What a check holds is readable to show what it would do.
    let again = check(&s, &who, "team", &source(&upstream, "team"));
    let read = ask(
        &s.control,
        &who,
        "overlay/read",
        json!({"token": again["result"]["token"]}),
    );
    assert_eq!(
        read["result"]["files"]["context/b.md"],
        data_encoding::BASE64.encode(b"new\n"),
        "{read}"
    );

    let removed = ask(&s.control, &who, "overlay/remove", json!({"name": "team"}));
    assert_eq!(removed["result"], json!({"removed": true}));
    assert!(!s.overlays.join("team").exists() && !s.overlays.join("team.json").exists());
    let listed = ask(&s.control, &who, "overlay/installed", json!({}));
    assert_eq!(listed["result"]["overlays"], json!({}), "{listed}");
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_pin_that_is_not_the_trees_digest_installs_nothing() {
    let s = setup(None, OverlayLimits::default());
    let upstream = repo(&s.tmp, "up", &[("OVERLAY.md", "x\n")]);
    let who = Caller::Owner;
    let wrong = format!("sha256:{}", "0".repeat(64));
    let refused = ask(
        &s.control,
        &who,
        "overlay/check",
        plus(json!({"name": "o", "pin": wrong}), source(&upstream, "")),
    );
    assert_eq!(refused["error"]["code"], REFUSED, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("nothing was installed"))
    );
    assert!(!s.overlays.join("o").exists());
    let right = check(&s, &who, "o", &source(&upstream, ""));
    let pinned = ask(
        &s.control,
        &who,
        "overlay/check",
        plus(
            json!({"name": "o", "pin": right["result"]["digest"]}),
            source(&upstream, ""),
        ),
    );
    assert!(pinned["result"]["token"].is_string(), "{pinned}");
    let bad = ask(
        &s.control,
        &who,
        "overlay/check",
        plus(json!({"name": "o", "pin": "nope"}), source(&upstream, "")),
    );
    assert_eq!(bad["error"]["code"], BAD_PARAMS, "{bad}");
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_source_that_cannot_be_an_overlay_is_refused_whole() {
    let s = setup(None, OverlayLimits::default());
    let who = Caller::Owner;
    // A link, planted without a checkout that could refuse it.
    let linked = repo(&s.tmp, "linked", &[("OVERLAY.md", "x\n")]);
    let target = {
        std::fs::write(linked.join("t.txt"), "../outside").expect("t");
        git(&linked, &["hash-object", "-w", "t.txt"])
    };
    git(
        &linked,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{target},link"),
        ],
    );
    git(&linked, &["commit", "--quiet", "-m", "link"]);
    let answer = check(&s, &who, "linked", &source(&linked, ""));
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("link or a submodule")),
        "{answer}"
    );
    // Two names a case-folding file system would merge.
    let folded = repo(&s.tmp, "folded", &[("OVERLAY.md", "x\n")]);
    let one = git(&folded, &["hash-object", "-w", "OVERLAY.md"]);
    git(
        &folded,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{one},overlay.md"),
        ],
    );
    git(&folded, &["commit", "--quiet", "-m", "case"]);
    let answer = check(&s, &who, "folded", &source(&folded, ""));
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("differ only in case")),
        "{answer}"
    );
    // Nothing at the path.
    let empty = repo(&s.tmp, "empty", &[("OVERLAY.md", "x\n")]);
    let answer = check(&s, &who, "empty", &source(&empty, "missing"));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    // No branch of that name.
    let answer = ask(
        &s.control,
        &who,
        "overlay/check",
        plus(
            json!({"name": "empty"}),
            json!({"repo": source(&empty, "")["repo"], "ref": "nothere", "path": ""}),
        ),
    );
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("no branch or tag")),
        "{answer}"
    );
    assert!(!s.overlays.join("linked").exists() && !s.overlays.join("folded").exists());
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn an_overlay_is_held_to_its_limits() {
    let s = setup(
        None,
        OverlayLimits {
            files: 2,
            bytes: 10,
        },
    );
    let who = Caller::Owner;
    let many = repo(&s.tmp, "many", &[("a", "1"), ("b", "2"), ("c", "3")]);
    let answer = check(&s, &who, "many", &source(&many, ""));
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("more than 2 files")),
        "{answer}"
    );
    let big = repo(&s.tmp, "big", &[("a", "0123456789ABC")]);
    let answer = check(&s, &who, "big", &source(&big, ""));
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("more than 10 bytes")),
        "{answer}"
    );
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_plan_is_its_keys_alone_and_each_method_needs_its_grant() {
    let s = setup(None, OverlayLimits::default());
    let upstream = repo(&s.tmp, "up", &[("OVERLAY.md", "x\n")]);
    let owner_of_plan = Caller::Owner;
    let checked = check(&s, &owner_of_plan, "o", &source(&upstream, ""));
    let token = checked["result"]["token"].clone();
    // Another key holding the grants cannot spend it, nor read it.
    let other = key("sha256:bb", &BOTH);
    for method in ["overlay/apply", "overlay/read"] {
        let answer = ask(&s.control, &other, method, json!({"token": token}));
        assert_eq!(answer["error"]["code"], REFUSED, "{method}: {answer}");
    }
    assert!(!s.overlays.join("o").exists());
    // A key with neither grant gets nothing; one with read cannot fetch or install.
    let nothing = key("sha256:aa", &[Grant::LedgerRead]);
    let reader = key("sha256:cc", &[Grant::OverlayRead]);
    for (who, method, params) in [
        (&nothing, "overlay/installed", json!({})),
        (&nothing, "catalog/read", json!({})),
        (
            &reader,
            "overlay/check",
            plus(json!({"name": "o"}), source(&upstream, "")),
        ),
        (&reader, "overlay/apply", json!({"token": token})),
        (&reader, "overlay/remove", json!({"name": "o"})),
        (&reader, "overlay/discard", json!({"name": "o"})),
        (
            &reader,
            "catalog/fetch",
            plus(json!({"label": "c"}), source(&upstream, "")),
        ),
        (&reader, "catalog/remove", json!({"label": "c"})),
    ] {
        let answer = ask(&s.control, who, method, params);
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let listed = ask(&s.control, &reader, "overlay/installed", json!({}));
    assert_eq!(listed["result"]["overlays"], json!({}));
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_key_names_a_source_on_this_computer_only_under_a_working_root_and_with_the_folders_grant() {
    let tmp = dir("places");
    let home = tmp.join("home");
    let work = home.join("work");
    fs::create_dir_all(&work).expect("work");
    let inside = repo(&work, "mine", &[("OVERLAY.md", "x\n")]);
    let in_home = repo(&home, "elsewhere", &[("OVERLAY.md", "secret\n")]);
    let outside = repo(&tmp, "private", &[("OVERLAY.md", "secret\n")]);
    let cfg = home.join("cfg");
    let kept = repo(&cfg, "overlays", &[("OVERLAY.md", "kept\n")]);
    let roots = vec![EntryRoots {
        peers: None,
        roots: vec![work.clone(), cfg.clone()],
    }];
    let s = setup_with(
        Some(&home),
        roots,
        Some(cfg.join("canoa.cfg")),
        OverlayLimits::default(),
    );
    let who = key(
        "sha256:aa",
        &[Grant::OverlayRead, Grant::OverlayWrite, Grant::Folders],
    );
    let allowed = check(&s, &who, "mine", &source(&inside, ""));
    assert!(allowed["result"]["token"].is_string(), "{allowed}");
    // Not the home, though it may be browsed; not outside; not the folder the host keeps its overlays in, even as a working root.
    for (name, repo) in [("home", &in_home), ("private", &outside), ("kept", &kept)] {
        let denied = check(&s, &who, name, &source(repo, ""));
        assert_eq!(denied["error"]["code"], REFUSED, "{name}: {denied}");
        assert!(
            denied["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("must be in a folder"))
        );
    }
    let denied = ask(
        &s.control,
        &who,
        "catalog/fetch",
        plus(json!({"label": "c"}), source(&in_home, "")),
    );
    assert_eq!(denied["error"]["code"], REFUSED, "{denied}");
    // Without the folders grant, a working root is not enough.
    let blind = key("sha256:bb", &BOTH);
    let denied = check(&s, &blind, "mine2", &source(&inside, ""));
    assert_eq!(denied["error"]["code"], REFUSED, "{denied}");
    // A network share, however it is written, is refused by its text.
    for repo in [
        "file:////attacker/share/x",
        "file://///attacker/share",
        "//attacker/share",
        "\\\\attacker\\share",
    ] {
        let denied = check(
            &s,
            &who,
            "unc",
            &json!({"repo": repo, "ref": "main", "path": ""}),
        );
        assert!(denied["error"].is_object(), "{repo}: {denied}");
    }
    // A working tree whose `.git` is a file points elsewhere, so it is no source.
    let linked = repo(&work, "linked", &[("OVERLAY.md", "x\n")]);
    fs::remove_dir_all(linked.join(".git")).expect("no git dir");
    fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", in_home.join(".git").display()),
    )
    .expect("a gitfile");
    let denied = check(&s, &who, "linked", &source(&linked, ""));
    assert_eq!(denied["error"]["code"], REFUSED, "{denied}");
    // The person running the host names any.
    let owner = check(&s, &Caller::Owner, "private", &source(&outside, ""));
    assert!(owner["result"]["token"].is_string(), "{owner}");
    let _ = fs::remove_dir_all(tmp);
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_catalog_is_fetched_and_its_files_read_literally_within_a_size() {
    let s = setup(None, OverlayLimits::default());
    let upstream = repo(
        &s.tmp,
        "cat",
        &[
            ("catalog.toml", "[overlays]\nteam = \"x\"\n"),
            ("icons/team.png", "0123456789"),
            ("sub/dir/f.txt", "f"),
        ],
    );
    let who = Caller::Owner;
    let fetched = ask(
        &s.control,
        &who,
        "catalog/fetch",
        plus(json!({"label": "main"}), source(&upstream, "catalog.toml")),
    );
    let commit = fetched["result"]["commit"]
        .as_str()
        .expect("a commit")
        .to_owned();
    assert_eq!(commit, git(&upstream, &["rev-parse", "HEAD"]));
    let read = ask(
        &s.control,
        &who,
        "catalog/read",
        json!({"label": "main", "commit": commit, "maxBytes": 5,
               "paths": ["catalog.toml", "icons/team.png", "sub/dir", "missing.txt", "../x", "sub/dir/f.txt"]}),
    );
    let files = &read["result"]["files"];
    assert_eq!(files["catalog.toml"]["size"], 22, "{read}");
    assert_eq!(
        files["icons/team.png"],
        json!({"size": 10}),
        "over the size: no data"
    );
    assert_eq!(
        files["sub/dir/f.txt"]["data"],
        data_encoding::BASE64.encode(b"f")
    );
    for none in ["sub/dir", "missing.txt", "../x"] {
        assert_eq!(files[none], Value::Null, "{none}: {read}");
    }
    // A commit that is not a whole id is refused before git is asked.
    let bad = ask(
        &s.control,
        &who,
        "catalog/read",
        json!({"label": "main", "commit": "HEAD", "paths": []}),
    );
    assert_eq!(bad["error"]["code"], BAD_PARAMS, "{bad}");
    let gone = ask(&s.control, &who, "catalog/remove", json!({"label": "main"}));
    assert_eq!(gone["result"], json!({"removed": true}));
    assert!(!s.overlays.join(".catalogs").join("main.git").exists());
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_check_that_was_not_installed_is_discarded_and_an_install_is_kept() {
    let s = setup(None, OverlayLimits::default());
    let upstream = repo(&s.tmp, "up", &[("OVERLAY.md", "x\n")]);
    let who = Caller::Owner;
    check(&s, &who, "o", &source(&upstream, ""));
    let cache = s.overlays.join(".cache").join("o.git");
    assert!(cache.exists());
    ask(&s.control, &who, "overlay/discard", json!({"name": "o"}));
    assert!(!cache.exists());
    let checked = check(&s, &who, "o", &source(&upstream, ""));
    ask(
        &s.control,
        &who,
        "overlay/apply",
        json!({"token": checked["result"]["token"]}),
    );
    ask(&s.control, &who, "overlay/discard", json!({"name": "o"}));
    assert!(cache.exists(), "an installed overlay keeps its history");
    // Names that are not names.
    for name in ["", "../x", "a b", "CON", &"x".repeat(65)] {
        let answer = ask(&s.control, &who, "overlay/remove", json!({"name": name}));
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{name}: {answer}");
    }
    let _ = fs::remove_dir_all(s.tmp);
}

#[test]
fn a_host_with_no_canoa_cfg_keeps_no_overlays() {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    for method in ["overlay/installed", "overlay/remove"] {
        let answer = ask(&control, &Caller::Owner, method, json!({"name": "o"}));
        assert_eq!(answer["error"]["code"], REFUSED, "{method}: {answer}");
    }
}
