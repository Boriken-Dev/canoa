//! The contract outside libraries are built against: the three kinds' description, every model's schema, and what a request
//! refuses. Regenerate the files with `CANOA_STORE_WRITE_KINDS=1 cargo test -p canoa --features store --test store_kinds`.
#![cfg(feature = "store")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::store::{
    Action, ApplyOutcome, ApplyRequest, Change, Check, Download, FetchRequest, Fetched,
    FindRequest, Found, InstallPlan, InstallPrompterVtable, InstallToolsVtable, LatestQuery,
    LatestReport, Launch, ManagedCopy, PlanRequest, Platform, ProgramCopy, ProgramQuery,
    ProgramReport, ProgramStoreVtable, Progress, Serves, StoreInfo, StoreSettings, UnpackRequest,
    Unpacked, Updatable,
};
use coqui_acp_api::guatiao::{FromValue, Map, Schema, Value};
use coqui_acp_api::{ProcessSpawnerVtable, SpawnedProcessVtable};
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
    if std::env::var_os("CANOA_STORE_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, fresh).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    // Compared as documents: another crate's `preserve_order` in the build changes the order a JSON object is written in.
    let (committed, fresh): (Json, Json) = (
        serde_json::from_str(&committed).expect("the committed file is JSON"),
        serde_json::from_str(fresh).expect("the fresh text is JSON"),
    );
    assert!(committed == fresh, "regenerate {}", path.display());
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document = format!(
        "{{\"kinds\":[{},{},{},{},{}]}}",
        ProgramStoreVtable::description(),
        InstallPrompterVtable::description(),
        InstallToolsVtable::description(),
        ProcessSpawnerVtable::description(),
        SpawnedProcessVtable::description()
    );
    committed(
        "include/canoa.store.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: Vec<(&str, Json)> = vec![
        ("StoreInfo", schema_of::<StoreInfo>()),
        ("Serves", schema_of::<Serves>()),
        ("Platform", schema_of::<Platform>()),
        ("StoreSettings", schema_of::<StoreSettings>()),
        ("ProgramQuery", schema_of::<ProgramQuery>()),
        ("ManagedCopy", schema_of::<ManagedCopy>()),
        ("Launch", schema_of::<Launch>()),
        ("ProgramReport", schema_of::<ProgramReport>()),
        ("ProgramCopy", schema_of::<ProgramCopy>()),
        ("Updatable", schema_of::<Updatable>()),
        ("LatestQuery", schema_of::<LatestQuery>()),
        ("LatestReport", schema_of::<LatestReport>()),
        ("Action", schema_of::<Action>()),
        ("PlanRequest", schema_of::<PlanRequest>()),
        ("InstallPlan", schema_of::<InstallPlan>()),
        ("Download", schema_of::<Download>()),
        ("Check", schema_of::<Check>()),
        ("Change", schema_of::<Change>()),
        ("ApplyRequest", schema_of::<ApplyRequest>()),
        ("ApplyOutcome", schema_of::<ApplyOutcome>()),
        ("Progress", schema_of::<Progress>()),
        ("FindRequest", schema_of::<FindRequest>()),
        ("Found", schema_of::<Found>()),
        ("FetchRequest", schema_of::<FetchRequest>()),
        ("Fetched", schema_of::<Fetched>()),
        ("UnpackRequest", schema_of::<UnpackRequest>()),
        ("Unpacked", schema_of::<Unpacked>()),
    ];
    let document: serde_json::Map<String, Json> = schemas
        .into_iter()
        .map(|(n, s)| (n.to_owned(), s))
        .collect();
    committed(
        "include/canoa.store.schemas.json",
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

/// What a host sends refuses a key it does not declare, so a field a newer host adds is never dropped silently by an older
/// store.
#[test]
fn a_request_with_a_key_it_does_not_declare_is_refused() {
    let unknown = ("surprise", Value::from(1_i64));
    assert!(LatestQuery::from_value(&map(std::slice::from_ref(&unknown))).is_err());
    let names = coqui_acp_api::guatiao::ToValue::to_value(&Vec::<String>::new()).expect("a list");
    assert!(FindRequest::from_value(&map(&[("names", names.clone())])).is_ok());
    assert!(FindRequest::from_value(&map(&[("names", names), unknown.clone()])).is_err());
    assert!(StoreSettings::from_value(&map(std::slice::from_ref(&unknown))).is_err());
    assert!(ProgramQuery::from_value(&map(&[unknown])).is_err());
}

#[test]
fn an_answer_is_read_leniently_and_a_missing_optional_is_left_out() {
    let report = LatestReport::from_value(&map(&[
        ("known", Value::from(true)),
        ("version", Value::from("1.2.3")),
        ("later_field", Value::from(1_i64)),
    ]))
    .expect("a newer writer's extra key is dropped");
    assert_eq!(report.version.as_deref(), Some("1.2.3"));
    let schema = schema_of::<LatestReport>().to_string();
    assert!(schema.contains("unknown_because"));
}

#[test]
fn the_models_round_trip_through_a_value() {
    use coqui_acp_api::guatiao::ToValue;
    let plan = {
        let mut plan = InstallPlan::new(Action::Install, Some("1.0.0".to_owned()));
        plan.downloads.push(Download::new(
            "https://example.test/x",
            Some(3),
            Check::Sha256 {
                value: "ab".repeat(32),
            },
        ));
        plan.changes.push(Change::new(
            canoa::store::USER_PATH,
            "PATH",
            "adds a directory",
            true,
        ));
        plan.token = "t".to_owned();
        plan
    };
    let value = plan.to_value().expect("a value");
    assert_eq!(InstallPlan::from_value(&value).expect("read back"), plan);
    let check = Check::Unverified {
        why: "no checksum".to_owned(),
    };
    assert_eq!(
        Check::from_value(&check.to_value().expect("v")).expect("c"),
        check
    );
}

#[test]
fn a_program_folder_name_is_one_to_sixty_four_of_lowercase_digits_and_hyphens() {
    assert!(canoa::store::is_program_name("claude"));
    assert!(canoa::store::is_program_name("a-1"));
    for bad in ["", "A", "a b", "a/b", "..", &"a".repeat(65)] {
        assert!(!canoa::store::is_program_name(bad), "{bad:?}");
    }
}
