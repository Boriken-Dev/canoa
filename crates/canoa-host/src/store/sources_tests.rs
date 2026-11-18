use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use canoa::store::source::{CatalogSourceProvider, ReadRequest, SourceField, SourceInfo};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use serde_json::json;

use super::Sources;

/// Whether the stand-in extension source fails.
pub(crate) static STANDIN_FAILS: AtomicBool = AtomicBool::new(false);

/// A source an extension would add, serving only the Skills type.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "standin", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct Standin;

impl CatalogSourceProvider for Standin {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new("standin", "A stand-in");
        info.types = vec!["skills".to_owned()];
        let mut team = SourceField::new("team", "Team name", "text").required();
        team.help = Some("Which team's list".to_owned());
        info.fields = vec![team, SourceField::new("seats", "Seats", "number")];
        Ok(info)
    }

    fn read(&self, request: ReadRequest) -> Result<String, ProviderError> {
        if STANDIN_FAILS.load(Ordering::SeqCst) {
            return Err(canoa::store::recipe::internal_error("the stand-in is down"));
        }
        Ok(format!(
            "# {} {} {}\n",
            request.kind, request.label, request.fields
        ))
    }
}

/// A source serving every type, which writes a catalog of overlays.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "everywhere", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct Everywhere;

impl CatalogSourceProvider for Everywhere {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        Ok(SourceInfo::new("everywhere", "Every type"))
    }

    fn read(&self, request: ReadRequest) -> Result<String, ProviderError> {
        if STANDIN_FAILS.load(Ordering::SeqCst) {
            return Err(canoa::store::recipe::internal_error("the stand-in is down"));
        }
        Ok(format!(
            "name = \"From {}\"\n\n[overlays.made]\nsource = \"git+https://example.org/made.git@main\"\n",
            request.label
        ))
    }
}

/// A source serving Skills and MCP servers, which answers in the form each documents.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "lister", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct Lister;

impl CatalogSourceProvider for Lister {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new("lister", "A lister");
        info.types = vec!["skills".to_owned(), "mcps".to_owned()];
        Ok(info)
    }

    fn read(&self, request: ReadRequest) -> Result<String, ProviderError> {
        if STANDIN_FAILS.load(Ordering::SeqCst) {
            return Err(canoa::store::recipe::internal_error("the stand-in is down"));
        }
        Ok(if request.kind == "skills" {
            json!({"skills": [{"name": "from-extension", "description": "Listed by an extension.", "source": "owner/repo"}]})
        } else {
            json!({"servers": [{"server": {"name": "io.example/ext", "description": "Listed by an extension."}}]})
        }
        .to_string())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-standin-sources",
    version = "0",
    providers = [Standin, Everywhere, Lister]
);

/// The built-in sources and the stand-in, as a host with that extension loaded holds them.
pub(crate) fn with_standin() -> Sources {
    let mut registry = Registry::new("canoa", "0");
    let loading = registry
        .register_local("standin", library)
        .expect("registers");
    assert!(loading.loaded().is_some());
    let mut sources = Sources::built_in();
    sources
        .add(Some("standin_ext"), Arc::new(registry))
        .expect("added");
    sources
}

fn run(fields: serde_json::Value) -> Result<String, String> {
    Sources::built_in().read(
        "command",
        ReadRequest::new("overlays", "made", fields.to_string()),
    )
}

#[cfg(windows)]
fn say(text: &str) -> Vec<String> {
    vec!["cmd".into(), "/C".into(), format!("echo {text}")]
}

#[cfg(not(windows))]
fn say(text: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), format!("echo {text}")]
}

#[cfg(windows)]
fn fail(text: &str) -> Vec<String> {
    vec![
        "cmd".into(),
        "/C".into(),
        format!("echo {text} 1>&2 & exit 3"),
    ]
}

#[cfg(not(windows))]
fn fail(text: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), format!("echo {text} >&2; exit 3")]
}

#[test]
fn read_and_command_are_built_in_and_serve_every_type_with_the_forms_they_ask() {
    let sources = Sources::built_in();
    let listed: Vec<(&SourceInfo, Option<&str>)> = sources.list().collect();
    let ids: Vec<&str> = listed.iter().map(|(info, _)| info.id.as_str()).collect();
    assert_eq!(ids, ["command", "read"]);
    assert!(listed.iter().all(|(_, extension)| extension.is_none()));

    let read = sources.info("read").expect("read");
    assert!(
        read.serves("overlays")
            && read.serves("skills")
            && read.serves("mcps")
            && read.serves("zzz")
    );
    assert_eq!(read.fields[0].name, "source");
    assert!(read.fields[0].required);

    let command = sources.info("command").expect("command");
    assert!(command.types.is_empty(), "it serves every type");
    assert!(
        [
            "overlays",
            "extensions",
            "skills",
            "mcps",
            "a_type_an_extension_added"
        ]
        .iter()
        .all(|kind| command.serves(kind))
    );
    let names: Vec<&str> = command
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["command", "cwd", "file", "timeout"]);
    assert_eq!(command.fields[0].shape, "list");
    assert!(command.fields[0].required);
    assert_eq!(command.fields[3].default.as_deref(), Some("60"));
}

