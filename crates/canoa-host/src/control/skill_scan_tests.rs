//! `skill/scan`: the skills a git source holds, found the way `npx skills` finds them, over bare repositories laid out like the
//! ones people publish skills in. Nothing here reaches the network.

use super::overlays_tests::{ask, dir, key, repo, source};
use super::*;

fn skill_md(name: &str) -> String {
    format!("---\nname: {name}\ndescription: The {name} skill.\n---\n# {name}\n")
}

fn host(name: &str) -> (Control, PathBuf, PathBuf) {
    let tmp = dir(name);
    let overlays = tmp.join("cfg").join("overlays");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_overlays(Overlays::new(
        Some(overlays.clone()),
        OverlayLimits::default(),
    ));
    (control, tmp, overlays)
}

fn scan(control: &Control, source: Value) -> Value {
    ask(control, &Caller::Owner, "skill/scan", source)
}

/// The skills a scan listed, as `(name, path)`.
fn listed(answer: &Value) -> Vec<(String, String)> {
    answer["result"]["skills"]
        .as_array()
        .unwrap_or_else(|| panic!("no skills in {answer}"))
        .iter()
        .map(|skill| {
            (
                skill["name"].as_str().expect("name").to_owned(),
                skill["path"].as_str().expect("path").to_owned(),
            )
        })
        .collect()
}

fn pair(name: &str, path: &str) -> (String, String) {
    (name.to_owned(), path.to_owned())
}

