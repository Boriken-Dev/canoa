//! `store/catalog`: the Skills and MCP servers catalogs read through every kind of source, over stand-ins and bare local
//! repositories. Nothing here reaches the network.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::overlays_tests::{ask, change, dir, key, repo};
use super::*;
use crate::control::registry::Registry;
use crate::store::index::StoreIndex;
use crate::store::programs::Programs;
use crate::store::shelf::Shelf;
use crate::store::sources::tests::{standin_down, with_standin};
use crate::store::tools::{Reply, ToolsLimits, Transport};

#[derive(Debug, Default)]
struct Canned {
    asked: Mutex<Vec<String>>,
}

impl Transport for Canned {
    fn get(&self, url: &str, _limits: &ToolsLimits) -> Result<Reply, String> {
        self.asked.lock().expect("lock").push(url.to_owned());
        let body = json!({
            "servers": [
                {"server": {"name": "io.example/files", "description": "Reads files"}},
                {"server": {"name": "io.example/mail", "description": "Sends mail"}},
            ],
            "metadata": {"nextCursor": "more"},
        });
        Ok(Reply {
            status: 200,
            location: None,
            body: Box::new(std::io::Cursor::new(body.to_string().into_bytes())),
        })
    }
}

struct Rig {
    tmp: PathBuf,
    config: PathBuf,
    transport: Arc<Canned>,
}

impl Rig {
    fn new(name: &str) -> Self {
        let tmp = dir(name);
        let config = tmp.join("cfg").join("canoa.cfg");
        std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
        Self {
            tmp,
            config,
            transport: Arc::new(Canned::default()),
        }
    }

    fn control(&self, cfg: &str) -> Control {
        std::fs::write(&self.config, cfg).expect("config");
        let programs = Programs::new(
            Shelf::unchecked(self.tmp.join("programs")),
            crate::store::registry::built_in().expect("the built-in stores"),
        );
        Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_files(Files::new(
            None,
            Vec::new(),
            Some(self.config.clone()),
            None,
            UploadLimits::default(),
        ))
        .with_overlays(Overlays::new(
            Some(self.tmp.join("cfg").join("overlays")),
            OverlayLimits::default(),
        ))
        .with_index(StoreIndex::new(
            Some(self.tmp.join("home").join("cache").join("store")),
            Duration::from_secs(3600),
            1 << 20,
        ))
        .with_registry(Registry::new(None).with_transport(self.transport.clone()))
        .with_stores(Stores::new(Arc::new(programs), Vec::new()).with_sources(with_standin()))
    }

    fn catalog(&self, control: &Control, kind: &str, label: &str, more: Value) -> Value {
        let mut params = json!({"type": kind, "label": label});
        params
            .as_object_mut()
            .expect("object")
            .extend(more.as_object().expect("object").clone());
        ask(control, &Caller::Owner, "store/catalog", params)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

#[cfg(windows)]
fn show(file: &str) -> String {
    format!("[\"cmd\", \"/C\", \"type\", \"{file}\"]")
}
#[cfg(not(windows))]
fn show(file: &str) -> String {
    format!("[\"cat\", \"{file}\"]")
}

fn folder_of(rig: &Rig) -> (PathBuf, String) {
    let folder = rig.tmp.join("gen");
    std::fs::create_dir_all(&folder).expect("dir");
    let cwd = folder.to_string_lossy().replace('\\', "/");
    (folder, cwd)
}

fn names(answer: &Value, key: &str) -> Vec<String> {
    answer["result"][key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} in {answer}"))
        .iter()
        .map(|each| {
            each["name"]
                .as_str()
                .or_else(|| each["server"]["name"].as_str())
                .expect("name")
                .to_owned()
        })
        .collect()
}

const LISTING: &str = r#"{"skills": [
  {"name": "pdf", "description": "Fill forms.", "source": "anthropics/skills", "path": "skills/pdf"},
  {"name": "lint", "description": "Lint it.", "source": "owner/tools@v2.0.0"}
]}"#;

const PAGE: &str = r#"{"servers": [
  {"server": {"name": "io.example/files", "description": "Reads files"}},
  {"server": {"name": "io.example/mail", "description": "Sends mail"}}
], "metadata": {"nextCursor": "p2"}}"#;

