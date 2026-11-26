//! The `content_source` and `catalog_generator` kinds: the contract outside providers are built against.
//! Regenerate the description files with `CANOA_CONTENT_WRITE_KINDS=1 cargo test -p canoa --features content --test content_kinds`.
#![cfg(feature = "content")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::content::{
    ByteRange, CatalogGeneratorProviderVtable, ContentHostVtable, ContentInfo,
    ContentSourceProviderVtable, Delivery, Digest, GenerateRequest, GeneratorInfo, ListEntry,
    ListRequest, Listing, Locator, OpenRequest, Opened, Refusal, Resolved, SchemeInfo, Stat,
    StatAnswer, Target,
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
    if std::env::var_os("CANOA_CONTENT_WRITE_KINDS").is_some() {
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

fn kinds_of(descriptions: &[&str]) -> Json {
    let document = format!("{{\"kinds\":[{}]}}", descriptions.join(","));
    serde_json::from_str(&document).expect("a kind's description is JSON")
}

#[test]
fn the_committed_description_of_content_source_is_what_this_build_describes() {
    committed(
        "include/canoa.content_source.kinds.json",
        &pretty(&kinds_of(&[
            &ContentSourceProviderVtable::description(),
            &ContentHostVtable::description(),
        ])),
    );
}

#[test]
fn the_committed_description_of_catalog_generator_is_what_this_build_describes() {
    committed(
        "include/canoa.catalog_generator.kinds.json",
        &pretty(&kinds_of(&[&CatalogGeneratorProviderVtable::description()])),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: serde_json::Map<String, Json> = [
        ("ByteRange", schema_of::<ByteRange>()),
        ("ContentInfo", schema_of::<ContentInfo>()),
        ("Delivery", schema_of::<Delivery>()),
        ("Digest", schema_of::<Digest>()),
        ("GenerateRequest", schema_of::<GenerateRequest>()),
        ("GeneratorInfo", schema_of::<GeneratorInfo>()),
        ("ListEntry", schema_of::<ListEntry>()),
        ("ListRequest", schema_of::<ListRequest>()),
        ("Listing", schema_of::<Listing>()),
        ("Locator", schema_of::<Locator>()),
        ("OpenRequest", schema_of::<OpenRequest>()),
        ("Opened", schema_of::<Opened>()),
        ("Refusal", schema_of::<Refusal>()),
        ("Resolved", schema_of::<Resolved>()),
        ("SchemeInfo", schema_of::<SchemeInfo>()),
        ("Stat", schema_of::<Stat>()),
        ("StatAnswer", schema_of::<StatAnswer>()),
        ("Target", schema_of::<Target>()),
    ]
    .into_iter()
    .map(|(name, schema)| (name.to_owned(), schema))
    .collect();
    committed(
        "include/canoa.content.schemas.json",
        &pretty(&Json::Object(schemas)),
    );
}

#[test]
fn an_answer_survives_the_trip_through_a_value() {
    let mut stat = Stat::file().sized(3);
    stat.digests.push(Digest::published("sha256", "ab"));
    let opened = Opened::inline(stat, b"abc".to_vec());
    let value = opened.to_value().expect("a value");
    assert_eq!(Opened::from_value(&value).expect("read back"), opened);
    let listing = Listing::of(vec![ListEntry::new(
        "a.toml",
        canoa::content::EntryKind::File,
    )]);
    let value = listing.to_value().expect("a value");
    assert_eq!(Listing::from_value(&value).expect("read back"), listing);
}
