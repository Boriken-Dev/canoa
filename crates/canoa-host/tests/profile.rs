//! The profile vectors the app is held to as well: each merge's view or first refusal, each overlay source, and each
//! AOS overlay's layer.
#![allow(
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use canoa_host::profile::{Layer, LayerId, LayerKind, OverlaySource, Profile, ProfileError, merge};
use serde_json::Value;

fn vectors() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/profile.json"
    ))
    .expect("the vectors are readable");
    serde_json::from_str(&text).expect("the vectors are JSON")
}

fn kind(name: &str) -> LayerKind {
    match name {
        "built-in" => LayerKind::BuiltIn,
        "overlay" => LayerKind::Overlay,
        "user" => LayerKind::User,
        "project" => LayerKind::Project,
        "agent" => LayerKind::Agent,
        "session" => LayerKind::Session,
        other => panic!("no layer kind {other}"),
    }
}

fn layers(case: &Value) -> Result<Vec<Layer>, (usize, ProfileError)> {
    let mut out = Vec::new();
    for (index, layer) in case["layers"]
        .as_array()
        .expect("layers")
        .iter()
        .enumerate()
    {
        let kind = kind(layer["kind"].as_str().expect("kind"));
        let profile = Profile::from_str(layer["toml"].as_str().expect("toml"), kind)
            .map_err(|error| (index, error))?;
        let name = layer["name"].as_str().unwrap_or("");
        out.push(Layer::new(LayerId::new(kind, name), None, profile));
    }
    Ok(out)
}

#[test]
fn every_merge_vector_gives_its_view_or_its_refusal() {
    let vectors = vectors();
    let cases = vectors["merges"].as_array().expect("merges");
    assert!(cases.len() > 40, "the vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        match (layers(case), case.get("effective"), case.get("error")) {
            (Ok(layers), Some(expected), None) => {
                assert_eq!(&merge(&layers).view(), expected, "{name}");
            }
            (Err((index, error)), None, Some(expected)) => {
                assert_eq!(expected["layer"], index, "{name}: the layer refused");
                assert_eq!(expected["code"], error.code(), "{name}: {error}");
                if let ProfileError::Invalid { key, .. } = &error {
                    assert_eq!(expected["key"], key.as_str(), "{name}");
                }
            }
            (Ok(_), _, Some(_)) => panic!("{name}: accepted, and the vector refuses it"),
            (Err((_, error)), Some(_), _) => {
                panic!("{name}: refused ({error}), and the vector takes it")
            }
            _ => panic!("{name}: a vector has either effective or error"),
        }
    }
}

#[test]
fn every_source_vector_parses_or_is_refused() {
    let vectors = vectors();
    for case in vectors["sources"].as_array().expect("sources") {
        let source = case["source"].as_str().expect("source");
        let parsed = OverlaySource::parse_git(source);
        if case.get("refused").is_some() {
            assert!(parsed.is_err(), "{source} is refused");
            continue;
        }
        let Ok(OverlaySource::Git {
            repo,
            reference,
            path,
        }) = parsed
        else {
            panic!("{source} parses as git");
        };
        assert_eq!(case["repo"], repo.as_str(), "{source}");
        assert_eq!(case["ref"], reference.as_str(), "{source}");
        assert_eq!(case["path"], path.as_str(), "{source}");
    }
}

