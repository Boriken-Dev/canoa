//! The profile vectors the app is held to as well: each merge's view or first refusal, each overlay source, and each
//! AOS overlay's layer.
#![allow(
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use canoa_host::profile::{Layer, LayerId, LayerKind, OverlaySource, Profile, ProfileError, merge};
use serde_json::Value;

fn vectors() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/profile.json"
    ))
    .expect("the vectors are readable");
    serde_json::from_str(&text).expect("the vectors are JSON")
}

fn kind(name: &str) -> LayerKind {
    match name {
        "built-in" => LayerKind::BuiltIn,
        "overlay" => LayerKind::Overlay,
        "user" => LayerKind::User,
        "project" => LayerKind::Project,
        "agent" => LayerKind::Agent,
        "session" => LayerKind::Session,
        other => panic!("no layer kind {other}"),
    }
}

fn layers(case: &Value) -> Result<Vec<Layer>, (usize, ProfileError)> {
    let mut out = Vec::new();
    for (index, layer) in case["layers"]
        .as_array()
        .expect("layers")
        .iter()
        .enumerate()
    {
        let kind = kind(layer["kind"].as_str().expect("kind"));
        let profile = Profile::from_str(layer["toml"].as_str().expect("toml"), kind)
            .map_err(|error| (index, error))?;
        let name = layer["name"].as_str().unwrap_or("");
        out.push(Layer::new(LayerId::new(kind, name), None, profile));
    }
    Ok(out)
}

#[test]
fn every_merge_vector_gives_its_view_or_its_refusal() {
    let vectors = vectors();
    let cases = vectors["merges"].as_array().expect("merges");
    assert!(cases.len() > 40, "the vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        match (layers(case), case.get("effective"), case.get("error")) {
            (Ok(layers), Some(expected), None) => {
                assert_eq!(&merge(&layers).view(), expected, "{name}");
            }
            (Err((index, error)), None, Some(expected)) => {
                assert_eq!(expected["layer"], index, "{name}: the layer refused");
                assert_eq!(expected["code"], error.code(), "{name}: {error}");
                if let ProfileError::Invalid { key, .. } = &error {
                    assert_eq!(expected["key"], key.as_str(), "{name}");
                }
            }
            (Ok(_), _, Some(_)) => panic!("{name}: accepted, and the vector refuses it"),
            (Err((_, error)), Some(_), _) => {
                panic!("{name}: refused ({error}), and the vector takes it")
            }
            _ => panic!("{name}: a vector has either effective or error"),
        }
    }
}

#[test]
fn every_source_vector_parses_or_is_refused() {
    let vectors = vectors();
    for case in vectors["sources"].as_array().expect("sources") {
        let source = case["source"].as_str().expect("source");
        let parsed = OverlaySource::parse_git(source);
        if case.get("refused").is_some() {
            assert!(parsed.is_err(), "{source} is refused");
            continue;
        }
        let Ok(OverlaySource::Git {
            repo,
            reference,
            path,
        }) = parsed
        else {
            panic!("{source} parses as git");
        };
        assert_eq!(case["repo"], repo.as_str(), "{source}");
        assert_eq!(case["ref"], reference.as_str(), "{source}");
        assert_eq!(case["path"], path.as_str(), "{source}");
    }
}

