//! The cases the application's form renderer is held to (`app/test/fixtures/form_vectors.json`), run against guatiao-intake itself:
//! the layout and the visibility rules are its, and the renderer in the application follows them from the same file.
#![cfg(feature = "settings-section")]

use guatiao::Value;
use guatiao::schema::read::SchemaRef;
use guatiao_intake::{FormRef, check, is_visible, layout};
use serde_json::json;

fn value(from: &serde_json::Value) -> Value {
    guatiao_serde::from_serde(from.clone()).expect("a JSON document is a value")
}

#[test]
fn guatiao_intake_lays_out_and_shows_the_cases_as_they_say() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../app/test/fixtures/form_vectors.json"
    ))
    .expect("the vectors are in the application's fixtures");
    let vectors: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert!(cases.len() >= 8, "the cases were read");

    for case in cases {
        let name = case["name"].as_str().unwrap();
        let schema = value(&case["schema"]);
        let form = value(&case["form"]);
        let schema_ref = SchemaRef::new(&schema).expect(name);
        let form_ref = FormRef::new(&form).expect(name);
        check(schema_ref, form_ref).unwrap_or_else(|why| panic!("{name}: {why}"));

        let groups: Vec<serde_json::Value> = layout(schema_ref, form_ref)
            .iter()
            .map(|group| {
                let (section, title) = group
                    .section
                    .map_or(("", ""), |section| (section.id(), section.label()));
                let mut each = json!({
                    "section": section,
                    "fields": group.fields.iter().map(|placed| placed.field.key()).collect::<Vec<_>>(),
                });
                if !title.is_empty() {
                    each["title"] = json!(title);
                }
                each
            })
            .collect();
        assert_eq!(
            serde_json::Value::Array(groups),
            case["layout"],
            "{name}: layout"
        );

        for shown in case["shown"].as_array().unwrap() {
            let held = value(&shown["values"]);
            let members: Vec<&str> = schema_ref
                .fields()
                .map(|field| field.key())
                .filter(|key| is_visible(schema_ref, form_ref, key, &held).unwrap())
                .collect();
            let expected: Vec<&str> = shown["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|member| member.as_str().unwrap())
                .collect();
            assert_eq!(members, expected, "{name}: shown for {}", shown["values"]);
        }
    }
}