#[test]
fn a_program_generates_a_skills_catalog_and_a_failing_one_says_why() {
    let rig = Rig::new("read-skills-command");
    let (folder, cwd) = folder_of(&rig);
    std::fs::write(folder.join("skills.json"), LISTING).expect("listing");
    let control = rig.control(&format!(
        "[store.skills.catalogs.made]\nprovider = \"command\"\ncommand = {}\ncwd = \"{cwd}\"\n",
        show("skills.json")
    ));
    let answer = rig.catalog(&control, "skills", "made", json!({}));
    assert_eq!(names(&answer, "skills"), ["pdf", "lint"], "{answer}");
    assert_eq!(answer["result"]["provider"], "command");
    assert_eq!(answer["result"]["label"], "made");
    assert_eq!(answer["result"]["type"], "skills");
    assert_eq!(answer["result"]["commit"], Value::Null);
    let pdf = &answer["result"]["skills"][0];
    assert_eq!(pdf["source"], "anthropics/skills");
    assert_eq!(pdf["path"], "skills/pdf");
    let lint = &answer["result"]["skills"][1];
    assert_eq!(lint["ref"], "v2.0.0", "{lint}");

    // What it prints is not a listing: its provider is named in the reason.
    std::fs::write(folder.join("skills.json"), "{\"servers\": []}").expect("listing");
    let wrong = rig.catalog(&control, "skills", "made", json!({}));
    assert_eq!(wrong["error"]["code"], REFUSED, "{wrong}");
    assert!(
        wrong["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("command gave no skills listing")),
        "{wrong}"
    );

    // The program fails: the reason is what it ended with.
    std::fs::remove_file(folder.join("skills.json")).expect("removed");
    let failed = rig.catalog(&control, "skills", "made", json!({}));
    assert!(
        failed["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("ended with")),
        "{failed}"
    );
}

#[test]
fn a_program_generates_an_mcp_registry_list_which_a_search_narrows() {
    let rig = Rig::new("read-mcps-command");
    let (folder, cwd) = folder_of(&rig);
    std::fs::write(folder.join("servers.json"), PAGE).expect("page");
    let control = rig.control(&format!(
        "[store.mcps.catalogs.corp]\nprovider = \"command\"\ncommand = {}\ncwd = \"{cwd}\"\n",
        show("servers.json")
    ));
    let all = rig.catalog(&control, "mcps", "corp", json!({}));
    assert_eq!(
        names(&all, "servers"),
        ["io.example/files", "io.example/mail"],
        "{all}"
    );
    assert_eq!(all["result"]["metadata"]["nextCursor"], "p2");
    let narrowed = rig.catalog(&control, "mcps", "corp", json!({"search": "mail"}));
    assert_eq!(
        names(&narrowed, "servers"),
        ["io.example/mail"],
        "{narrowed}"
    );
    assert!(rig.transport.asked.lock().expect("lock").is_empty());
}

#[test]
fn an_extensions_source_serves_the_types_its_description_lists_and_no_other() {
    let rig = Rig::new("read-extension");
    let control = rig.control(
        "[store.skills.catalogs.theirs]\nprovider = \"lister\"\n\n\
         [store.mcps.catalogs.theirs]\nprovider = \"lister\"\n\n\
         [store.mcps.catalogs.skillsonly]\nprovider = \"standin\"\nteam = \"a\"\n",
    );
    standin_down("theirs", false);
    let skills = rig.catalog(&control, "skills", "theirs", json!({}));
    assert_eq!(names(&skills, "skills"), ["from-extension"], "{skills}");
    assert_eq!(skills["result"]["provider"], "lister");
    let servers = rig.catalog(&control, "mcps", "theirs", json!({}));
    assert_eq!(names(&servers, "servers"), ["io.example/ext"], "{servers}");

    let refused = rig.catalog(&control, "mcps", "skillsonly", json!({}));
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("does not serve")),
        "{refused}"
    );
    standin_down("theirs", true);
    let down = rig.catalog(&control, "skills", "theirs", json!({}));
    standin_down("theirs", false);
    assert!(
        down["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("the stand-in is down")),
        "{down}"
    );
}

fn skill_md(name: &str) -> String {
    format!("---\nname: {name}\ndescription: The {name} skill.\n---\n# {name}\n")
}

fn file_url(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

#[test]
fn the_read_source_scans_a_repository_for_its_skills() {
    let rig = Rig::new("read-skills-scan");
    let pdf = skill_md("pdf");
    let docx = skill_md("docx");
    let upstream = repo(
        &rig.tmp,
        "skills",
        &[
            ("skills/pdf/SKILL.md", &pdf),
            ("skills/docx/SKILL.md", &docx),
        ],
    );
    let url = file_url(&upstream);
    let control = rig.control(&format!(
        "[store.skills.catalogs]\nanthropics = false\nmine = \"{url}\"\n"
    ));
    let all = rig.catalog(&control, "skills", "mine", json!({}));
    assert_eq!(names(&all, "skills"), ["docx", "pdf"], "{all}");
    assert_eq!(all["result"]["commit"].as_str().map(str::len), Some(40));
    assert_eq!(all["result"]["provider"], "read");
    assert_eq!(all["result"]["skills"][0]["source"], url);
    assert_eq!(all["result"]["skills"][0]["path"], "skills/docx");
}

#[test]
fn the_read_source_reads_a_registry_by_address_with_the_search_and_cursor_and_a_git_file() {
    let rig = Rig::new("read-mcps");
    let upstream = repo(&rig.tmp, "servers", &[("servers.json", PAGE)]);
    let git = format!("{}@main#servers.json", file_url(&upstream));
    let control = rig.control(&format!(
        "[store.mcps.catalogs]\ncorp = \"https://registry.example/\"\nlocal = \"{git}\"\n"
    ));

    // The official registry is always there, under its own label.
    let official = rig.catalog(
        &control,
        "mcps",
        "official",
        json!({"search": "file s", "cursor": "a b"}),
    );
    assert_eq!(
        names(&official, "servers"),
        ["io.example/files"],
        "{official}"
    );
    assert_eq!(official["result"]["metadata"]["nextCursor"], "more");
    let corp = rig.catalog(&control, "mcps", "corp", json!({}));
    assert_eq!(names(&corp, "servers").len(), 2, "{corp}");
    assert_eq!(
        *rig.transport.asked.lock().expect("lock"),
        [
            "https://registry.modelcontextprotocol.io/v0.1/servers?limit=30&cursor=a%20b&search=file%20s",
            "https://registry.example/v0.1/servers?limit=30",
        ]
    );

    let local = rig.catalog(&control, "mcps", "local", json!({}));
    assert_eq!(
        names(&local, "servers"),
        ["io.example/files", "io.example/mail"],
        "{local}"
    );
    // It is read again at the commit the ref names now.
    change(
        &upstream,
        &[(
            "servers.json",
            r#"{"servers": [{"server": {"name": "io.example/new"}}]}"#,
        )],
        "second",
    );
    let again = rig.catalog(&control, "mcps", "local", json!({}));
    assert_eq!(names(&again, "servers"), ["io.example/new"], "{again}");
}

#[test]
fn a_catalog_that_is_not_there_a_type_read_elsewhere_and_a_key_without_the_grant_are_refused() {
    let rig = Rig::new("read-refusals");
    let control = rig.control("[store.mcps.catalogs]\nofficial = \"https://elsewhere.example/\"\n");
    let missing = rig.catalog(&control, "skills", "nobody", json!({}));
    assert_eq!(missing["error"]["code"], NOT_FOUND, "{missing}");
    for kind in ["overlays", "extensions"] {
        let elsewhere = rig.catalog(&control, kind, "boriken", json!({}));
        assert_eq!(elsewhere["error"]["code"], -32602, "{kind}: {elsewhere}");
    }
    let unknown = rig.catalog(&control, "toys", "x", json!({}));
    assert_eq!(unknown["error"]["code"], NOT_FOUND, "{unknown}");

    // `official` is a removable default, so `[store.mcps.catalogs] official = ...` replaces it.
    let official = ask(
        &control,
        &Caller::Owner,
        "store/catalogs",
        json!({"type": "mcps"}),
    );
    let rows = official["result"]["types"][0]["catalogs"]
        .as_array()
        .expect("rows");
    assert_eq!(rows[0]["label"], "official");
    assert_eq!(rows[0]["source"], "https://elsewhere.example/");

    let reader = key("sha256:aa", &[Grant::OverlayRead]);
    let denied = ask(
        &control,
        &reader,
        "store/catalog",
        json!({"type": "mcps", "label": "official"}),
    );
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
    let writer = key("sha256:bb", &[Grant::OverlayWrite]);
    let allowed = ask(
        &control,
        &writer,
        "store/catalog",
        json!({"type": "mcps", "label": "official"}),
    );
    assert!(allowed["result"].is_object(), "{allowed}");
}