/// The overlays of a case as layers, in composition order, and the refusals; a manifest's `requires` and `priority`
/// order them as `coqui_aos` composes them.
fn overlay_layers(case: &Value) -> (Vec<Layer>, Vec<Value>) {
    use canoa_host::profile::{OverlayInput, overlay_layer};
    use coqui_aos::{self as aos, OverlayTree};
    let os = case["os"].as_str().expect("os");
    let mut layers = std::collections::BTreeMap::new();
    let mut manifests = std::collections::BTreeMap::new();
    let mut refused = Vec::new();
    for overlay in case["overlays"].as_array().expect("overlays") {
        let identity = overlay["identity"].as_str().expect("identity");
        let tree = OverlayTree::from_texts(
            overlay["files"]
                .as_object()
                .expect("files")
                .iter()
                .map(|(path, text)| (path.clone(), text.as_str().expect("text"))),
        );
        let mut input =
            OverlayInput::new(identity, overlay["root"].as_str().expect("root"), os, &tree);
        input.harness = case["harness"].as_str();
        input.pinned = overlay["pinned"] == true;
        match overlay_layer(&input) {
            Ok(layer) => {
                let manifest = layer.aos.as_ref().expect("an AOS layer").manifest.clone();
                manifests.insert(aos::normalize_name(identity), manifest);
                layers.insert(aos::normalize_name(identity), layer);
            }
            Err(refusal) => refused.push(serde_json::json!({
                "identity": identity, "code": refusal.code, "subject": refusal.subject
            })),
        }
    }
    let requested: Vec<String> = manifests.keys().cloned().collect();
    let visited = aos::resolve(&manifests, &requested)
        .expect("no cycle")
        .install_order;
    let order = aos::composition_order(&manifests, &visited);
    let mut out: Vec<Layer> = order
        .iter()
        .filter_map(|identity| layers.remove(identity))
        .collect();
    if let Some(user) = case["user"].as_str() {
        let profile = Profile::from_str(user, LayerKind::User).expect("the user layer parses");
        out.push(Layer::new(LayerId::new(LayerKind::User, ""), None, profile));
    }
    (out, refused)
}

