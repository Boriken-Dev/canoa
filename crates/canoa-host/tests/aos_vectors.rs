//! The Agent Overlay Specification's conformance vectors, vendored in `tests/vectors/aos/` (`AOS-SOURCE.json` names the
//! revision). The app runs the same files (`app/test/aos_vectors_test.dart`). A vector Canoa does not apply is skipped by
//! name, and every test prints what it skipped.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::{BTreeMap, BTreeSet};

use canoa_host::aos::{
    self, AosError, EntryKind, Environment, Facts, Limits, Overlay, SourceKind, TreeEntry,
};
use serde_json::{Map, Value, json};

fn vectors(name: &str) -> Vec<Value> {
    let path = format!(
        "{}/tests/vectors/aos/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).expect("the vectors are readable");
    let file: Value = serde_json::from_str(&text).expect("the vectors are JSON");
    file["cases"].as_array().expect("cases").clone()
}

fn id(case: &Value) -> &str {
    case["id"].as_str().expect("an id")
}

/// An error class matches the vector's, which may list two.
fn class_matches(expected: &Value, code: &str) -> bool {
    match expected {
        Value::String(class) => class == code,
        Value::Array(classes) => classes.iter().any(|class| class == code),
        _ => false,
    }
}

fn report(file: &str, ran: usize, skipped: &[(String, &str)]) {
    println!("{file}: {ran} passed, {} skipped", skipped.len());
    for (id, why) in skipped {
        println!("  skipped {id}: {why}");
    }
}

fn bytes_of(case: &Value) -> Vec<u8> {
    if let Some(text) = case["text"].as_str() {
        return text.as_bytes().to_vec();
    }
    base64(case["base64"].as_str().expect("text or base64"))
}

fn base64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        let value = ALPHABET
            .iter()
            .position(|letter| *letter == byte)
            .expect("base64") as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().expect("a string").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn environment(values: &Value, windows: bool, secret: &[String]) -> Environment<'static> {
    let mut env = Environment::new(windows);
    for (name, value) in values.as_object().into_iter().flatten() {
        env.set(
            name,
            value.as_str().expect("a string"),
            secret.contains(name),
        );
    }
    env
}

fn env_json(env: &Environment) -> Value {
    Value::Object(
        env.values()
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect(),
    )
}

#[test]
fn reading() {
    let mut ran = 0;
    let mut skipped = Vec::new();
    for case in vectors("reading") {
        let id = id(&case).to_owned();
        let bytes = bytes_of(&case);
        let format = case["format"].as_str().expect("format");
        let text = String::from_utf8_lossy(&bytes);
        let yaml = text
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .is_some_and(|line| {
                line.starts_with("#!") && {
                    let word = line[2..].split_whitespace().last().unwrap_or("");
                    let word = word.rsplit(['/', '\\']).next().unwrap_or("");
                    matches!(
                        word.trim_start_matches('.').to_ascii_lowercase().as_str(),
                        "yaml" | "yml"
                    )
                }
            });
        let expect = &case["expect"];
        let result: Result<Value, AosError> = match format {
            "env-script" => {
                skipped.push((
                    id,
                    "Canoa runs no env script, so reads no env script output",
                ));
                continue;
            }
            "catalog" => {
                skipped.push((id, "a catalog, which canoa-host does not read"));
                continue;
            }
            "aos" if yaml => {
                // Canoa has no YAML reader: it refuses the format by name, as the specification lets it.
                let refused = aos::read_document(&bytes, Limits::default());
                assert_eq!(
                    refused.map_err(|error| error.code),
                    Err("unsupported-format"),
                    "{id}: YAML is refused"
                );
                skipped.push((id, "YAML, which Canoa refuses as an unsupported format"));
                continue;
            }
            "aos" => {
                let mut limits = Limits::default();
                if let Some(max) = case["limits"]["max_depth"].as_u64() {
                    limits.max_depth = max as usize;
                }
                if let Some(max) = case["limits"]["max_bytes"].as_u64() {
                    limits.max_bytes = max as usize;
                }
                aos::read_document(&bytes, limits).map(Value::Object)
            }
            "dotenv" => {
                let windows = case["os"] == "windows";
                let secret = strings(&case["secret_variables"]);
                let mut env = environment(&case["env"], windows, &secret);
                aos::read_dotenv(&bytes, &mut env).map(|set| {
                    Value::Object(
                        set.into_iter()
                            .map(|(name, value)| (name, json!(value)))
                            .collect(),
                    )
                })
            }
            other => panic!("{id}: no format {other}"),
        };
        match result {
            Ok(data) => {
                let expected = expect
                    .get("data")
                    .unwrap_or_else(|| panic!("{id}: read, and the vector refuses it"));
                assert_eq!(&data, expected, "{id}");
            }
            Err(error) => assert!(
                class_matches(&expect["error"], error.code),
                "{id}: refused as {}, and the vector expects {}",
                error.code,
                expect
            ),
        }
        ran += 1;
    }
    report("reading", ran, &skipped);
}

