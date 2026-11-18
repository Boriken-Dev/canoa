//! What a catalog source says of itself: which Store types it serves and whether its fields can be a form.
#![cfg(feature = "store")]

use canoa::store::source::{SourceField, SourceInfo};

#[test]
fn a_source_with_no_types_serves_every_type_and_one_with_types_serves_only_those() {
    let everywhere = SourceInfo::new("read", "Read");
    assert!(everywhere.serves("overlays") && everywhere.serves("anything"));
    let mut one = SourceInfo::new("only_skills", "Only skills");
    one.types = vec!["skills".to_owned()];
    assert!(one.serves("skills"));
    assert!(!one.serves("overlays"));
}

#[test]
fn a_source_is_refused_for_an_id_a_shape_or_a_field_listed_twice() {
    let mut info = SourceInfo::new("run", "Run");
    info.fields = vec![
        SourceField::new("command", "Command", "list").required(),
        SourceField::new("cwd", "Folder", "path"),
    ];
    assert_eq!(info.problem(), None);

    assert!(SourceInfo::new("Bad Id", "x").problem().is_some());
    assert!(SourceInfo::new("run", " ").problem().is_some());

    let mut odd = info.clone();
    odd.fields.push(SourceField::new("when", "When", "date"));
    assert!(odd.problem().is_some_and(|why| why.contains("date")));

    let mut twice = info;
    twice.fields.push(SourceField::new("cwd", "Again", "text"));
    assert!(twice.problem().is_some_and(|why| why.contains("twice")));
}