#[test]
fn every_overlay_vector_gives_its_view_refusals_and_environment() {
    let vectors = vectors();
    let cases = vectors["overlays"].as_array().expect("overlays");
    assert!(cases.len() >= 6, "the overlay vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let (layers, refused) = overlay_layers(case);
        assert_eq!(
            Value::Array(refused),
            case["refused"].clone(),
            "{name}: refused"
        );
        let effective = merge(&layers);
        assert_eq!(effective.view(), case["view"], "{name}: view");
        let host = case["host"].as_object().cloned().unwrap_or_default();
        let lookup = |variable: &str| {
            host.get(variable)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        match (effective.environment(lookup), case.get("environment")) {
            (Ok(found), Some(expected)) => {
                let found: serde_json::Map<String, Value> = found
                    .into_iter()
                    .map(|(variable, value)| (variable, Value::String(value.expose().to_owned())))
                    .collect();
                assert_eq!(&Value::Object(found), expected, "{name}: environment");
            }
            (Err(error), None) => {
                let expected = &case["environment_error"];
                assert_eq!(expected["code"], error.code(), "{name}: {error}");
                if let ProfileError::Resolve { key, .. } = &error {
                    assert_eq!(expected["key"], key.as_str(), "{name}");
                }
            }
            (Ok(_), None) => panic!("{name}: the environment builds, and the vector refuses it"),
            (Err(error), Some(_)) => panic!("{name}: the environment is refused ({error})"),
        }
    }
}

/// An overlay's layer from the text of its files.
fn layer_of(files: &[(&str, &str)]) -> Result<Layer, canoa_host::profile::OverlayRefusal> {
    use canoa_host::profile::{OverlayInput, overlay_layer};
    let tree = coqui_aos::OverlayTree::from_texts(
        files.iter().map(|(path, text)| ((*path).to_owned(), *text)),
    );
    overlay_layer(&OverlayInput::new("team", "/ov/team", "linux", &tree))
}

#[test]
fn an_overlay_md_applies_as_the_overlay_cfg_and_context_md_form_does() {
    use canoa_host::profile::Context;
    let body = "# Team\n\nBe brief.\n";
    let skill = ("skills/review/SKILL.md", "---\nname: review\n---\n");
    let front = "---\nname: team\nenv:\n  TEAM_MODE: strict\n  TEAM_NOTE: two words\n---\n";
    let md = layer_of(&[("OVERLAY.md", &format!("{front}{body}")), skill]).expect("OVERLAY.md");
    let cfg = layer_of(&[
        (
            "overlay.cfg",
            "name = \"team\"\n[env]\nTEAM_MODE = \"strict\"\nTEAM_NOTE = \"two words\"\n",
        ),
        ("CONTEXT.md", body),
        skill,
    ])
    .expect("overlay.cfg");
    assert_eq!(
        md.profile.contexts,
        [Context::TemplateBody(body.to_owned())]
    );
    assert_eq!(
        cfg.profile.contexts,
        [Context::Template("CONTEXT.md".to_owned())]
    );
    let (mut md_profile, mut cfg_profile) = (md.profile.clone(), cfg.profile.clone());
    md_profile.contexts.clear();
    cfg_profile.contexts.clear();
    assert_eq!(
        md_profile, cfg_profile,
        "the same profile but for the context's form"
    );
    let (md_aos, cfg_aos) = (md.aos.expect("aos"), cfg.aos.expect("aos"));
    assert_eq!(md_aos.manifest, cfg_aos.manifest, "the same manifest");
    assert_eq!(md_aos.not_applied, cfg_aos.not_applied);
}

#[test]
fn a_body_that_is_context_ignores_and_reports_context_md() {
    use canoa_host::profile::Context;
    let layer = layer_of(&[
        ("OVERLAY.md", "---\nname: team\n---\nFrom the body.\n"),
        ("CONTEXT.md", "From the file.\n"),
    ])
    .expect("applies");
    assert_eq!(
        layer.profile.contexts,
        [Context::TemplateBody("From the body.\n".to_owned())]
    );
    assert_eq!(
        layer.aos.expect("aos").not_applied,
        ["context: CONTEXT.md, ignored while the body of OVERLAY.md is the context"]
    );
}

#[test]
fn naming_overlay_md_as_context_puts_its_body_there_and_a_blank_body_nothing() {
    use canoa_host::profile::Context;
    let named = layer_of(&[(
        "OVERLAY.md",
        "---\ndefaults:\n  context: false\ncontext: [OVERLAY.md]\n---\nThe body.\n",
    )])
    .expect("applies");
    assert_eq!(
        named.profile.contexts,
        [Context::TemplateBody("The body.\n".to_owned())]
    );
    let blank =
        layer_of(&[("OVERLAY.md", "---\ncontext: [OVERLAY.md]\n---\n \n")]).expect("applies");
    assert!(blank.profile.contexts.is_empty());
}

#[test]
fn a_body_that_is_not_text_refuses_the_overlay() {
    let mut bytes = b"---\nname: team\n---\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe]);
    let tree = coqui_aos::OverlayTree::default().with_bytes("OVERLAY.md", bytes);
    let input = canoa_host::profile::OverlayInput::new("team", "/ov/team", "linux", &tree);
    let refusal = canoa_host::profile::overlay_layer(&input).expect_err("refused");
    assert_eq!(refusal.code, "manifest");
    assert!(
        refusal.subject.starts_with("invalid-utf8"),
        "{}",
        refusal.subject
    );
}

/// The skills of one overlay as canoa's layer gives them: the bundled ones it can deliver by name, and the rest, which
/// canoa never resolves or fetches itself (it has no catalogs): `(name, origin, source, pin)`.
fn layer_skills(
    identity: &str,
    files: &serde_json::Map<String, Value>,
) -> Vec<(String, String, String, String)> {
    use canoa_host::profile::{OverlayInput, overlay_layer};
    use coqui_aos::SkillOrigin;
    let tree = coqui_aos::OverlayTree::from_texts(
        files
            .iter()
            .map(|(path, text)| (path.clone(), text.as_str().expect("text"))),
    );
    let layer =
        overlay_layer(&OverlayInput::new(identity, "/ov/team", "linux", &tree)).expect("applies");
    let mut out: Vec<(String, String, String, String)> = layer
        .profile
        .skills
        .keys()
        .map(|name| {
            (
                name.clone(),
                "bundled".to_owned(),
                String::new(),
                String::new(),
            )
        })
        .collect();
    for entry in &layer.aos.expect("aos").elsewhere {
        let origin = match entry.origin {
            SkillOrigin::Source => "source",
            SkillOrigin::Catalog => "catalog",
            _ => panic!("bundled skills are in the profile"),
        };
        out.push((
            entry.name.clone(),
            origin.to_owned(),
            entry.source.clone().unwrap_or_default(),
            entry.pin.clone().unwrap_or_default(),
        ));
    }
    out.sort();
    out
}

#[test]
fn canoa_and_dotagents_give_the_same_skills_for_one_overlay() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/skills_from_source.json"
    ))
    .expect("the case is readable");
    let cases: Value = serde_json::from_str(&text).expect("JSON");
    for case in cases["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        for (identity, files) in case["overlays"].as_object().expect("overlays") {
            let got = layer_skills(identity, files.as_object().expect("files"));
            // Canoa holds no catalogs: a name it was not given a source for is "catalog" however the consumer that has
            // catalogs resolved it (resolved, missing or ambiguous).
            let mut want: Vec<(String, String, String, String)> = case["expect"]["skills"]
                .as_array()
                .expect("skills")
                .iter()
                .filter(|skill| skill["overlay"] == identity.as_str())
                .map(|skill| {
                    let origin = match skill["origin"].as_str().expect("origin") {
                        "bundled" => "bundled",
                        "source" => "source",
                        _ => "catalog",
                    };
                    let known = origin == "source";
                    let text = |key: &str| {
                        if known {
                            skill[key].as_str().unwrap_or("").to_owned()
                        } else {
                            String::new()
                        }
                    };
                    (
                        skill["name"].as_str().expect("name").to_owned(),
                        origin.to_owned(),
                        text("source"),
                        text("pin"),
                    )
                })
                .collect();
            want.sort();
            assert_eq!(got, want, "{name}: {identity}");
        }
    }
}