#[test]
fn validation() {
    let mut ran = 0;
    for case in vectors("validation") {
        let id = id(&case);
        let document = case["document"].as_object().expect("a mapping");
        let files: Option<BTreeSet<String>> = case
            .get("files")
            .map(|files| strings(files).into_iter().collect());
        let issues = aos::validate(document, files.as_ref());
        if let Some(applies) = case["expect"]["applies"].as_bool() {
            assert_eq!(
                aos::applies(&issues),
                applies,
                "{id}: applies, found {issues:?}"
            );
        }
        if case["expect"]["valid"] == true {
            assert!(issues.is_empty(), "{id}: valid, found {issues:?}");
        } else {
            let at = case["expect"]["at"].as_str().expect("at");
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.at == at || issue.at.starts_with(&format!("{at}/"))),
                "{id}: a problem at {at}, found {issues:?}"
            );
        }
        ran += 1;
    }
    report("validation", ran, &[]);
}

#[test]
fn catalogs_are_not_read_by_canoa_host() {
    let cases = vectors("catalog");
    let skipped: Vec<(String, &str)> = cases
        .iter()
        .map(|case| {
            (
                id(case).to_owned(),
                "catalogs and icons: canoa-host is no store, and reads neither",
            )
        })
        .collect();
    report("catalog", 0, &skipped);
}

#[test]
fn names() {
    let mut ran = 0;
    for case in vectors("names") {
        let name = case["name"].as_str().expect("name");
        let expect = &case["expect"];
        assert_eq!(
            aos::valid_overlay_name(name),
            expect["valid"] == true,
            "{name}"
        );
        if expect["valid"] == true {
            assert_eq!(aos::normalize_name(name), expect["normalized"], "{name}");
            assert_eq!(aos::root_variable(name), expect["variable"], "{name}");
        }
        ran += 1;
    }
    report("names", ran, &[]);
}

#[test]
fn sources() {
    let mut ran = 0;
    for case in vectors("source") {
        let id = id(&case);
        let expect = &case["expect"];
        match aos::parse_source(case["source"].as_str().expect("source")) {
            Ok(source) => {
                let kind = match source.kind {
                    SourceKind::Git => "git",
                    SourceKind::Url => "url",
                    SourceKind::Directory => "directory",
                    _ => "unknown",
                };
                assert_eq!(
                    json!({"kind": kind, "location": source.location, "ref": source.reference, "path": source.path}),
                    *expect,
                    "{id}"
                );
            }
            Err(error) => assert!(
                class_matches(&expect["error"], error.code),
                "{id}: refused as {}, and the vector expects {expect}",
                error.code
            ),
        }
        ran += 1;
    }
    report("source", ran, &[]);
}