#[test]
fn a_repository_laid_out_like_anthropics_skills_lists_each_skill_and_installs_one() {
    let (control, tmp, overlays) = host("scan-anthropic");
    let pdf = skill_md("pdf");
    let docx = skill_md("docx");
    let upstream = repo(
        &tmp,
        "skills",
        &[
            ("README.md", "# Skills\n"),
            ("skills/pdf/SKILL.md", &pdf),
            ("skills/pdf/scripts/fill.py", "print('x')\n"),
            ("skills/docx/SKILL.md", &docx),
            ("template/SKILL.md", &skill_md("template")),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(
        listed(&answer),
        [pair("docx", "skills/docx"), pair("pdf", "skills/pdf")],
        "{answer}"
    );
    let first = &answer["result"]["skills"][0];
    assert_eq!(first["description"], "The docx skill.");
    assert!(first.get("problem").is_none(), "{first}");
    assert_eq!(answer["result"]["commit"].as_str().map(str::len), Some(40));
    assert_eq!(answer["result"]["skipped"], 0);
    assert_eq!(answer["result"]["truncated"], false);
    // A scan is a list: it carries no file of any skill, and keeps nothing.
    assert!(!answer.to_string().contains("fill.py"), "{answer}");
    let left: Vec<_> = fs::read_dir(overlays.join(".skills"))
        .map(|read| read.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");

    // One is installed from where the scan said it is: the files of that folder, at the commit the scan read.
    let mut fetch = source(&upstream, "skills/pdf");
    fetch["ref"] = answer["result"]["commit"].clone();
    let files = ask(&control, &Caller::Owner, "skill/fetch", fetch);
    let names: Vec<&String> = files["result"]["files"]
        .as_object()
        .expect("files")
        .keys()
        .collect();
    assert_eq!(names, ["SKILL.md", "scripts/fill.py"], "{files}");
}

#[test]
fn a_repository_of_curated_skills_lists_those_under_skills_and_its_dot_folders() {
    let (control, tmp, _overlays) = host("scan-quality");
    let upstream = repo(
        &tmp,
        "web-quality-skills",
        &[
            ("skills/performance/SKILL.md", &skill_md("performance")),
            ("skills/accessibility/SKILL.md", &skill_md("accessibility")),
            (
                "skills/.curated/best-practices/SKILL.md",
                &skill_md("best-practices"),
            ),
            ("skills/.experimental/seo/SKILL.md", &skill_md("seo")),
            ("docs/example/SKILL.md", &skill_md("example")),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(
        listed(&answer),
        [
            pair("accessibility", "skills/accessibility"),
            pair("best-practices", "skills/.curated/best-practices"),
            pair("performance", "skills/performance"),
            pair("seo", "skills/.experimental/seo"),
        ],
        "a skill outside the usual folders is not offered while there are some in them: {answer}"
    );
}

#[test]
fn a_repository_with_an_agents_skills_folder_lists_those_skills() {
    let (control, tmp, _overlays) = host("scan-agent-folders");
    let upstream = repo(
        &tmp,
        "project",
        &[
            (".claude/skills/review/SKILL.md", &skill_md("review")),
            (".claude/skills/review/helpers/check.sh", "echo ok\n"),
            (".agents/skills/lint/SKILL.md", &skill_md("lint")),
            ("src/main.rs", "fn main() {}\n"),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(
        listed(&answer),
        [
            pair("lint", ".agents/skills/lint"),
            pair("review", ".claude/skills/review")
        ],
        "{answer}"
    );
    let files = ask(
        &control,
        &Caller::Owner,
        "skill/fetch",
        source(&upstream, ".claude/skills/review"),
    );
    let names: Vec<&String> = files["result"]["files"]
        .as_object()
        .expect("files")
        .keys()
        .collect();
    assert_eq!(names, ["SKILL.md", "helpers/check.sh"], "{files}");
}

#[test]
fn a_repository_that_is_one_skill_lists_it_alone_with_the_whole_repository_as_its_files() {
    let (control, tmp, _overlays) = host("scan-root");
    let upstream = repo(
        &tmp,
        "one",
        &[
            ("SKILL.md", &skill_md("whole")),
            ("notes.md", "n\n"),
            ("skills/other/SKILL.md", &skill_md("other")),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(listed(&answer), [pair("whole", "")], "{answer}");
    let files = ask(
        &control,
        &Caller::Owner,
        "skill/fetch",
        source(&upstream, ""),
    );
    assert!(
        files["result"]["files"]["notes.md"].is_string(),
        "the root skill's files are the repository's: {files}"
    );
}

#[test]
fn skills_outside_the_usual_folders_are_found_to_a_depth_and_not_beyond_it() {
    let (control, tmp, _overlays) = host("scan-depth");
    let upstream = repo(
        &tmp,
        "deep",
        &[
            ("tools/one/two/shallow/SKILL.md", &skill_md("shallow")),
            ("a/b/c/d/e/f/deep/SKILL.md", &skill_md("deep")),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(
        listed(&answer),
        [pair("shallow", "tools/one/two/shallow")],
        "{answer}"
    );
}

#[test]
fn a_scan_can_start_in_a_folder_of_the_source() {
    let (control, tmp, _overlays) = host("scan-path");
    let upstream = repo(
        &tmp,
        "mono",
        &[
            ("packages/agent/skills/review/SKILL.md", &skill_md("review")),
            ("packages/other/skills/lint/SKILL.md", &skill_md("lint")),
        ],
    );
    let answer = scan(&control, source(&upstream, "packages/agent"));
    assert_eq!(
        listed(&answer),
        [pair("review", "packages/agent/skills/review")],
        "{answer}"
    );
    // The folder itself holding a SKILL.md is its one skill, named by its folder.
    let one = scan(&control, source(&upstream, "packages/agent/skills/review"));
    assert_eq!(
        listed(&one),
        [pair("review", "packages/agent/skills/review")],
        "{one}"
    );
}

#[test]
fn what_is_no_skill_is_left_out_and_what_cannot_be_installed_says_why() {
    let (control, tmp, _overlays) = host("scan-problems");
    let upstream = repo(
        &tmp,
        "mixed",
        &[
            ("skills/good/SKILL.md", &skill_md("good")),
            ("skills/plain/SKILL.md", "# no front matter\n"),
            (
                "skills/nodescription/SKILL.md",
                "---\nname: nodescription\n---\nbody\n",
            ),
            (
                "skills/hidden/SKILL.md",
                "---\nname: hidden\ndescription: Internal.\nmetadata:\n  internal: true\n---\nbody\n",
            ),
            ("skills/folder/SKILL.md", &skill_md("another")),
            ("skills/copy/SKILL.md", &skill_md("good")),
            ("skills/dots/SKILL.md", &skill_md("dots")),
            ("skills/node_modules/x/SKILL.md", &skill_md("x")),
            (
                "skills/odd/SKILL.md",
                "---\nname: \"odd name\"\ndescription: Spaces.\n---\nbody\n",
            ),
        ],
    );
    let answer = scan(&control, source(&upstream, ""));
    let skills = answer["result"]["skills"].as_array().expect("skills");
    let by_name = |name: &str| skills.iter().find(|skill| skill["name"] == name);
    assert!(by_name("good").is_some_and(|skill| skill.get("problem").is_none()));
    assert!(by_name("dots").is_some(), "{answer}");
    let mismatch = by_name("another").expect("listed");
    assert!(
        mismatch["problem"]
            .as_str()
            .is_some_and(|why| why.contains("folder")),
        "{mismatch}"
    );
    let odd = by_name("odd name").expect("listed");
    assert!(odd["problem"].is_string(), "{odd}");
    for left_out in ["plain", "nodescription", "hidden", "x"] {
        assert!(by_name(left_out).is_none(), "{left_out}: {answer}");
    }
    // `skills/copy` names a skill already listed: its first folder keeps the name.
    assert_eq!(
        skills
            .iter()
            .filter(|skill| skill["name"] == "good")
            .count(),
        1
    );
    // The plain file, the one with no description, the internal one and the copy.
    assert_eq!(answer["result"]["skipped"], 4, "{answer}");
}

#[test]
fn a_source_with_no_skill_lists_none_and_a_missing_branch_is_refused() {
    let (control, tmp, _overlays) = host("scan-none");
    let upstream = repo(&tmp, "empty", &[("README.md", "nothing\n")]);
    let answer = scan(&control, source(&upstream, ""));
    assert_eq!(listed(&answer), [], "{answer}");
    let mut missing = source(&upstream, "");
    missing["ref"] = "nope".into();
    let refused = scan(&control, missing);
    assert!(refused["error"].is_object(), "{refused}");
    // A source that names no ref is the repository's head.
    let mut headless = source(&upstream, "");
    headless["ref"] = Value::Null;
    assert!(scan(&control, headless)["error"].is_null());
}

#[test]
fn a_scan_follows_the_overlay_grants_and_the_places_a_key_may_browse() {
    let (control, tmp, _overlays) = host("scan-grants");
    let upstream = repo(&tmp, "tools", &[("SKILL.md", &skill_md("tools"))]);
    let reader = key("reader", &[Grant::OverlayRead]);
    let writer = key("writer", &[Grant::OverlayWrite]);
    let denied = ask(&control, &reader, "skill/scan", source(&upstream, ""));
    assert_eq!(denied["error"]["code"], -32001, "{denied}");
    let local = ask(&control, &writer, "skill/scan", source(&upstream, ""));
    assert_eq!(local["error"]["code"], -32002, "{local}");
    let mut bad = source(&upstream, "");
    bad["own"] = "yes".into();
    let wrong = scan(&control, bad);
    assert!(wrong["error"].is_object(), "{wrong}");
}
