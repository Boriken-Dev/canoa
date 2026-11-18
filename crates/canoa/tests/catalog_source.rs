//! What a catalog source says of itself: which Store types it serves and whether its form can be shown.
#![cfg(feature = "store")]

use canoa::store::source::SourceInfo;
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;

/// What a stand-in source asks for.
#[derive(ToValue, FromValue, Schema, Form)]
#[allow(dead_code, reason = "only its description is read")]
struct Asked {
    /// The program and its arguments.
    #[schema(title = "Command")]
    command: Vec<String>,
    /// Where it runs.
    #[schema(title = "Folder")]
    cwd: Option<String>,
}

#[test]
fn a_source_with_no_types_serves_every_type_and_one_with_types_serves_only_those() {
    let mut info = SourceInfo::new("run", "Run");
    assert!(info.serves("overlays") && info.serves("anything"));
    info.types = vec!["skills".into()];
    assert!(info.serves("skills"));
    assert!(!info.serves("overlays"));
}

#[test]
fn a_source_is_refused_for_an_id_a_missing_title_or_a_form_its_schema_does_not_fit() {
    let info = SourceInfo::new("run", "Run").asking::<Asked>().unwrap();
    assert_eq!(info.problem(), None);
    assert_eq!(
        SourceInfo::new("run", "Run").problem(),
        None,
        "a source may ask for nothing"
    );

    for id in ["", "Run", "a b", &"a".repeat(33)] {
        assert!(SourceInfo::new(id, "Run").problem().is_some(), "{id:?}");
    }
    assert!(SourceInfo::new("run", " ").problem().is_some());

    let mut odd = SourceInfo::new("run", "Run");
    odd.form = info.form.clone();
    assert!(odd.problem().is_some_and(|why| why.contains("no schema")));
}
