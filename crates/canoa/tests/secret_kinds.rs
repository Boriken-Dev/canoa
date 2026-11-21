//! The `secret_provider` kind: the contract outside providers, scripts included, are built against. Its description files are
//! checked with the other kinds' by `script_kinds.rs`.
#![cfg(feature = "secret")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use canoa::secret_provider::{
    Held, Keeping, Named, Names, ProviderInfo, SecretProvider, SecretProviderVtable,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_harness::LocalSpawner;
use guatiao::library::Object;
use guatiao::{FromValue, Map, ToValue, Value};
use serde_json::Value as Json;

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
    let spawned = |name: &str| {
        document["slots"]
            .as_array()
            .and_then(|slots| slots.iter().find(|slot| slot["name"] == name))
            .map(|slot| slot["args"].to_string().contains("process_spawner"))
    };
    // Every call that may run a program is handed the host's spawner; `describe` runs nothing.
    assert_eq!(spawned("describe"), Some(false));
    for call in ["names", "get", "put", "forget"] {
        assert_eq!(spawned(call), Some(true), "{call}");
    }
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
    fn names(
        &self,
        _: Object<dyn ProcessSpawner>,
    ) -> Result<Names, guatiao::library::ProviderError> {
        Err(canoa::status::unsupported("it lists nothing"))
    }
    fn get(
        &self,
        _: Named,
        _: Object<dyn ProcessSpawner>,
    ) -> Result<Held, guatiao::library::ProviderError> {
        Ok(Held::none())
    }
    fn put(
        &self,
        _: Keeping,
        _: Object<dyn ProcessSpawner>,
    ) -> Result<(), guatiao::library::ProviderError> {
        Err(canoa::status::unsupported("it keeps nothing"))
    }
    fn forget(
        &self,
        _: Named,
        _: Object<dyn ProcessSpawner>,
    ) -> Result<(), guatiao::library::ProviderError> {
        Err(canoa::status::unsupported("it keeps nothing"))
    }
}

#[test]
fn a_provider_that_cannot_do_a_call_says_so_with_unsupported() {
    let fixed = Fixed;
    let error = fixed
        .put(Keeping::new("n", "v"), LocalSpawner::new().into_object())
        .expect_err("refused");
    assert_eq!(error.status, guatiao::Status::GUATIAO_ERR_UNSUPPORTED);
}
