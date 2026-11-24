use serde_json::json;

use super::*;

#[test]
fn a_string_is_read_by_the_read_provider_and_a_table_names_its_provider_and_fields() {
    let table = json!({
        "plain": "https://example.org/o.git@main#catalog.toml",
        "full": {"provider": "read", "source": "https://example.org/p.git@main#c.toml"},
        "made": {"provider": "command", "command": ["gen", "--all"], "timeout": 5},
        "bare": {"source": "https://example.org/b.git@main#c.toml"},
        "off": false,
    });
    let parsed = read(&table);
    let entries: Vec<&Entry> = parsed
        .iter()
        .map(|each| each.as_ref().expect("valid"))
        .collect();
    let labels: Vec<&str> = entries.iter().map(|each| each.label.as_str()).collect();
    assert_eq!(
        labels,
        ["plain", "full", "made", "bare"],
        "in the file's order, and false lists nothing"
    );
    assert_eq!(entries[3].provider, READ, "a table naming none is read");
    assert_eq!(
        entries[0].source(),
        Some("https://example.org/o.git@main#catalog.toml")
    );
    assert_eq!(entries[2].provider, "command");
    assert_eq!(entries[2].fields["command"], json!(["gen", "--all"]));
    assert_eq!(entries[2].fields["timeout"], json!(5));
    assert!(!entries[2].fields.contains_key("provider"));
}

#[test]
fn what_is_no_catalog_says_why_and_the_others_stay() {
    let table = json!({
        "my catalog": "https://example.org/o.git@main#c.toml",
        "Team": "https://example.org/t.git@main#c.toml",
        "team": "https://example.org/u.git@main#c.toml",
        "number": 3,
        "odd": {"provider": 7},
        "good": "https://example.org/g.git@main#c.toml",
    });
    let parsed = read(&table);
    let bad: Vec<&str> = parsed
        .iter()
        .filter_map(|each| each.as_ref().err())
        .map(|(label, _)| label.as_str())
        .collect();
    assert!(bad.contains(&"my catalog") && bad.contains(&"number") && bad.contains(&"odd"));
    assert_eq!(
        parsed.iter().filter(|each| each.is_ok()).count(),
        2,
        "{parsed:?}"
    );
    assert!(read(&json!("text")).is_empty());
    assert!(read(&Value::Null).is_empty());
}

#[test]
fn only_a_source_the_file_does_not_already_hold_counts_as_new() {
    let now = json!({"overlays": {"catalogs": {
        "made": {"provider": "command", "command": ["gen"]},
        "plain": "https://example.org/o.git@main#c.toml",
    }}});
    // The same table written back, with a plain catalog added: nothing new runs.
    let same = json!({"overlays": {"catalogs": {
        "made": {"provider": "command", "command": ["gen"]},
        "plain": "https://example.org/o.git@main#c.toml",
        "more": {"provider": "read", "source": "https://example.org/m.git@main#c.toml"},
        "text": "owner/repo",
    }}, "skills": {"catalogs": {"mine": "owner/repo"}}, "contents_ttl_seconds": 5});
    assert!(!names_new_source(&now, &same));
    let changed =
        json!({"overlays": {"catalogs": {"made": {"provider": "command", "command": ["other"]}}}});
    assert!(names_new_source(&now, &changed));
    let added = json!({"skills": {"catalogs": {"mine": {"provider": "everywhere"}}}});
    assert!(names_new_source(&now, &added));
    assert!(!names_new_source(&now, &Value::Null));
}

#[test]
fn each_type_reads_its_own_table_so_a_catalog_of_one_is_never_another_types() {
    let store = json!({
        "overlays": {"catalogs": {"team": "https://example.org/o.git@main#c.toml"}},
        "skills": {"catalogs": {"mine": "owner/repo"}},
        "mcps": {"catalogs": {"corp": "https://registry.example/"}},
        "extensions": {"catalogs": {"ext": "https://example.org/e.git@main#extensions.toml"}},
    });
    let labels = |kind: &str| -> Vec<String> {
        read(table_of(&store, kind))
            .into_iter()
            .filter_map(Result::ok)
            .map(|entry| entry.label)
            .collect()
    };
    assert_eq!(labels("overlays"), ["team"]);
    assert_eq!(labels("skills"), ["mine"]);
    assert_eq!(labels("mcps"), ["corp"]);
    assert_eq!(labels("extensions"), ["ext"]);
    assert!(labels("runtimes").is_empty());
}

