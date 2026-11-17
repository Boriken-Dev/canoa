//! What an extension's type may name as its icon: one of the application's icons by name, or a small PNG or WebP it ships.
#![cfg(feature = "store")]

use canoa::store::extension::{ExtensionType, MAX_ICON_TEXT};

fn with_icon(icon: Option<&str>) -> ExtensionType {
    let mut kind = ExtensionType::new("echoes", "Echoes", ["echo_store"]);
    kind.icon = icon.map(str::to_owned);
    kind
}

#[test]
fn a_name_or_a_small_image_is_an_icon() {
    assert_eq!(with_icon(None).problem(), None);
    assert_eq!(with_icon(Some("palette")).problem(), None);
    assert_eq!(
        with_icon(Some("a name nothing draws")).problem(),
        None,
        "the application draws its default for a name it does not know"
    );
    assert_eq!(
        with_icon(Some("data:image/png;base64,iVBORw0KGgo=")).problem(),
        None
    );
    assert_eq!(
        with_icon(Some("data:image/webp;base64,UklGRg==")).problem(),
        None
    );
}

#[test]
fn an_image_that_is_not_a_small_png_or_webp_is_refused() {
    for icon in [
        "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
        "data:image/png;base64,",
        "data:image/png;base64,not base64!",
        "data:text/html;base64,PGI+",
        "data:image/png,iVBORw0KGgo=",
    ] {
        assert!(with_icon(Some(icon)).problem().is_some(), "{icon}");
    }
    let big = format!("data:image/png;base64,{}", "A".repeat(MAX_ICON_TEXT));
    let why = with_icon(Some(&big)).problem().expect("too large");
    assert!(why.contains("echoes"), "{why}");
}