#[test]
fn every_loadout_vector_resolves_to_its_collection_or_is_refused() {
    use canoa_host::Config;
    let vectors = vectors();
    let cases = vectors["loadouts"].as_array().expect("loadouts");
    assert!(cases.len() >= 9, "the vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let parsed = Config::from_str_in(
            case["config"].as_str().expect("config"),
            std::path::Path::new("."),
        );
        let asked = case["loadout"].as_str();
        match (parsed, case.get("resolved"), case.get("error")) {
            (Ok(config), Some(expected), None) => {
                let got = config
                    .local_profiles
                    .resolve_loadout(asked)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                let got = got.map_or(Value::Null, |got| {
                    serde_json::json!({
                        "name": got.name,
                        "overlays": got.overlays,
                        "missing_overlays": got.missing_overlays,
                        "skills": got.skills,
                        "mcp": got.mcp,
                    })
                });
                assert_eq!(&got, expected, "{name}");
            }
            (Ok(config), None, Some(expected)) => {
                assert_eq!(expected["code"], "unknown-loadout", "{name}");
                let error = config
                    .local_profiles
                    .resolve_loadout(asked)
                    .expect_err("refused");
                assert_eq!(expected["name"], error.0.as_str(), "{name}");
            }
            (Err(_), None, Some(expected)) => assert_eq!(expected["code"], "invalid", "{name}"),
            (Ok(_), _, Some(_)) => panic!("{name}: accepted, and the vector refuses it"),
            (Err(error), Some(_), _) => {
                panic!("{name}: refused ({error}), and the vector takes it")
            }
            _ => panic!("{name}: a vector has either resolved or error"),
        }
    }
}