#[test]
fn digest() {
    let mut ran = 0;
    for case in vectors("digest") {
        let id = id(&case);
        let entries: Vec<TreeEntry> = case["tree"]
            .as_array()
            .expect("tree")
            .iter()
            .map(|entry| {
                let path = entry["path"]
                    .as_str()
                    .map(|path| path.as_bytes().to_vec())
                    .unwrap_or_else(|| base64(entry["path_base64"].as_str().expect("a path")));
                let components = path
                    .split(|byte| *byte == b'/')
                    .map(<[u8]>::to_vec)
                    .collect();
                let kind = match entry["type"].as_str() {
                    None | Some("file") => EntryKind::File,
                    Some("dir") => EntryKind::Dir,
                    Some("symlink") => EntryKind::Symlink,
                    Some("special") => EntryKind::Special,
                    Some(other) => panic!("{id}: no entry type {other}"),
                };
                let data = if entry.get("text").is_some() || entry.get("base64").is_some() {
                    bytes_of(entry)
                } else {
                    Vec::new()
                };
                TreeEntry {
                    components,
                    kind,
                    data,
                }
            })
            .collect();
        let expect = &case["expect"];
        match aos::digest_entries(&entries) {
            Ok(tree) => {
                assert_eq!(tree.digest, expect["digest"], "{id}");
                let lines: Vec<Value> = tree
                    .lines
                    .iter()
                    .map(|line| json!(String::from_utf8(line.clone()).expect("UTF-8")))
                    .collect();
                assert_eq!(Value::Array(lines), expect["lines"], "{id}");
            }
            Err(error) => assert!(
                class_matches(&expect["error"], error.code),
                "{id}: refused as {}, and the vector expects {expect}",
                error.code
            ),
        }
        ran += 1;
    }
    report("digest", ran, &[]);
}

/// The files an overlay holds, and the text of those read as env files: a vector lists paths, or maps each to its text.
fn tree(files: &Value) -> (BTreeSet<String>, BTreeMap<String, String>) {
    match files {
        Value::Object(texts) => (
            texts.keys().cloned().collect(),
            texts
                .iter()
                .map(|(path, text)| (path.clone(), text.as_str().expect("text").to_owned()))
                .collect(),
        ),
        Value::Array(_) => (strings(files).into_iter().collect(), BTreeMap::new()),
        _ => (BTreeSet::new(), BTreeMap::new()),
    }
}

