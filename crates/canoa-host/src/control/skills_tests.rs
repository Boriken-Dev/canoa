use super::overlays_tests::{ask, dir, key, repo, source};
use super::*;

fn skill_text(name: &str) -> String {
    format!("---\nname: {name}\ndescription: Does a thing.\n---\n# {name}\n")
}

fn digest_of(name: &str, files: &[(&str, &[u8])]) -> String {
    let files: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
        .collect();
    let settings = coqui_aos::Settings::default();
    // Files that are no skill of that name have no digest to name: a test of them passes its own.
    coqui_aos::read_fetched_skill(name, &files, None, &settings).map_or_else(
        |_| format!("sha256:{}", "0".repeat(64)),
        |read| read.digest.to_string(),
    )
}

fn encoded(files: &[(&str, &[u8])]) -> Value {
    let mut out = serde_json::Map::new();
    for (path, bytes) in files {
        out.insert(
            (*path).to_owned(),
            Value::String(data_encoding::BASE64.encode(bytes)),
        );
    }
    Value::Object(out)
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

fn saved(control: &Control, name: &str, files: &[(&str, &[u8])], extra: Value) -> Value {
    let mut params = json!({
        "name": name,
        "source": "git+https://git.example/tools.git@main#skills/review",
        "ref": "main",
        "digest": digest_of(name, files),
        "overlays": ["team"],
        "files": encoded(files),
    });
    params
        .as_object_mut()
        .expect("object")
        .extend(extra.as_object().expect("object").clone());
    ask(control, &Caller::Owner, "skill/save", params)
}

#[test]
fn a_skill_is_fetched_from_a_source_with_its_files_and_nothing_under_an_excluded_name() {
    let (control, tmp, overlays) = host("skill-fetch");
    let upstream = repo(
        &tmp,
        "tools",
        &[
            ("skills/review/SKILL.md", &skill_text("review")),
            ("skills/review/scripts/run.sh", "echo hi\n"),
            ("skills/review/node_modules/x/index.js", "skipped\n"),
            ("skills/other/SKILL.md", &skill_text("other")),
        ],
    );
    let mut params = source(&upstream, "skills/review");
    let answer = ask(&control, &Caller::Owner, "skill/fetch", params.clone());
    let files = answer["result"]["files"].as_object().expect("files");
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        ["SKILL.md", "scripts/run.sh"],
        "{answer}"
    );
    assert_eq!(
        data_encoding::BASE64
            .decode(files["scripts/run.sh"].as_str().expect("text").as_bytes())
            .expect("base64"),
        b"echo hi\n"
    );
    assert_eq!(answer["result"]["commit"].as_str().map(str::len), Some(40));
    // What was fetched leaves nothing behind.
    let left: Vec<_> = fs::read_dir(overlays.join(".skills"))
        .map(|read| read.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");

    // A source that names no ref is the repository's head.
    params["ref"] = Value::Null;
    let headless = ask(&control, &Caller::Owner, "skill/fetch", params);
    assert!(
        headless["result"]["files"]["SKILL.md"].is_string(),
        "{headless}"
    );

    let missing = ask(
        &control,
        &Caller::Owner,
        "skill/fetch",
        source(&upstream, "skills/none"),
    );
    assert!(missing["error"].is_object(), "{missing}");
}

#[test]
fn a_skill_is_saved_listed_read_and_removed_as_the_app_keeps_it() {
    let (control, _tmp, overlays) = host("skill-keep");
    let files: [(&str, &[u8]); 2] = [
        ("SKILL.md", skill_bytes("Review")),
        ("scripts/run.sh", b"echo hi\n"),
    ];
    let done = saved(&control, "Review", &files, json!({"chosen": true}));
    assert!(done["error"].is_null(), "{done}");
    // The app names the directory and the record by the lower-case name.
    assert_eq!(
        fs::read(overlays.join(".skills/review/scripts/run.sh")).expect("file"),
        b"echo hi\n"
    );
    let record: Value =
        serde_json::from_slice(&fs::read(overlays.join(".skills/review.json")).expect("record"))
            .expect("JSON");
    assert_eq!(record["name"], "Review");
    assert_eq!(record["chosen"], true);
    assert_eq!(record["overlays"], json!(["team"]));
    assert!(control.overlays.chosen_skill("Review"));

    let listed = ask(&control, &Caller::Owner, "skill/list", json!({}));
    let copy = &listed["result"]["skills"]["review"];
    assert_eq!(copy["name"], "Review", "{listed}");
    assert_eq!(copy["digest"], digest_of("Review", &files));
    assert_eq!(copy["catalog"], Value::Null);
    assert!(
        copy.get("files").is_none(),
        "a list carries no file contents"
    );

    let read = ask(
        &control,
        &Caller::Owner,
        "skill/read",
        json!({"name": "Review"}),
    );
    assert_eq!(read["result"]["files"], encoded(&files), "{read}");

    // A new copy replaces the old whole.
    let next: [(&str, &[u8]); 1] = [("SKILL.md", skill_bytes("Review"))];
    let again = saved(&control, "Review", &next, json!({}));
    assert!(again["error"].is_null(), "{again}");
    assert!(!overlays.join(".skills/review/scripts").exists());
    assert!(!overlays.join(".skills/.old-review").exists());

    let gone = ask(
        &control,
        &Caller::Owner,
        "skill/remove",
        json!({"name": "REVIEW"}),
    );
    assert_eq!(gone["result"]["removed"], true, "{gone}");
    assert!(!overlays.join(".skills/review").exists());
    assert!(!overlays.join(".skills/review.json").exists());
    let none = ask(&control, &Caller::Owner, "skill/list", json!({}));
    assert_eq!(none["result"]["skills"], json!({}));
}

#[test]
fn a_skill_that_fails_the_checks_is_not_written() {
    let (control, _tmp, overlays) = host("skill-refused");
    let files: [(&str, &[u8]); 1] = [("SKILL.md", skill_bytes("review"))];
    let refused = |label: &str, answer: Value| {
        assert!(answer["error"].is_object(), "{label}: {answer}");
        assert!(
            !overlays.join(".skills/review").exists()
                && !overlays.join(".skills/review.json").exists(),
            "{label}: wrote something"
        );
    };
    refused(
        "wrong digest",
        saved(
            &control,
            "review",
            &files,
            json!({"digest": format!("sha256:{}", "0".repeat(64))}),
        ),
    );
    refused(
        "a name that is not the skill's",
        saved(
            &control,
            "review",
            &[("SKILL.md", skill_bytes("other"))],
            json!({"digest": digest_of("other", &[("SKILL.md", skill_bytes("other"))])}),
        ),
    );
    refused(
        "no SKILL.md",
        saved(
            &control,
            "review",
            &[("notes.md", b"x")],
            json!({"digest": digest_of("review", &files)}),
        ),
    );
    refused(
        "a path out of the folder",
        saved(
            &control,
            "review",
            &[("SKILL.md", skill_bytes("review")), ("../escape.txt", b"x")],
            json!({"digest": digest_of("review", &files)}),
        ),
    );
    refused(
        "an overlay name that is not one",
        saved(&control, "review", &files, json!({"overlays": ["../x"]})),
    );
    let bad_name = saved(&control, "../review", &files, json!({}));
    assert!(bad_name["error"].is_object(), "{bad_name}");
    assert!(!overlays.join("review").exists());
    assert!(!overlays.join(".skills/..").join("escape.txt").exists());
    let removing = ask(
        &control,
        &Caller::Owner,
        "skill/remove",
        json!({"name": ".."}),
    );
    assert!(removing["error"].is_object(), "{removing}");
}

#[test]
fn a_skill_with_more_files_than_a_skill_may_hold_is_refused() {
    let (control, _tmp, _overlays) = host("skill-big");
    let names: Vec<String> = (0..1025).map(|n| format!("f{n}.txt")).collect();
    let mut files: Vec<(&str, &[u8])> = vec![("SKILL.md", skill_bytes("review"))];
    files.extend(names.iter().map(|name| (name.as_str(), &b"x"[..])));
    let answer = ask(
        &control,
        &Caller::Owner,
        "skill/save",
        json!({
            "name": "review",
            "source": "git+https://git.example/tools.git@main",
            "digest": format!("sha256:{}", "0".repeat(64)),
            "overlays": [],
            "files": encoded(&files),
        }),
    );
    assert!(answer["error"].is_object(), "{answer}");
}

#[test]
fn a_record_that_names_another_skill_than_its_file_is_not_listed() {
    let (control, _tmp, overlays) = host("skill-record");
    let files: [(&str, &[u8]); 1] = [("SKILL.md", skill_bytes("review"))];
    assert!(saved(&control, "review", &files, json!({}))["error"].is_null());
    let record = fs::read_to_string(overlays.join(".skills/review.json")).expect("record");
    fs::write(overlays.join(".skills/other.json"), record).expect("copy");
    let listed = ask(&control, &Caller::Owner, "skill/list", json!({}));
    let names: Vec<&String> = listed["result"]["skills"]
        .as_object()
        .expect("skills")
        .keys()
        .collect();
    assert_eq!(names, ["review"], "{listed}");
}

#[test]
fn skills_follow_the_overlay_grants() {
    let (control, tmp, _overlays) = host("skill-grants");
    let text = skill_text("review");
    let upstream = repo(&tmp, "tools", &[("SKILL.md", text.as_str())]);
    let reader = key("reader", &[Grant::OverlayRead]);
    let writer = key("writer", &[Grant::OverlayWrite]);
    let files: [(&str, &[u8]); 1] = [("SKILL.md", skill_bytes("review"))];
    let save = json!({
        "name": "review",
        "source": "git+https://git.example/tools.git@main",
        "digest": digest_of("review", &files),
        "overlays": [],
        "files": encoded(&files),
    });
    let denied = |who: &Caller, method: &str, params: Value| {
        let answer = ask(&control, who, method, params);
        assert_eq!(answer["error"]["code"], -32001, "{method}: {answer}");
    };
    denied(&writer, "skill/list", json!({}));
    denied(&writer, "skill/read", json!({"name": "review"}));
    denied(&reader, "skill/fetch", source(&upstream, ""));
    denied(&reader, "skill/save", save.clone());
    denied(&reader, "skill/remove", json!({"name": "review"}));
    assert!(ask(&control, &writer, "skill/save", save)["error"].is_null());
    assert!(ask(&control, &reader, "skill/list", json!({}))["error"].is_null());
    // A source on this computer needs a key that may browse it.
    let local = ask(&control, &writer, "skill/fetch", source(&upstream, ""));
    assert_eq!(local["error"]["code"], -32002, "{local}");
}

/// The text of a skill for a test to borrow for as long as it runs.
fn skill_bytes(name: &str) -> &'static [u8] {
    Box::leak(skill_text(name).into_bytes().into_boxed_slice())
}