/// The overlays of a case as layers, in composition order, and the refusals; a manifest's `requires` and `priority`
/// order them as `canoa_host::aos` composes them.
fn overlay_layers(case: &Value) -> (Vec<Layer>, Vec<Value>) {
    use canoa_host::aos::{self, EntryKind, TreeEntry};
    use canoa_host::profile::{OverlayInput, overlay_layer};
    let os = case["os"].as_str().expect("os");
    let mut layers = std::collections::BTreeMap::new();
    let mut manifests = std::collections::BTreeMap::new();
    let mut refused = Vec::new();
    for overlay in case["overlays"].as_array().expect("overlays") {
        let identity = overlay["identity"].as_str().expect("identity");
        let tree: Vec<TreeEntry> = overlay["files"]
            .as_object()
            .expect("files")
            .iter()
            .map(|(path, text)| TreeEntry {
                components: path
                    .split('/')
                    .map(|name| name.as_bytes().to_vec())
                    .collect(),
                kind: EntryKind::File,
                data: text.as_str().expect("text").as_bytes().to_vec(),
            })
            .collect();
        let mut input =
            OverlayInput::new(identity, overlay["root"].as_str().expect("root"), os, &tree);
        input.harness = case["harness"].as_str();
        input.pinned = overlay["pinned"] == true;
        match overlay_layer(&input) {
            Ok(layer) => {
                let manifest = layer.aos.as_ref().expect("an AOS layer").manifest.clone();
                manifests.insert(aos::normalize_name(identity), manifest);
                layers.insert(aos::normalize_name(identity), layer);
            }
            Err(refusal) => refused.push(serde_json::json!({
                "identity": identity, "code": refusal.code, "subject": refusal.subject
            })),
        }
    }
    let requested: Vec<String> = manifests.keys().cloned().collect();
    let (visited, _) = aos::resolve(&manifests, &requested).expect("no cycle");
    let order = aos::composition_order(&manifests, &visited);
    let mut out: Vec<Layer> = order
        .iter()
        .filter_map(|identity| layers.remove(identity))
        .collect();
    if let Some(user) = case["user"].as_str() {
        let profile = Profile::from_str(user, LayerKind::User).expect("the user layer parses");
        out.push(Layer::new(LayerId::new(LayerKind::User, ""), None, profile));
    }
    (out, refused)
}

#[test]
fn every_overlay_vector_gives_its_view_refusals_and_environment() {
    let vectors = vectors();
    let cases = vectors["overlays"].as_array().expect("overlays");
    assert!(cases.len() >= 6, "the overlay vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let (layers, refused) = overlay_layers(case);
        assert_eq!(
            Value::Array(refused),
            case["refused"].clone(),
            "{name}: refused"
        );
        let effective = merge(&layers);
        assert_eq!(effective.view(), case["view"], "{name}: view");
        let host = case["host"].as_object().cloned().unwrap_or_default();
        let lookup = |variable: &str| {
            host.get(variable)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        match (effective.environment(lookup), case.get("environment")) {
            (Ok(found), Some(expected)) => {
                let found: serde_json::Map<String, Value> = found
                    .into_iter()
                    .map(|(variable, value)| (variable, Value::String(value.expose().to_owned())))
                    .collect();
                assert_eq!(&Value::Object(found), expected, "{name}: environment");
            }
            (Err(error), None) => {
                let expected = &case["environment_error"];
                assert_eq!(expected["code"], error.code(), "{name}: {error}");
                if let ProfileError::Resolve { key, .. } = &error {
                    assert_eq!(expected["key"], key.as_str(), "{name}");
                }
            }
            (Ok(_), None) => panic!("{name}: the environment builds, and the vector refuses it"),
            (Err(error), Some(_)) => panic!("{name}: the environment is refused ({error})"),
        }
    }
}

#[test]
fn the_reserved_variables_are_the_vectors_list() {
    use canoa_host::profile::{RESERVED_ENV, RESERVED_ENV_PREFIXES, RESERVED_ENV_SUFFIXES};
    let vectors = vectors();
    let list = |key: &str| -> Vec<String> {
        vectors["reserved_env"][key]
            .as_array()
            .expect("a list")
            .iter()
            .map(|name| name.as_str().expect("text").to_owned())
            .collect()
    };
    assert_eq!(list("names"), RESERVED_ENV);
    assert_eq!(list("prefixes"), RESERVED_ENV_PREFIXES);
    assert_eq!(list("suffixes"), RESERVED_ENV_SUFFIXES);
}