/// The presets of `config` as the vectors write them: each by name, with only the settings it holds.
fn presets_of(config: &canoa_host::Config) -> Value {
    let mut all = serde_json::Map::new();
    for (name, preset) in &config.local_profiles.presets {
        let mut one = serde_json::Map::new();
        for (key, value) in [
            ("source", &preset.source),
            ("model", &preset.model),
            ("effort", &preset.effort),
            ("mode", &preset.mode),
            ("context", &preset.context),
            ("cache_lifetime", &preset.cache_lifetime),
            ("loadout", &preset.loadout),
        ] {
            if let Some(value) = value {
                one.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
        all.insert(name.clone(), Value::Object(one));
    }
    Value::Object(all)
}

#[test]
fn every_preset_vector_reads_to_its_presets_and_its_session_layer_or_is_refused() {
    use canoa_host::Config;
    let vectors = vectors();
    let cases = vectors["presets"].as_array().expect("presets");
    assert!(cases.len() >= 8, "the vectors are all there");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let parsed = Config::from_str_in(
            case["config"].as_str().expect("config"),
            std::path::Path::new("."),
        );
        match (parsed, case.get("presets"), case.get("error")) {
            (Ok(config), Some(expected), None) => {
                assert_eq!(&presets_of(&config), expected, "{name}");
                if let Some(used) = case["use"].as_str() {
                    let preset = config.local_profiles.presets.get(used).expect("used");
                    let own = config.local_profiles.profile.clone().unwrap_or_default();
                    let layers = [
                        Layer::new(LayerId::new(LayerKind::User, ""), None, own),
                        Layer::new(
                            LayerId::new(LayerKind::Session, ""),
                            None,
                            preset.session_layer(),
                        ),
                    ];
                    assert_eq!(&merge(&layers).view(), &case["view"], "{name}");
                }
            }
            (Err(_), None, Some(expected)) => assert_eq!(expected["code"], "invalid", "{name}"),
            (Ok(_), _, Some(_)) => panic!("{name}: accepted, and the vector refuses it"),
            (Err(error), Some(_), _) => {
                panic!("{name}: refused ({error}), and the vector takes it")
            }
            _ => panic!("{name}: a vector has either presets or error"),
        }
    }
}

/// The environment an overlay's layer gives a session on `os`, over a host whose own `PATH`, `PYTHONPATH` and `NODE_PATH`
/// hold one entry each, `host-bin`, `host-py` and `host-node`.
fn environment_of(
    os: &str,
    root: &str,
    files: &[(&str, &str)],
) -> std::collections::BTreeMap<String, String> {
    use canoa_host::profile::{OverlayInput, overlay_layer};
    let tree = coqui_aos::OverlayTree::from_texts(
        files.iter().map(|(path, text)| ((*path).to_owned(), *text)),
    );
    let layer = overlay_layer(&OverlayInput::new("team", root, os, &tree)).expect("applies");
    let host = |name: &str| {
        let host = match name {
            "PATH" => "host-bin",
            "PYTHONPATH" => "host-py",
            "NODE_PATH" => "host-node",
            _ => return None,
        };
        Some(host.to_owned())
    };
    merge(&[layer])
        .environment(host)
        .expect("builds")
        .into_iter()
        .map(|(name, value)| (name, value.expose().to_owned()))
        .collect()
}

#[test]
fn an_overlays_bin_and_lib_go_first_on_the_host_s_path_variables() {
    let files = [
        ("bin/tool", "x"),
        ("lib/python/t.py", "x"),
        ("lib/node/t.js", "x"),
    ];
    let linux = environment_of("linux", "/ov/team", &files);
    assert_eq!(linux["PATH"], "/ov/team/bin:host-bin");
    assert_eq!(linux["PYTHONPATH"], "/ov/team/lib/python:host-py");
    assert_eq!(linux["NODE_PATH"], "/ov/team/lib/node:host-node");
    assert_eq!(linux["TEAM_OVERLAY_ROOT"], "/ov/team");
    let windows = environment_of("windows", "C:\\ov\\team", &files);
    assert_eq!(windows["PATH"], "C:\\ov\\team\\bin;host-bin");
    assert_eq!(windows["PYTHONPATH"], "C:\\ov\\team\\lib\\python;host-py");
    assert_eq!(windows["NODE_PATH"], "C:\\ov\\team\\lib\\node;host-node");
}

#[test]
fn a_declared_path_replaces_its_default_and_an_overlay_with_no_bin_adds_nothing() {
    let none = environment_of("linux", "/ov/team", &[("CONTEXT.md", "x")]);
    assert!(!none.contains_key("PATH") && !none.contains_key("PYTHONPATH"));
    let appended = environment_of(
        "linux",
        "/ov/team",
        &[
            (
                "overlay.cfg",
                "[[paths]]\nvariable = \"PATH\"\npath = \"bin\"\nposition = \"append\"\n",
            ),
            ("bin/tool", "x"),
        ],
    );
    assert_eq!(appended["PATH"], "host-bin:/ov/team/bin");
}

#[test]
fn a_path_addition_that_leaves_the_overlays_own_folder_is_refused() {
    for path in ["../up", "bin/../../up", "/etc", "C:/Windows", "a//b"] {
        let manifest = format!("[[paths]]\nvariable = \"MANPATH\"\npath = \"{path}\"\n");
        let refusal = layer_of(&[("overlay.cfg", &manifest)]).expect_err(path);
        assert!(
            ["invalid", "path"].contains(&refusal.code),
            "{path}: {} {}",
            refusal.code,
            refusal.subject
        );
    }
}

#[test]
fn every_variable_applies_and_the_sensitive_ones_are_listed_in_the_report() {
    for variable in ["LD_LIBRARY_PATH", "PERL5LIB", "HOME", "NODE_OPTIONS"] {
        let manifest = format!("[[paths]]\nvariable = \"{variable}\"\npath = \"lib\"\n");
        layer_of(&[("overlay.cfg", &manifest), ("lib/x", "x")]).expect(variable);
    }
    let layer = layer_of(&[(
        "overlay.cfg",
        "[env]\nNODE_OPTIONS = \"--require x\"\nLD_PRELOAD = \"x.so\"\nHTTPS_PROXY = \"http://proxy.example\"\n\
         ANTHROPIC_BASE_URL = \"https://gateway.example\"\nCODEX_HOME = \"x\"\nPYTHONPATH = \"x\"\n",
    )])
    .expect("every one of them applies");
    for name in [
        "NODE_OPTIONS",
        "LD_PRELOAD",
        "HTTPS_PROXY",
        "ANTHROPIC_BASE_URL",
        "CODEX_HOME",
        "PYTHONPATH",
    ] {
        assert!(layer.profile.env.contains_key(name), "{name}");
    }
    let view = merge(&[layer]).view();
    let listed: Vec<&str> = view["overlays"][0]["non_defaults"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|setting| setting[0] == "env-sensitive")
        .filter_map(|setting| setting[1].as_str())
        .collect();
    for name in [
        "NODE_OPTIONS",
        "LD_PRELOAD",
        "HTTPS_PROXY",
        "ANTHROPIC_BASE_URL",
        "CODEX_HOME",
    ] {
        assert!(listed.contains(&name), "{name} in {listed:?}");
    }
}

#[test]
fn only_the_names_the_specification_reserves_for_the_host_are_refused() {
    for name in ["AOS_MODE", "FOO_OVERLAY_ROOT", "foo_overlay_state"] {
        let refusal = layer_of(&[("env", &format!("{name}=x\n"))]).expect_err(name);
        assert_eq!((refusal.code, refusal.subject.as_str()), ("reserved", name));
        let manifest = format!("[env]\n{name} = \"x\"\n");
        let refusal = layer_of(&[("overlay.cfg", &manifest)]).expect_err(name);
        assert!(
            ["reserved", "invalid"].contains(&refusal.code),
            "{name}: {} {}",
            refusal.code,
            refusal.subject
        );
    }
}

#[test]
fn a_header_is_read_from_the_variable_even_over_plain_http_and_the_report_lists_the_transport() {
    use canoa_host::profile::McpServer;
    let layer = layer_of(&[(
        "overlay.cfg",
        "[[mcp.servers]]\nname = \"remote\"\ntransport = \"streamable-http\"\nurl = \"http://remote.example/mcp\"\n\
         headers_env = { Authorization = \"REMOTE_TOKEN\" }\n",
    )])
    .expect("applies");
    let Some(canoa_host::profile::Change::Set(McpServer::Http { headers, .. })) =
        layer.profile.mcp.get("remote")
    else {
        panic!("an http server");
    };
    assert!(headers.contains_key("Authorization"));
    let aos = layer.aos.as_ref().expect("an AOS layer");
    assert!(
        !aos.not_applied
            .iter()
            .any(|line| line.contains("never sent")),
        "{:?}",
        aos.not_applied
    );
    let view = merge(&[layer]).view();
    let listed: Vec<&str> = view["overlays"][0]["non_defaults"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|setting| setting[0].as_str())
        .collect();
    assert!(listed.contains(&"insecure-transport"), "{listed:?}");
}
