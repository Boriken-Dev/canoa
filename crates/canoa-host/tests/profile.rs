//! The profile vectors the app is held to as well: each merge's view or first refusal, and each overlay source.

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
