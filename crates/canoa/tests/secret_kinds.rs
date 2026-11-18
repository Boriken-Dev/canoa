//! The `secret_provider` kind: the contract outside providers, scripts included, are built against.
//! Regenerate the description files with `CANOA_SECRET_WRITE_KINDS=1 cargo test -p canoa --features secret --test secret_kinds`,
//! then `python python/write_secret_kinds.py` with Guatiao's Python binding on `PYTHONPATH`.
#![cfg(feature = "secret")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::secret_provider::{
    Held, Keeping, Named, Names, ProviderInfo, SecretProvider, SecretProviderVtable,
};
use guatiao::{FromValue, Map, Schema, ToValue, Value};
use guatiao_serde::Serializable;
use serde_json::Value as Json;

fn file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn pretty(value: &Json) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("written");
    text.push('\n');
    text
}

fn committed(path: &str, fresh: &str) {
    let path = file(path);
    if std::env::var_os("CANOA_SECRET_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, fresh).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    let (committed, fresh): (Json, Json) = (
        serde_json::from_str(&committed).expect("the committed file is JSON"),
        serde_json::from_str(fresh).expect("the fresh text is JSON"),
    );
    assert!(committed == fresh, "regenerate {}", path.display());
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document = format!("{{\"kinds\":[{}]}}", SecretProviderVtable::description());
    committed(
        "include/canoa.secret.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: Vec<(&str, Json)> = vec![
        ("ProviderInfo", schema_of::<ProviderInfo>()),
        ("Named", schema_of::<Named>()),
        ("Keeping", schema_of::<Keeping>()),
        ("Held", schema_of::<Held>()),
        ("Names", schema_of::<Names>()),
    ];
    let document: serde_json::Map<String, Json> = schemas
        .into_iter()
        .map(|(n, s)| (n.to_owned(), s))
        .collect();
    committed(
        "include/canoa.secret.schemas.json",
        &pretty(&Json::Object(document)),
    );
}

fn map(pairs: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.set(key, value.clone());
    }
    Value::from(map)
}

/// A script builds these from plain data: a description with only a title, and an answer with only a value.
#[test]
fn a_script_can_answer_with_the_few_keys_it_has() {
    let info = ProviderInfo::from_value(&map(&[("title", Value::from("Vault"))]))
        .expect("a title is all a description needs");
    assert!(!info.can_store && !info.can_list && !info.needs_person);
    let held = Held::from_value(&map(&[("value", Value::from("v"))])).expect("a value");
    assert_eq!(held.value.as_ref().map(|v| v.expose().as_str()), Some("v"));
    assert_eq!(held.expires_in, None);
    let expiring = Held::from_value(&map(&[
        ("value", Value::from("v")),
        ("expires_in", Value::from(30_i64)),
    ]))
    .expect("a value that ends");
    assert_eq!(expiring.expires_in, Some(30));
    assert!(
        Held::from_value(&map(&[]))
            .expect("nothing held")
            .value
            .is_none()
    );
}

/// What a host sends refuses a key it does not declare, so a field a newer host adds is never dropped silently by an older
/// provider.
#[test]
fn a_request_with_a_key_it_does_not_declare_is_refused() {
    assert!(Named::from_value(&map(&[("name", Value::from("n"))])).is_ok());
    assert!(
        Named::from_value(&map(&[
            ("name", Value::from("n")),
            ("surprise", Value::from(1_i64))
        ]))
        .is_err()
    );
    assert!(
        Keeping::from_value(&map(&[
            ("name", Value::from("n")),
            ("value", Value::from("v")),
            ("surprise", Value::from(1_i64))
        ]))
        .is_err()
    );
}

#[test]
fn a_value_round_trips_through_a_guatiao_value_and_is_not_printed() {
    let keeping = Keeping::new("n", "s3cret-text");
    let value = keeping.to_value().expect("a value");
    let back = Keeping::from_value(&value).expect("read again");
    assert_eq!(back.value.expose().as_str(), "s3cret-text");
    assert!(!format!("{back:?}").contains("s3cret"));
}

#[test]
fn the_kind_is_the_five_calls_and_no_listing_of_values() {
    let document: Json = serde_json::from_str(&SecretProviderVtable::description()).expect("JSON");
    let methods: Vec<&str> = document["methods"]
        .as_array()
        .or_else(|| document["slots"].as_array())
        .expect("the kind lists its methods")
        .iter()
        .filter_map(|method| method["name"].as_str())
        .collect();
    assert_eq!(methods, ["describe", "names", "get", "put", "forget"]);
}

/// A provider written in Rust implements the trait and nothing else is needed.
struct Fixed;

impl SecretProvider for Fixed {
    fn describe(&self) -> Result<ProviderInfo, guatiao::library::ProviderError> {
        Ok(ProviderInfo::new("Fixed"))
    }
    fn names(&self) -> Result<Names, guatiao::library::ProviderError> {
        Err(canoa::secret_provider::unsupported("it lists nothing"))
    }
    fn get(&self, _: Named) -> Result<Held, guatiao::library::ProviderError> {
        Ok(Held::none())
    }
    fn put(&self, _: Keeping) -> Result<(), guatiao::library::ProviderError> {
        Err(canoa::secret_provider::unsupported("it keeps nothing"))
    }
    fn forget(&self, _: Named) -> Result<(), guatiao::library::ProviderError> {
        Err(canoa::secret_provider::unsupported("it keeps nothing"))
    }
}

#[test]
fn a_provider_that_cannot_do_a_call_says_so_with_unsupported() {
    let fixed = Fixed;
    let error = fixed.put(Keeping::new("n", "v")).expect_err("refused");
    assert_eq!(error.status, guatiao::Status::GUATIAO_ERR_UNSUPPORTED);
}
