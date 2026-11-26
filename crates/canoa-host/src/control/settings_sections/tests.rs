use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::Keeper;
use super::{get, list, set};
use crate::config::Config;
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

/// A configuration that keeps what the host keeps for other kinds in sealed files under `tmp`, so no test touches the operating
/// system's store.
fn sealed(tmp: &std::path::Path) -> Config {
    Config::from_str_in("[secrets]\naccounts = \"file\"\n", tmp).expect("a configuration")
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
    let answer = list(&with_extension(), true);
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

/// The sections the application draws itself, in the order it lists them.
const BUILT_IN: [&str; 21] = [
    "general",
    "appearance",
    "network",
    "notifications",
    "voice",
    "shortcuts",
    "telemetry",
    "sources",
    "profiles",
    "mcp",
    "permissions",
    "accounts",
    "shells",
    "usage",
    "schedules",
    "extensions",
    "catalog_sources",
    "content",
    "advanced",
    "providers",
    "about",
];

fn ids(answer: &Value) -> Vec<String> {
    answer["sections"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|each| each["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn the_sections_the_application_draws_are_listed_as_providers_first_and_an_extension_taken_away_takes_only_its_own()
 {
    let without = list(&SectionCatalog::from_registries(Vec::new()), true);
    assert_eq!(ids(&without), BUILT_IN);
    let with = ids(&list(&with_extension(), true));
    assert_eq!(with[..BUILT_IN.len()], BUILT_IN);
    assert_eq!(with[BUILT_IN.len()..], ["cache", "region"]);
    for id in BUILT_IN {
        let each = row(&without, id);
        assert_eq!(each["origin"], "built-in", "{id}");
        assert_eq!(
            each["schema"],
            Value::Null,
            "{id}: the application draws it"
        );
        assert!(
            each["group"].is_string() && each["icon"].is_string() && each["keywords"].is_string(),
            "{id}: {each}"
        );
    }
    assert_eq!(row(&without, "accounts")["group"], "Agents");
    assert_eq!(row(&without, "about")["group"], "About");
}

/// The Dart file the application falls back on, written from what the host lists of its own.
fn fallback_source(listed: &Value) -> String {
    let json = serde_json::to_string_pretty(listed).expect("rows are JSON");
    assert!(!json.contains("'''"), "the raw string would end early");
    format!(
        "// What the host lists of the application's own Settings pages: the list Settings shows until the host has answered, and when it\n\
         // cannot or will not. Written by `CANOA_WRITE_SECTIONS=1 cargo test -p canoa-host --lib settings_sections`, which also checks it.\n\
         library;\n\
         \n\
         /// The answer of `settings/sections` for the pages the application draws itself, as JSON.\n\
         const builtInSectionsListing = r'''\n{json}\n''';\n"
    )
}

#[test]
fn the_apps_fallback_is_what_this_build_lists_of_its_own() {
    let listed = list(&SectionCatalog::from_registries(Vec::new()), false);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/lib/settings/built_in_sections.dart");
    let text = fallback_source(&listed);
    if std::env::var_os("CANOA_WRITE_SECTIONS").is_some() {
        fs::write(&path, &text).expect("the file is written");
    }
    assert!(
        fs::read_to_string(&path).expect("the file is committed") == text,
        "regenerate it: CANOA_WRITE_SECTIONS=1 cargo test -p canoa-host --lib settings_sections"
    );
}

#[test]
fn a_key_that_reads_no_configuration_is_listed_the_applications_own_pages_and_none_a_provider_adds()
{
    let with = list(&with_extension(), true);
    let without = list(&with_extension(), false);
    assert_eq!(ids(&without), BUILT_IN);
    assert!(ids(&with).len() > BUILT_IN.len());
    for each in without["sections"].as_array().expect("a list") {
        assert_eq!(each["origin"], "built-in", "{each}");
        assert_eq!(
            each["schema"],
            Value::Null,
            "no form of a provider's: {each}"
        );
    }
}

#[test]
fn a_section_kept_in_canoa_cfg_round_trips_and_keeps_its_secret_unless_it_is_cleared() {
    let tmp = dir("config");
    let files = files(&tmp);
    let config = sealed(&tmp);
    let keeper = Keeper::new(&config, Some(&tmp));
    let catalog = with_extension();
    let none = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(none["values"], json!({}));
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "region", "values": {"name": "eu", "tier": "fast", "token": "s3cret"}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(read["values"], json!({"name": "eu", "tier": "fast"}));
    assert_eq!(read["secrets_set"], json!(["token"]), "{read}");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("the file");
    assert!(
        text.contains("[settings.region]") && text.contains("eu") && !text.contains("s3cret"),
        "the file holds the section's values and not its secret: {text}"
    );
    assert!(!text.contains("token"), "{text}");
    assert_eq!(
        keeper
            .get("region", "token")
            .expect("asked")
            .as_deref()
            .map(String::as_str),
        Some("s3cret"),
        "the secret is the named secret settings/region/token"
    );
    assert!(
        !tmp.join("secrets").exists()
            || fs::read_dir(tmp.join("secrets"))
                .expect("the folder")
                .all(|entry| !fs::read(entry.expect("an entry").path())
                    .expect("a file")
                    .windows(6)
                    .any(|window| window == b"s3cret")),
        "the sealed file does not hold it in clear"
    );
    crate::config::Config::from_str_in(&text, std::path::Path::new("."))
        .expect("the host still reads a canoa.cfg that holds a section's values");
    // A form that never showed the secret leaves it alone.
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "region", "values": {"name": "us", "tier": "standard", "token": ""}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(read["values"]["name"], "us");
    assert_eq!(read["secrets_set"], json!(["token"]));
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "region", "values": {"name": "us"}, "clear": ["token"]}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(read["secrets_set"], json!([]), "{read}");
    assert_eq!(keeper.get("region", "token").expect("asked"), None);
    // The values stay in the file when the extension goes, and come back with it.
    let gone = SectionCatalog::from_registries(Vec::new());
    assert!(get(&gone, &files, &keeper, &json!({"id": "region"})).is_err());
    assert!(
        fs::read_to_string(tmp.join("canoa.cfg"))
            .expect("the file")
            .contains("us")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_secret_an_earlier_host_kept_in_canoa_cfg_is_moved_to_a_named_secret_once() {
    let tmp = dir("legacy");
    let files = files(&tmp);
    fs::write(
        tmp.join("canoa.cfg"),
        "[settings.region]\nname = \"eu\"\ntoken = \"old-token\"\n\n[other]\nkeep = 1\n",
    )
    .expect("written");
    let config = sealed(&tmp);
    let keeper = Keeper::new(&config, Some(&tmp));
    let catalog = with_extension();
    let read = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(read["values"], json!({"name": "eu"}), "{read}");
    assert_eq!(read["secrets_set"], json!(["token"]), "{read}");
    assert!(read.get("secrets_problem").is_none(), "{read}");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("the file");
    assert!(
        !text.contains("old-token") && !text.contains("token"),
        "{text}"
    );
    assert!(
        text.contains("name = \"eu\"") && text.contains("[other]"),
        "{text}"
    );
    assert_eq!(
        keeper
            .get("region", "token")
            .expect("asked")
            .as_deref()
            .map(String::as_str),
        Some("old-token")
    );
    // Asked again, nothing is left to move and the secret is still there.
    let again = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert_eq!(again, read);
    // Saving what the form showed, the secret left empty, keeps it.
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "region", "values": {"name": "us", "token": ""}}),
    )
    .expect("saved");
    assert_eq!(
        keeper
            .get("region", "token")
            .expect("asked")
            .as_deref()
            .map(String::as_str),
        Some("old-token")
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_secret_that_cannot_be_moved_stays_where_it_was_and_the_form_says_why() {
    let tmp = dir("stuck");
    let files = files(&tmp);
    let original = "[settings.region]\nname = \"eu\"\ntoken = \"old-token\"\n";
    fs::write(tmp.join("canoa.cfg"), original).expect("written");
    // `[secrets] accounts` names an entry that is no provider: nothing can keep a secret.
    let config = Config::from_str_in("[secrets]\naccounts = \"missing\"\n", &tmp).expect("config");
    let keeper = Keeper::new(&config, Some(&tmp));
    let catalog = with_extension();
    let read = get(&catalog, &files, &keeper, &json!({"id": "region"})).expect("read");
    assert!(
        read["secrets_problem"]
            .as_str()
            .is_some_and(|why| why.contains("missing")),
        "{read}"
    );
    assert_eq!(read["values"], json!({"name": "eu"}));
    assert_eq!(
        fs::read_to_string(tmp.join("canoa.cfg")).expect("the file"),
        original
    );
    // Saving the section is refused rather than dropping the secret from the file.
    let error = set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "region", "values": {"name": "us"}}),
    )
    .expect_err("refused");
    assert!(error.1.contains("missing"), "{}", error.1);
    assert_eq!(
        fs::read_to_string(tmp.join("canoa.cfg")).expect("the file"),
        original
    );
    // A host with no home keeps no secret.
    fs::write(tmp.join("canoa.cfg"), "").expect("emptied");
    let homeless = Keeper::new(&config, None);
    let error = set(
        &catalog,
        &files,
        &homeless,
        &json!({"id": "region", "values": {"name": "us", "token": "x"}}),
    )
    .expect_err("refused");
    assert!(
        error.1.contains("token") && error.1.contains("no home"),
        "{}",
        error.1
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_section_kept_by_its_provider_is_read_and_written_through_it() {
    let tmp = dir("provider");
    let files = files(&tmp);
    let config = sealed(&tmp);
    let keeper = Keeper::new(&config, Some(&tmp));
    let catalog = with_extension();
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "cache", "values": {"entries": 40}}),
    )
    .expect("saved");
    let read = get(&catalog, &files, &keeper, &json!({"id": "cache"})).expect("read");
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
    let config = sealed(&tmp);
    let keeper = Keeper::new(&config, Some(&tmp));
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
        let error = set(&catalog, &files, &keeper, &params).expect_err("refused");
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
    let refused = ask(
        &[Grant::StoreRead],
        "settings/section_get",
        json!({"id": "x"}),
    );
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    // The list of pages is the application's own preferences: a key with no grant at all is answered, with those pages only.
    for grants in [&[Grant::StoreRead][..], &[][..]] {
        let listed = ask(grants, "settings/sections", json!({}));
        let rows = listed["result"]["sections"].as_array().expect("a list");
        assert!(!rows.is_empty(), "{listed}");
        assert!(
            rows.iter().all(|row| row["origin"] == "built-in"),
            "{listed}"
        );
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

#[test]
fn the_power_group_is_a_group_of_general_kept_where_the_host_reads_it() {
    let tmp = dir("power");
    let files = files(&tmp);
    let config = sealed(&tmp);
    let keeper = Keeper::new(&config, Some(&tmp));
    let catalog = SectionCatalog::of(&config);
    let answer = list(&catalog, true);
    let power = row(&answer, "power");
    assert_eq!(power["under"], "general");
    assert_eq!(power["stored"], "config");
    assert_eq!(
        power["schema"]["properties"]["keep_awake"]["type"],
        "boolean"
    );
    assert!(
        row(&list(&catalog, false), "general")["id"] == "general",
        "General is still the application's own page"
    );
    assert!(
        list(&catalog, false)["sections"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|each| each["id"] != "power"),
        "a group the host adds is not one of the application's own pages"
    );
    set(
        &catalog,
        &files,
        &keeper,
        &json!({"id": "power", "values": {"keep_awake": true}}),
    )
    .expect("saved");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("the file");
    assert!(
        text.contains("[settings.power]") && text.contains("keep_awake = true"),
        "{text}"
    );
    assert!(
        Config::from_str_in(&text, &tmp).expect("parses").keep_awake,
        "{text}"
    );
}
