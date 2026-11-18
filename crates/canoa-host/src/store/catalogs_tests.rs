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
    assert!(labels(json!({"boriken": false})).is_empty());
    assert_eq!(
        labels(json!({"boriken": "https://example.org/b.git@main#c.toml"})),
        ["boriken"]
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
