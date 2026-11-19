use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::{get, list, set};
use crate::control::files::{Files, UploadLimits};
use crate::settings_sections::SectionCatalog;
use crate::settings_sections::standin::with_extension;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-sections-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn files(tmp: &std::path::Path) -> Files {
    let cfg = tmp.join("canoa.cfg");
    fs::write(&cfg, "").expect("written");
    Files::new(
        Some(tmp.to_path_buf()),
        Vec::new(),
        Some(cfg),
        Some(tmp.join("uploads")),
        UploadLimits::default(),
    )
}

fn row<'a>(answer: &'a Value, id: &str) -> &'a Value {
    answer["sections"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == id)
        .unwrap_or_else(|| panic!("no section {id}: {answer}"))
}

#[test]
fn an_extensions_section_is_listed_as_its_own_and_another_as_a_group_under_an_existing_section() {
    let answer = list(&with_extension());
    let region = row(&answer, "region");
    assert_eq!(region["origin"], "regions");
    assert_eq!(region["group"], "Regions");
    assert_eq!(region["under"], Value::Null);
    assert_eq!(region["stored"], "config");
    assert_eq!(region["icon"], "map");
    assert_eq!(region["schema"]["required"], json!(["name"]));
    assert_eq!(region["form"]["fields"]["name"]["placeholder"], "eu-west");
    let token = &region["schema"]["properties"]["token"];
    assert!(
        token["x-sensitive"] == true || token["writeOnly"] == true,
        "a secret is marked in the schema the form is drawn from: {token}"
    );
    let cache = row(&answer, "cache");
    assert_eq!(cache["under"], "general");
    assert_eq!(cache["stored"], "provider");
}

#[test]
fn an_extension_taken_away_takes_its_sections_from_the_list() {
    let without = list(&SectionCatalog::from_registries(Vec::new()));
    assert_eq!(without["sections"], json!([]));
    assert_eq!(
        list(&with_extension())["sections"]
            .as_array()
            .expect("a list")
            .len(),
        2
    );
}

#[test]
fn a_section_kept_in_canoa_cfg_round_trips_and_keeps_its_secret_unless_it_is_cleared() {
    let tmp = dir("config");
    let files = files(&tmp);
    let catalog = with_extension();
    let none = get(&catalog, &files, &json!({"id": "region"})).expect("read");
    assert_eq!(none["values"], json!({}));
    set(
        &catalog,
        &files,
        &json!({"id": "region", "values": {"name": "eu", "tier": "fast", "token": "s3cret"}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &json!({"id": "region"})).expect("read");
    assert_eq!(read["values"], json!({"name": "eu", "tier": "fast"}));
    assert_eq!(read["secrets_set"], json!(["token"]), "{read}");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("the file");
    assert!(
        text.contains("[settings.region]") && text.contains("s3cret"),
        "{text}"
    );
    crate::config::Config::from_str_in(&text, std::path::Path::new("."))
        .expect("the host still reads a canoa.cfg that holds a section's values");
    // A form that never showed the secret leaves it alone.
    set(
        &catalog,
        &files,
        &json!({"id": "region", "values": {"name": "us", "tier": "standard", "token": ""}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &json!({"id": "region"})).expect("read");
    assert_eq!(read["values"]["name"], "us");
    assert_eq!(read["secrets_set"], json!(["token"]));
    set(
        &catalog,
        &files,
        &json!({"id": "region", "values": {"name": "us"}, "clear": ["token"]}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &json!({"id": "region"})).expect("read");
    assert_eq!(read["secrets_set"], json!([]), "{read}");
    // The values stay in the file when the extension goes, and come back with it.
    let gone = SectionCatalog::from_registries(Vec::new());
    assert!(get(&gone, &files, &json!({"id": "region"})).is_err());
    assert!(
        fs::read_to_string(tmp.join("canoa.cfg"))
            .expect("the file")
            .contains("us")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_section_kept_by_its_provider_is_read_and_written_through_it() {
    let tmp = dir("provider");
    let files = files(&tmp);
    let catalog = with_extension();
    set(
        &catalog,
        &files,
        &json!({"id": "cache", "values": {"entries": 40}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &json!({"id": "cache"})).expect("read");
    assert_eq!(read["values"], json!({"entries": 40}));
    assert!(
        !fs::read_to_string(tmp.join("canoa.cfg"))
            .expect("the file")
            .contains("cache"),
        "a section the provider keeps writes nothing to canoa.cfg"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn values_that_do_not_fit_the_schema_or_name_no_section_are_refused() {
    let tmp = dir("refused");
    let files = files(&tmp);
    let catalog = with_extension();
    for (params, why) in [
        (json!({"id": "region", "values": {"tier": "fast"}}), "name"),
        (
            json!({"id": "region", "values": {"name": "a", "tier": "slow"}}),
            "region",
        ),
        (
            json!({"id": "region", "values": {"name": "a", "color": "red"}}),
            "color",
        ),
        (json!({"id": "region", "values": "x"}), "object"),
        (
            json!({"id": "nothing", "values": {}}),
            "no settings section",
        ),
    ] {
        let error = set(&catalog, &files, &params).expect_err("refused");
        assert!(error.1.contains(why), "{params}: {}", error.1);
    }
    assert_eq!(
        fs::read_to_string(tmp.join("canoa.cfg")).expect("the file"),
        "",
        "a refused write changes nothing"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn listing_needs_the_grant_that_reads_the_configuration_and_saving_the_one_that_writes_it() {
    use crate::control::{
        Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
    };
    let tmp = dir("grants");
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files(&tmp));
    let ask = |grants: &[Grant], method: &str, params: Value| {
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: grants.iter().copied().collect(),
        };
        let line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .expect("encodes");
        serde_json::from_slice::<Value>(&control.handle(&caller, &line).expect("answered"))
            .expect("JSON")
    };
    for method in ["settings/sections", "settings/section_get"] {
        let refused = ask(&[Grant::StoreRead], method, json!({"id": "x"}));
        assert_eq!(refused["error"]["code"], DENIED, "{method}: {refused}");
    }
    let listed = ask(&[Grant::ConfigRead], "settings/sections", json!({}));
    assert!(listed["result"]["sections"].is_array(), "{listed}");
    let refused = ask(
        &[Grant::ConfigRead],
        "settings/section_set",
        json!({"id": "x", "values": {}}),
    );
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let missing = ask(
        &[Grant::ConfigWrite],
        "settings/section_set",
        json!({"id": "x", "values": {}}),
    );
    assert!(
        missing["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("no settings section")),
        "{missing}"
    );
    let _ = fs::remove_dir_all(&tmp);
}