#[test]
fn a_default_catalog_is_listed_first_until_its_label_is_named_or_turned_off() {
    let types = built_in_types();
    let overlays = types
        .iter()
        .find(|kind| kind.id == "overlays")
        .expect("overlays");
    let labels = |table: Value| -> Vec<String> {
        rows_of(overlays, &table)
            .into_iter()
            .map(|row| row["label"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    assert_eq!(labels(json!(null)), ["boriken"]);
    assert_eq!(
        labels(json!({"mine": "https://example.org/m.git@main#c.toml"})),
        ["boriken", "mine"]
    );
    assert_eq!(
        labels(json!({"boriken": false})),
        ["boriken"],
        "a default turned off is still listed, off"
    );
    assert_eq!(
        labels(json!({"boriken": "https://example.org/b.git@main#c.toml"})),
        ["boriken"]
    );

    let mcps = types.iter().find(|kind| kind.id == "mcps").expect("mcps");
    let mcp_labels = |table: Value| -> Vec<String> {
        rows_of(mcps, &table)
            .into_iter()
            .map(|row| row["label"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    assert_eq!(mcp_labels(json!(null)), ["official"]);
    assert_eq!(mcp_labels(json!({"official": false})), ["official"]);
    assert_eq!(
        mcp_labels(json!({"official": "https://mirror.example/"})),
        ["official"]
    );

    let extensions = types
        .iter()
        .find(|kind| kind.id == "extensions")
        .expect("extensions");
    assert!(
        rows_of(extensions, &json!(null)).is_empty(),
        "none until the table names one"
    );
}

#[test]
fn store_catalogs_section_uses_a_closed_guatiao_schema() {
    use coqui_acp_api::guatiao::FromValue as _;

    let doc = guatiao_serde::adapt::from_value::<Value>(&schema().expect("schema")).expect("json");
    assert_eq!(doc["additionalProperties"], false);
    assert!(doc["properties"].get("catalogs").is_some());

    let valid =
        guatiao_serde::from_serde(json!({"catalogs": {"team": "owner/repo"}})).expect("val");
    let decoded = CatalogTableSchema::from_value(&valid).expect("decodes");
    assert!(decoded.catalogs.contains_key("team"));

    let unknown = guatiao_serde::from_serde(json!({"unexpected": 1})).expect("val");
    assert!(CatalogTableSchema::from_value(&unknown).is_err());
}

fn scope(defaults: &[(&str, &str)]) -> TypeCatalogs {
    TypeCatalogs {
        id: "whisper".to_owned(),
        title: "Whisper".to_owned(),
        noun: "source".to_owned(),
        plural: "sources".to_owned(),
        lists: "models".to_owned(),
        defaults: defaults
            .iter()
            .map(|(label, provider)| Default {
                label: (*label).to_owned(),
                provider: (*provider).to_owned(),
                fields: Map::new(),
                pin: Some("5359861c".to_owned()),
                snapshot: None,
            })
            .collect(),
        fixed: None,
        format: Some("canoa.models/1".to_owned()),
    }
}

fn shown(kind: &TypeCatalogs, table: Value) -> Vec<(String, String, bool)> {
    rows_of(kind, &table)
        .into_iter()
        .map(|row| {
            (
                row["label"].as_str().unwrap_or_default().to_owned(),
                row["origin"].as_str().unwrap_or("problem").to_owned(),
                row["enabled"].as_bool().unwrap_or(false),
            )
        })
        .collect()
}

#[test]
fn an_extensions_defaults_come_first_then_an_entry_replaces_by_label_adds_or_turns_one_off() {
    let kind = scope(&[("ggerganov", "whisper_hf"), ("local", "whisper_folder")]);
    let row = |label: &str, origin: &str, on: bool| (label.to_owned(), origin.to_owned(), on);
    assert_eq!(
        shown(&kind, Value::Null),
        [
            row("ggerganov", "default", true),
            row("local", "default", true)
        ]
    );
    let table = json!({
        "mine": {"provider": "whisper_hf", "repo": "me/models"},
        "Local": {"provider": "read", "source": "D:/models.json"},
        "ggerganov": false,
    });
    assert_eq!(
        shown(&kind, table.clone()),
        [
            row("ggerganov", "default", false),
            row("Local", "replaced", true),
            row("mine", "added", true),
        ],
        "the defaults keep their places, a replacement takes the place of its default, a new label follows"
    );
    let rows = rows_of(&kind, &table);
    assert_eq!(
        rows[1]["provider"], "read",
        "an entry replaces its default whole"
    );
    assert_eq!(rows[1]["fields"], json!({"source": "D:/models.json"}));
    assert!(
        rows[1].get("pin").is_none(),
        "the default's pin is not carried into the entry that replaced it"
    );
    assert_eq!(
        rows[0]["pin"], "5359861c",
        "an off default keeps what it was"
    );
    assert_eq!(rows[0]["trust"], "pinned");
}

#[test]
fn a_catalog_that_is_off_or_no_catalog_is_not_found_and_says_why() {
    let kind = scope(&[("ggerganov", "whisper_hf")]);
    let off = find(&kind, &json!({"ggerganov": false}), "Ggerganov");
    assert!(off.is_err_and(|why| why.contains("turned off")));
    assert!(find(&kind, &json!({"ggerganov": false}), "other").is_err());
    let found = find(&kind, &Value::Null, "ggerganov").expect("the default");
    assert_eq!(
        (found.provider.as_str(), found.origin),
        ("whisper_hf", Origin::Default)
    );
    let bad = find(&kind, &json!({"ggerganov": {"provider": 7}}), "ggerganov");
    assert!(
        bad.is_err_and(|why| why.contains("provider")),
        "an entry that is no catalog takes its default's place and says why"
    );
}

#[test]
fn the_hosts_keys_are_read_apart_from_the_providers_fields() {
    let table = json!({"mirror": {
        "provider": "codex_releases", "origin": "https://mirror.example",
        "pin": "sha256:1e0c", "trusted": true, "ttl": 90, "keys": ["5A3F"],
    }});
    let entry = read(&table).remove(0).expect("valid");
    assert_eq!(
        Value::Object(entry.fields.clone()),
        json!({"origin": "https://mirror.example"})
    );
    assert_eq!(
        (
            entry.pin.as_deref(),
            entry.trusted,
            entry.ttl,
            entry.keys.as_slice()
        ),
        (
            Some("sha256:1e0c"),
            true,
            Some(90),
            ["5A3F".to_owned()].as_slice()
        )
    );
    assert_eq!(
        entry.trust(),
        "unverified",
        "an entry that takes files with no digest is drawn as such"
    );
    for bad in [
        json!({"pin": 3}),
        json!({"trusted": "yes"}),
        json!({"ttl": "soon"}),
        json!({"keys": "5A3F"}),
    ] {
        let parsed = read(&json!({"x": bad}));
        assert!(parsed[0].is_err(), "{parsed:?}");
    }
    let plain = read(&json!({"a": {"provider": "x"}}))
        .remove(0)
        .expect("valid");
    assert_eq!(plain.trust(), "verified");
}

#[test]
fn the_types_fixed_catalog_cannot_be_replaced_or_turned_off() {
    let mut kind = scope(&[]);
    kind.fixed = Some(("official", "https://registry.example"));
    for table in [
        json!({"official": "https://mirror.example"}),
        json!({"official": false}),
    ] {
        let rows = rows_of(&kind, &table);
        assert_eq!(rows[0]["fixed"], true);
        assert_eq!(rows[0]["enabled"], true);
        if table["official"] != json!(false) {
            assert!(rows[1].get("problem").is_some(), "{rows:?}");
        }
    }
}
