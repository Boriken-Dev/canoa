#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;

fn layer(kind: LayerKind, dir: Option<PathBuf>, text: &str) -> Layer {
    Layer::new(
        LayerId::new(kind, ""),
        dir,
        Profile::from_str(text, kind).expect("parses"),
    )
}

#[test]
fn a_literal_value_never_shows_in_debug_and_a_secret_never_shows_at_all() {
    let profile =
        Profile::from_str("[env.A]\nvalue = \"hunter2\"\n", LayerKind::User).expect("parses");
    assert!(!format!("{profile:?}").contains("hunter2"));
    let secret = Secret("hunter2".to_owned());
    assert!(!format!("{secret:?} {secret}").contains("hunter2"));
}

#[test]
fn references_resolve_only_when_asked_and_errors_never_quote_the_value() {
    let dir = std::env::temp_dir().join(format!("canoa-profile-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    std::fs::write(dir.join("token"), "file-secret\r\n").expect("a file");
    let effective = merge(&[layer(
        LayerKind::User,
        Some(dir.clone()),
        "[env.LITERAL]\nvalue = \"plain\"\n[env.FROM_ENV]\nfrom_env = \"CANOA_PROFILE_TEST\"\n[env.FROM_FILE]\nfrom_file = \"token\"\n",
    )]);
    let view = effective.view().to_string();
    assert!(
        !view.contains("file-secret") && !view.contains("env-secret"),
        "the view holds references only"
    );

    let lookup = |name: &str| (name == "CANOA_PROFILE_TEST").then(|| "env-secret".to_owned());
    let resolved = effective.environment(lookup).expect("resolves");
    let values: Vec<_> = resolved
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose()))
        .collect();
    assert_eq!(
        values,
        [
            ("LITERAL", "plain"),
            ("FROM_ENV", "env-secret"),
            ("FROM_FILE", "file-secret")
        ]
    );

    let missing = effective
        .environment(|_| None)
        .expect_err("an unset variable");
    assert_eq!(missing.code(), "resolve");
    assert!(missing.to_string().contains("env.FROM_ENV"));
    std::fs::remove_dir_all(&dir).expect("cleaned");
}

#[test]
fn a_relative_file_in_a_layer_with_no_directory_does_not_resolve() {
    let effective = merge(&[layer(
        LayerKind::Session,
        None,
        "[env.A]\nfrom_file = \"token\"\n",
    )]);
    let error = effective.environment(|_| None).expect_err("no directory");
    assert_eq!(error.code(), "resolve");
}

#[test]
fn a_parse_error_has_a_position_and_no_text_from_the_document() {
    let error = Profile::from_str("model = \"a\"\nsecret_key = \"hunter2\"\n", LayerKind::User)
        .expect_err("refused");
    let ProfileError::Parse { line, .. } = error else {
        panic!("a parse error");
    };
    assert_eq!(line, 2);
    assert!(!error.to_string().contains("hunter2") && !error.to_string().contains("secret_key"));
}

#[test]
fn a_canoa_cfg_table_reads_as_a_document_does() {
    let table: toml::Table =
        toml::from_str("model = \"m\"\n[skills.s]\npath = \"/abs\"\n").expect("TOML");
    let profile = Profile::from_table(table.clone(), LayerKind::User)
        .expect("a user layer takes an absolute path");
    assert_eq!(profile.model.as_deref(), Some("m"));
    let error = Profile::from_table(table, LayerKind::Overlay).expect_err("an overlay does not");
    assert_eq!(error.code(), "path");
}

#[test]
fn a_project_secret_file_is_read_only_inside_the_project_links_followed() {
    let root = std::env::temp_dir().join(format!("canoa-profile-link-{}", std::process::id()));
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("a directory");
    std::fs::write(project.join("token"), "inside\n").expect("a file");
    std::fs::write(root.join("outside"), "outside").expect("a file");
    let resolve = |name: &str| {
        let text = format!("[env.A]\nfrom_file = \"{name}\"\n");
        merge(&[layer(LayerKind::Project, Some(project.clone()), &text)]).environment(|_| None)
    };
    let inside = resolve("token").expect("inside the project");
    assert_eq!(inside[0].1.expose(), "inside");

    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(root.join("outside"), project.join("escape"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(root.join("outside"), project.join("escape"));
    if linked.is_ok() {
        let error = resolve("escape").expect_err("a link out of the project");
        assert_eq!(error.code(), "resolve");
    } else {
        eprintln!("this account cannot make links: the escape is not tried");
    }
    std::fs::remove_dir_all(&root).expect("cleaned");
}