fn facts(value: &Value) -> Facts {
    let grants = value["grants"]
        .as_object()
        .map(|grants| {
            grants
                .iter()
                .map(|(provider, keys)| (provider.clone(), strings(keys)))
                .collect()
        })
        .unwrap_or_default();
    let map = |key: &str| -> BTreeMap<String, String> {
        value[key]
            .as_object()
            .map(|values| {
                values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_str().expect("text").to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut facts = Facts::new(value["os"].as_str().expect("os"));
    facts.grants = grants;
    facts.host_env = map("host_env");
    facts.secrets = map("secrets");
    facts.secret_files = value["secret_files"]
        .is_object()
        .then(|| map("secret_files"));
    facts.providers = value
        .get("providers")
        .map_or_else(|| vec!["env".to_owned(), "file".to_owned()], strings);
    facts
}

/// The weakenings Canoa reports: those of what it applies, never of setup, env scripts or dependencies it does not run.
const APPLIED_WEAKENINGS: &[&str] = &[
    "unpinned",
    "env-sensitive",
    "env-passthrough",
    "secret-read",
    "insecure-transport",
];

fn weakenings(list: &[aos::Weakening]) -> Value {
    let set: BTreeSet<(String, String)> = list
        .iter()
        .filter(|weakening| APPLIED_WEAKENINGS.contains(&weakening.id))
        .map(|weakening| (weakening.id.to_owned(), weakening.subject.clone()))
        .collect();
    Value::Array(
        set.into_iter()
            .map(|(id, subject)| json!([id, subject]))
            .collect(),
    )
}

#[test]
fn compose() {
    let mut ran = 0;
    let mut skipped = Vec::new();
    for case in vectors("compose") {
        let id = id(&case).to_owned();
        let expect = &case["expect"];
        if expect.get("environments").is_some() {
            skipped.push((
                id,
                "dependency environments: Canoa installs no dependencies",
            ));
            continue;
        }
        let manifests: BTreeMap<String, Map<String, Value>> = case["overlays"]
            .as_object()
            .expect("overlays")
            .iter()
            .map(|(name, manifest)| {
                (
                    name.clone(),
                    manifest.as_object().expect("a mapping").clone(),
                )
            })
            .collect();
        let requested = strings(&case["requested"]);
        let resolved = aos::resolve(&manifests, &requested);
        let (visited, missing) = match resolved {
            Err(cycle) => {
                assert_eq!(expect["error"], "cycle", "{id}");
                assert_eq!(json!(cycle), expect["chain"], "{id}");
                ran += 1;
                continue;
            }
            Ok(found) => found,
        };
        let active = aos::composition_order(&manifests, &visited);
        assert_eq!(json!(active), expect["active"], "{id}: active");
        if let Some(order) = expect.get("install_order") {
            assert_eq!(&json!(visited), order, "{id}: install order");
            let missing: Vec<Value> = missing.iter().map(|(by, name)| json!([by, name])).collect();
            assert_eq!(Value::Array(missing), expect["missing"], "{id}: missing");
            let roots: Map<String, Value> = active
                .iter()
                .map(|identity| (identity.clone(), json!(aos::root_variable(identity))))
                .collect();
            assert_eq!(Value::Object(roots), expect["root_variables"], "{id}");
            let files = |identity: &str| tree(&case["files"][identity]).0;
            let routing: Vec<Value> = active
                .iter()
                .flat_map(|identity| {
                    manifests[identity]
                        .get("routing")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            assert_eq!(Value::Array(routing), expect["routing"], "{id}: routing");
            let each = |of: fn(&Map<String, Value>, &BTreeSet<String>) -> Vec<String>| -> Value {
                Value::Array(
                    active
                        .iter()
                        .flat_map(|identity| {
                            of(&manifests[identity], &files(identity))
                                .into_iter()
                                .map(move |path| json!([identity, path]))
                        })
                        .collect(),
                )
            };
            assert_eq!(each(aos::rules_of), expect["rules"], "{id}: rules");
            assert_eq!(each(aos::context_of), expect["context"], "{id}: context");
            let mut winners: BTreeMap<String, (String, String)> = BTreeMap::new();
            let mut collisions = BTreeSet::new();
            for identity in &active {
                for name in aos::skills_of(&manifests[identity], &files(identity)) {
                    let folded = name.to_ascii_lowercase();
                    if winners.contains_key(&folded) {
                        collisions.insert(folded.clone());
                    }
                    winners.insert(folded, (name, identity.clone()));
                }
            }
            let skills: Map<String, Value> = winners
                .into_values()
                .map(|(name, identity)| (name, json!(identity)))
                .collect();
            assert_eq!(Value::Object(skills), expect["skills"], "{id}: skills");
            assert_eq!(json!(collisions), expect["skill_collisions"], "{id}");
        }
        if let Some(expected) = expect.get("env") {
            let facts_value = &case["facts"];
            let facts = facts(facts_value);
            let windows = facts.os == "windows";
            let mut env = environment(&facts_value["base_env"], windows, &[]);
            for identity in &active {
                env.set(
                    &aos::root_variable(identity),
                    facts_value["roots"][identity].as_str().expect("a root"),
                    false,
                );
            }
            let mut found = Vec::new();
            let mut outcome = Ok(());
            for identity in &active {
                let (files, texts) = tree(&case["files"][identity]);
                let overlay = Overlay {
                    identity,
                    manifest: &manifests[identity],
                    files: &files,
                    root: facts_value["roots"][identity].as_str().expect("a root"),
                    state: None,
                    read: &|path: &str| texts.get(path).map(|text| text.as_bytes().to_vec()),
                };
                outcome = aos::apply_overlay(&mut env, &overlay, &facts, &mut found);
                if outcome.is_err() {
                    break;
                }
            }
            match outcome {
                Ok(()) => assert_eq!(&env_json(&env), expected, "{id}: env"),
                Err(error) => assert_eq!(
                    json!({"error": error.code, "subject": error.subject}),
                    *expected,
                    "{id}: env"
                ),
            }
        }
        ran += 1;
    }
    report("compose", ran, &skipped);
}

/// The error classes of applying an environment, which Canoa reports; the others come from setup or dependencies.
const ENV_ERRORS: &[&str] = &[
    "unresolved-reference",
    "invalid-reference",
    "missing-file",
    "invalid-env-file",
    "env-path-conflict",
    "reserved-variable",
    "undefined-variable",
    "secret-placeholder",
    "invalid-pattern",
];

#[test]
fn effective() {
    let mut ran = 0;
    let mut skipped = Vec::new();
    for case in vectors("effective") {
        let id = id(&case).to_owned();
        let expect = &case["expect"];
        let facts_value = &case["facts"];
        if facts_value["packages"]
            .as_object()
            .is_some_and(|packages| !packages.is_empty())
            && expect["dependencies"]
                .as_array()
                .is_some_and(|found| !found.is_empty())
        {
            skipped.push((
                id,
                "installed packages: Canoa installs no dependencies, so adds no package directory",
            ));
            continue;
        }
        if let Some(code) = expect["error"].as_str()
            && !ENV_ERRORS.contains(&code)
        {
            skipped.push((
                id,
                "a refusal of setup or dependencies, which Canoa does not run",
            ));
            continue;
        }
        let manifest = case["manifest"].as_object().expect("a manifest");
        let facts = facts(facts_value);
        let (files, texts) = tree(&facts_value["files"]);
        let identity = facts_value["identity"].as_str().expect("identity");
        let root = facts_value["root"].as_str().expect("root");
        let state = facts_value["state"].as_str();
        let mut env = environment(&facts_value["base_env"], facts.os == "windows", &[]);
        env.set(&aos::root_variable(identity), root, false);
        if let Some(state) = state {
            env.set(&aos::state_variable(identity), state, false);
        }
        let overlay = Overlay {
            identity,
            manifest,
            files: &files,
            root,
            state,
            read: &|path: &str| texts.get(path).map(|text| text.as_bytes().to_vec()),
        };
        let mut found = Vec::new();
        if facts_value["pinned"] != true {
            found.push(aos::Weakening::new("unpinned", ""));
        }
        match aos::apply_overlay(&mut env, &overlay, &facts, &mut found) {
            Err(error) => {
                assert_eq!(
                    json!({"error": error.code, "subject": error.subject}),
                    json!({"error": expect["error"], "subject": expect["subject"]}),
                    "{id}"
                );
            }
            Ok(()) => {
                assert!(
                    expect.get("error").is_none(),
                    "{id}: applied, and the vector refuses it"
                );
                let ungranted = aos::grants_report(manifest, &facts, &mut found);
                assert_eq!(env_json(&env), expect["env"], "{id}: env");
                assert_eq!(json!(ungranted), expect["ungranted"], "{id}: ungranted");
                let expected: Vec<aos::Weakening> = expect["weakenings"]
                    .as_array()
                    .expect("weakenings")
                    .iter()
                    .filter_map(|pair| {
                        let id = pair[0].as_str()?;
                        let id = APPLIED_WEAKENINGS.iter().find(|known| **known == id)?;
                        Some(aos::Weakening::new(id, pair[1].as_str()?))
                    })
                    .collect();
                assert_eq!(
                    weakenings(&found),
                    weakenings(&expected),
                    "{id}: weakenings"
                );
            }
        }
        ran += 1;
    }
    report("effective", ran, &skipped);
}