#[test]
fn a_source_an_extension_adds_is_listed_with_its_extension_and_the_form_it_describes() {
    let sources = with_standin();
    let found = sources
        .list()
        .find(|(info, _)| info.id == "standin")
        .expect("listed");
    assert_eq!(found.1, Some("standin_ext"));
    assert_eq!(found.0.types, ["skills"]);
    assert_eq!(found.0.fields[0].label, "Team name");
    assert_eq!(found.0.fields[0].help.as_deref(), Some("Which team's list"));
    assert!(found.0.fields[0].required && !found.0.fields[1].required);
    assert!(
        sources
            .info("everywhere")
            .is_some_and(|info| info.serves("overlays"))
    );
}

#[test]
fn a_provider_is_asked_only_for_a_type_it_serves_and_an_id_taken_twice_is_refused() {
    let sources = with_standin();
    let request = |kind: &str| ReadRequest::new(kind, "mine", json!({"team": "a"}).to_string());
    assert!(
        sources
            .read("standin", request("skills"))
            .expect("served")
            .starts_with("# skills mine")
    );
    let refused = sources
        .read("standin", request("overlays"))
        .expect_err("not served");
    assert!(refused.contains("does not serve"), "{refused}");
    assert!(
        sources
            .read("nobody", request("skills"))
            .expect_err("none")
            .contains("no catalog source")
    );

    let mut taken = Sources::default();
    let mut first = Registry::new("canoa", "0");
    first.register_local("again", library).expect("registers");
    taken.add(Some("one"), Arc::new(first)).expect("first");
    let mut second = Registry::new("canoa", "0");
    second.register_local("again", library).expect("registers");
    let refused = taken.add(Some("two"), Arc::new(second)).expect_err("taken");
    assert!(refused.contains("already offered"), "{refused}");
}

#[test]
fn the_command_source_answers_with_what_the_program_prints() {
    let said = run(json!({"command": say("name = made")})).expect("printed");
    assert_eq!(said.trim(), "name = made");
}

#[test]
fn the_command_source_reads_a_file_the_program_names_from_its_folder() {
    let dir = std::env::temp_dir().join(format!("canoa-source-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("out.toml"), "name = \"from a file\"\n").expect("file");
    let folder = dir.to_string_lossy().into_owned();
    let said = run(json!({"command": say("ignored"), "cwd": folder, "file": "out.toml"}))
        .expect("read the file");
    assert_eq!(said.trim(), "name = \"from a file\"");
    let _ = std::fs::remove_file(dir.join("out.toml"));
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn a_program_that_fails_is_reported_with_the_end_of_what_it_said() {
    let why = run(json!({"command": fail("it broke here")})).expect_err("failed");
    assert!(why.contains("it broke here"), "{why}");
    assert!(why.contains("ended with"), "{why}");
}

#[test]
fn a_program_that_is_missing_or_names_no_command_or_runs_too_long_is_reported() {
    let missing = run(json!({"command": ["canoa-no-such-program-anywhere"]})).expect_err("none");
    assert!(missing.contains("could not start"), "{missing}");
    assert!(
        run(json!({}))
            .expect_err("no command")
            .contains("program, then its arguments")
    );
    assert!(run(json!({"command": []})).is_err());
    assert!(run(json!({"command": say("x"), "timeout": "soon"})).is_err());
    #[cfg(windows)]
    let slow = vec!["ping", "-n", "30", "127.0.0.1"];
    #[cfg(not(windows))]
    let slow = vec!["sleep", "30"];
    let why = run(json!({"command": slow, "timeout": 1})).expect_err("stopped");
    assert!(why.contains("was stopped after 1 s"), "{why}");
}

#[test]
fn the_read_source_reads_a_file_on_this_computer() {
    let dir = std::env::temp_dir().join(format!("canoa-source-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("catalog.toml");
    std::fs::write(&file, "name = \"here\"\n").expect("file");
    let fields = json!({"source": file.to_string_lossy()});
    let text = Sources::built_in()
        .read(
            "read",
            ReadRequest::new("overlays", "x", fields.to_string()),
        )
        .expect("read");
    assert_eq!(text, "name = \"here\"\n");
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
}
