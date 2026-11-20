//! The `account_speech` kind: the contract outside providers are built against.
//! Regenerate the description files with `CANOA_SPEECH_WRITE_KINDS=1 cargo test -p canoa --features speech --test account_speech`.
#![cfg(feature = "speech")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::speech::account::{
    AccountSpeechInfo, AccountSpeechModel, AccountSpeechService, AccountSpeechVtable,
};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
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
    if std::env::var_os("CANOA_SPEECH_WRITE_KINDS").is_some() {
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

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document = format!("{{\"kinds\":[{}]}}", AccountSpeechVtable::description());
    committed(
        "include/canoa.account_speech.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: serde_json::Map<String, Json> = [
        ("AccountSpeechModel", schema_of::<AccountSpeechModel>()),
        ("AccountSpeechService", schema_of::<AccountSpeechService>()),
        ("AccountSpeechInfo", schema_of::<AccountSpeechInfo>()),
    ]
    .into_iter()
    .map(|(name, schema)| (name.to_owned(), schema))
    .collect();
    committed(
        "include/canoa.account_speech.schemas.json",
        &pretty(&Json::Object(schemas)),
    );
}

#[test]
fn a_description_survives_the_trip_through_a_value() {
    let mut service = AccountSpeechService::new(
        "acme",
        "Acme",
        "acme-api",
        "https://api.acme.test/v1",
        "https://api.acme.test/v1/audio/transcriptions",
        "api_key",
    );
    service.models = vec![AccountSpeechModel::new("m", "M")];
    let info = AccountSpeechInfo::new(vec![service]);
    let value = info.to_value().expect("a value");
    assert_eq!(
        AccountSpeechInfo::from_value(&value).expect("read back"),
        info
    );
}
