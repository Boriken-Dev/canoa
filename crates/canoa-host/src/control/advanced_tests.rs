//! `config/advanced` and `config/advanced_set` through the control channel: the tables no dedicated page edits, listed with their
//! schemas and made, changed and taken out in the person's own canoa.cfg, with what a lower layer gives shown and left alone, and the
//! same methods and the page methods over a store that is no file.

use std::path::PathBuf;
use std::sync::Arc;

use canoa::config_store::{ConfigStore as _, Key, MemoryStore, json_of};
use serde_json::{Value, json};

use super::overlays_tests::{ask, dir, key};
use super::*;

struct Rig {
    tmp: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        let tmp = dir(name);
        let config = tmp.join("cfg").join("canoa.cfg");
        std::fs::create_dir_all(config.parent().expect("parent")).expect("dir");
        Self { tmp, config }
    }

    fn control(&self, cfg: &str) -> Control {
        std::fs::write(&self.config, cfg).expect("config");
        control_with(Files::new(
            None,
            Vec::new(),
            Some(self.config.clone()),
            None,
            UploadLimits::default(),
        ))
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.config).expect("canoa.cfg")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn control_with(files: Files) -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files)
}

fn hosting() -> Caller {
    key(
        "sha256:aa",
        &[Grant::Hosting, Grant::ConfigRead, Grant::ConfigWrite],
    )
}

fn table(answer: &Value, path: &str) -> Value {
    answer["result"]["tables"]
        .as_array()
        .and_then(|tables| {
            tables.iter().find(|table| {
                table["path"].as_array().is_some_and(|keys| {
                    keys.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(".")
                        == path
                })
            })
        })
        .unwrap_or_else(|| panic!("no table {path} in {answer}"))
        .clone()
}

#[test]
fn every_table_is_listed_with_a_schema_of_the_settings_the_host_reads() {
    let rig = Rig::new("advanced-list");
    let control = rig.control("# mine\n[desktop]\nstart_hidden = true\n");
    let listed = ask(&control, &hosting(), "config/advanced", json!({}));
    let web = table(&listed, "host.web");
    assert_eq!(web["shape"], "flat");
    let office = &web["schema"]["properties"]["office"];
    assert_eq!(office["type"], "boolean", "{web}");
    assert_eq!(office["title"], "Office");
    assert_eq!(web["schema"]["properties"]["hosts"]["type"], "array");
    assert_eq!(table(&listed, "host.tools")["shape"], "entries");
    assert_eq!(table(&listed, "host.openers")["shape"], "list");
    assert_eq!(
        table(&listed, "host.tools")["schema"]["properties"]["prompt"]["x-json"],
        true,
        "a free-form member says so"
    );
    assert!(web["own"].is_null() && web["inherited"] == json!({}));
}

#[test]
fn a_key_without_hosting_may_neither_list_nor_write() {
    let rig = Rig::new("advanced-grant");
    let control = rig.control("");
    let plain = key("sha256:bb", &[Grant::ConfigWrite, Grant::ConfigRead]);
    for (method, params) in [
        ("config/advanced", json!({})),
        (
            "config/advanced_set",
            json!({"path": ["host", "web"], "value": {"office": true}}),
        ),
    ] {
        let refused = ask(&control, &plain, method, params);
        assert_eq!(refused["error"]["code"], DENIED, "{method}: {refused}");
    }
    assert_eq!(rig.text(), "");
}

#[test]
fn a_table_of_settings_is_made_changed_and_taken_out_and_the_rest_of_the_file_stays() {
    let rig = Rig::new("advanced-flat");
    let control = rig.control("# mine\n[host.agents.a]\nprogram = \"x\"\n");
    let set = |value: Value| {
        ask(
            &control,
            &hosting(),
            "config/advanced_set",
            json!({"path": ["host", "web"], "value": value}),
        )
    };
    let made = set(json!({"listen": "127.0.0.1:7800", "office": true, "hosts": ["a.example"]}));
    assert!(made["result"]["restart"].as_bool().is_some(), "{made}");
    let text = rig.text();
    assert!(text.starts_with("# mine\n"), "{text}");
    assert!(
        text.contains("[host.agents.a]") && text.contains("office = true"),
        "{text}"
    );
    let listed = ask(&control, &hosting(), "config/advanced", json!({}));
    assert_eq!(
        table(&listed, "host.web")["own"]["listen"],
        "127.0.0.1:7800"
    );

    let changed = set(json!({"listen": "127.0.0.1:7801"}));
    assert!(changed["result"].is_object(), "{changed}");
    let text = rig.text();
    assert!(
        text.contains("7801") && !text.contains("office"),
        "a setting left out is taken out: {text}"
    );
    assert!(text.contains("[host.agents.a]"), "{text}");

    let removed = set(Value::Null);
    assert!(removed["result"].is_object(), "{removed}");
    let text = rig.text();
    assert!(
        !text.contains("host.web") && !text.contains("7801"),
        "{text}"
    );
    assert!(text.contains("[host.agents.a]"), "{text}");
}

#[test]
fn an_entry_is_added_edited_and_removed_by_name() {
    let rig = Rig::new("advanced-entry");
    let control = rig.control("[desktop]\nstart_hidden = true\n");
    let entry = |value: Value| {
        ask(
            &control,
            &hosting(),
            "config/advanced_set",
            json!({"path": ["host", "tools", "pi"], "value": value}),
        )
    };
    let made = entry(json!({
        "program": "pi",
        "runs": "session",
        "unasked": "edits files",
        "prompt": {"type": "prompt", "message": "{prompt}"},
    }));
    assert!(made["result"].is_object(), "{made}");
    assert!(rig.text().contains("[host.tools.pi]"), "{}", rig.text());
    let listed = ask(&control, &hosting(), "config/advanced", json!({}));
    assert_eq!(table(&listed, "host.tools")["own"]["pi"]["program"], "pi");

    let edited = entry(json!({
        "program": "pi2",
        "runs": "session",
        "unasked": "edits files",
        "prompt": {"type": "prompt", "message": "{prompt}"},
    }));
    assert!(edited["result"].is_object(), "{edited}");
    assert!(rig.text().contains("pi2"));

    let removed = entry(Value::Null);
    assert!(removed["result"].is_object(), "{removed}");
    assert!(!rig.text().contains("host.tools"), "{}", rig.text());
    assert!(rig.text().contains("start_hidden"));
}

#[test]
fn a_list_of_tables_is_written_whole() {
    let rig = Rig::new("advanced-list-shape");
    let control = rig.control("");
    let set = |value: Value| {
        ask(
            &control,
            &hosting(),
            "config/advanced_set",
            json!({"path": ["host", "openers"], "value": value}),
        )
    };
    let made =
        set(json!([{"id": "code", "label": "VS Code", "program": "code", "args": ["{path}"]}]));
    assert!(made["result"].is_object(), "{made}");
    assert!(rig.text().contains("VS Code"), "{}", rig.text());
    let removed = set(Value::Null);
    assert!(removed["result"].is_object(), "{removed}");
    assert!(!rig.text().contains("VS Code"));
}

#[test]
fn a_refused_write_changes_nothing_and_says_why() {
    let rig = Rig::new("advanced-refused");
    let before = "[desktop]\nstart_hidden = true\n";
    let control = rig.control(before);
    let set = |path: Value, value: Value| {
        ask(
            &control,
            &hosting(),
            "config/advanced_set",
            json!({"path": path, "value": value}),
        )
    };
    let unknown = set(json!(["host", "web"]), json!({"no_such_setting": 1}));
    assert!(
        unknown["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("no setting no_such_setting")),
        "{unknown}"
    );
    let wrong = set(json!(["host", "web"]), json!({"office": "yes"}));
    assert!(
        wrong["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("office")),
        "{wrong}"
    );
    let elsewhere = set(json!(["host", "agents", "x"]), json!({"program": "x"}));
    assert_eq!(
        elsewhere["error"]["code"], DENIED,
        "a table another page edits is not offered here: {elsewhere}"
    );
    let refused_by_the_host = set(json!(["host", "web"]), json!({"max_connections": 0}));
    assert!(
        refused_by_the_host["error"].is_object(),
        "the host's own reader says no: {refused_by_the_host}"
    );
    assert_eq!(rig.text(), before);
}

#[test]
fn a_check_writes_nothing_and_a_stale_revision_is_refused() {
    let rig = Rig::new("advanced-check");
    let before = "[desktop]\nstart_hidden = true\n";
    let control = rig.control(before);
    let checked = ask(
        &control,
        &hosting(),
        "config/advanced_set",
        json!({"path": ["host", "web"], "value": {"office": true}, "check": true}),
    );
    assert!(checked["result"].is_object(), "{checked}");
    assert_eq!(rig.text(), before);
    let revision =
        ask(&control, &hosting(), "config/advanced", json!({}))["result"]["revision"].clone();
    std::fs::write(&rig.config, "[desktop]\nstart_hidden = false\n").expect("another writer");
    let stale = ask(
        &control,
        &hosting(),
        "config/advanced_set",
        json!({"path": ["host", "web"], "value": {"office": true}, "revision": revision}),
    );
    assert_eq!(stale["error"]["code"], super::files::CHANGED, "{stale}");
}

#[test]
fn what_a_lower_layer_gives_is_shown_with_its_file_and_is_not_taken_out_from_here() {
    let rig = Rig::new("advanced-layers");
    let low = rig.tmp.join("cfg").join("installed.cfg");
    std::fs::write(&low, "[host.tools.shared]\nprogram = \"shared\"\nruns = \"message\"\nunasked = \"x\"\n[host.web]\nmode = \"local\"\n").expect("low");
    std::fs::write(&rig.config, "[desktop]\nstart_hidden = true\n").expect("own");
    let layers = crate::config::Layers::from_files(vec![low, rig.config.clone()]);
    let files = Files::new(
        None,
        Vec::new(),
        Some(rig.config.clone()),
        None,
        UploadLimits::default(),
    )
    .with_store(Arc::new(crate::config_store::FileStore::of(&layers)));
    let control = control_with(files);
    let listed = ask(&control, &hosting(), "config/advanced", json!({}));
    let tools = table(&listed, "host.tools");
    assert!(tools["own"].is_null());
    assert!(
        tools["inherited"]["shared"]["layer"]
            .as_str()
            .is_some_and(|layer| layer.contains("installed.cfg")),
        "{tools}"
    );
    assert_eq!(
        table(&listed, "host.web")["inherited"]["mode"]["value"],
        "local"
    );
    let removal = ask(
        &control,
        &hosting(),
        "config/advanced_set",
        json!({"path": ["host", "tools", "shared"], "value": null}),
    );
    assert!(
        removal["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("installed.cfg") && why.contains("remove it there")),
        "{removal}"
    );
    // A form that leaves out what only a lower layer gives does not try to take it out.
    let saved = ask(
        &control,
        &hosting(),
        "config/advanced_set",
        json!({"path": ["host", "web"], "value": {"office": true}}),
    );
    assert!(saved["result"].is_object(), "{saved}");
    let own = std::fs::read_to_string(&rig.config).expect("own");
    assert!(
        own.contains("office = true") && !own.contains("mode"),
        "{own}"
    );
}

#[test]
fn the_page_methods_and_the_advanced_ones_work_over_a_store_that_is_no_file() {
    let store = Arc::new(MemoryStore::new());
    let control = control_with(
        Files::new(None, Vec::new(), None, None, UploadLimits::default()).with_store(store.clone()),
    );
    let caller = hosting();
    let set = ask(
        &control,
        &caller,
        "config/set",
        json!({"section": "sessions", "value": {"keep": 7}}),
    );
    assert!(set["result"].is_object(), "{set}");
    let got = ask(
        &control,
        &caller,
        "config/get",
        json!({"section": "sessions"}),
    );
    assert_eq!(got["result"]["value"]["keep"], 7, "{got}");
    let advanced = ask(
        &control,
        &caller,
        "config/advanced_set",
        json!({"path": ["host", "web"], "value": {"office": true}}),
    );
    assert!(advanced["result"].is_object(), "{advanced}");
    let held = store.get(Key::new(["host", "web", "office"])).expect("get");
    assert_eq!(held.value.as_ref().and_then(json_of), Some(json!(true)));
    let listed = ask(&control, &caller, "config/advanced", json!({}));
    assert_eq!(
        table(&listed, "host.web")["own"]["office"],
        true,
        "{listed}"
    );
    let removed = ask(
        &control,
        &caller,
        "config/set",
        json!({"section": "sessions", "value": null}),
    );
    assert!(removed["result"].is_object(), "{removed}");
    assert!(
        store
            .get(Key::new(["sessions"]))
            .expect("get")
            .value
            .is_none()
    );
}

#[test]
fn the_layers_are_read_and_ops_are_made_in_the_own_layer_through_the_store() {
    let rig = Rig::new("advanced-config-change");
    let low = rig.tmp.join("cfg").join("installed.cfg");
    std::fs::write(&low, "[sessions]\nkeep = 500\n").expect("low");
    std::fs::write(&rig.config, "# mine\n[desktop]\nstart_hidden = true\n").expect("own");
    let layers = crate::config::Layers::from_files(vec![low, rig.config.clone()]);
    let files = Files::new(
        None,
        Vec::new(),
        Some(rig.config.clone()),
        None,
        UploadLimits::default(),
    )
    .with_store(Arc::new(crate::config_store::FileStore::of(&layers)));
    let control = control_with(files);
    let read = ask(&control, &hosting(), "config/layers", json!({}));
    let found = read["result"]["layers"].as_array().expect("layers");
    assert_eq!(found.len(), 2, "{read}");
    assert_eq!(found[0]["document"]["sessions"]["keep"], 500);
    assert_eq!(found[1]["writable"], true);
    assert_eq!(found[1]["document"]["desktop"]["start_hidden"], true);

    let changed = ask(
        &control,
        &hosting(),
        "config/change",
        json!({"ops": [
            {"path": ["sessions"], "value": {"keep": 7}},
            {"path": ["desktop", "start_hidden"]},
        ], "revision": read["result"]["revision"]}),
    );
    assert!(changed["result"]["revision"].is_string(), "{changed}");
    let text = rig.text();
    assert!(
        text.starts_with("# mine\n") && text.contains("keep = 7") && !text.contains("start_hidden"),
        "{text}"
    );
    let stale = ask(
        &control,
        &hosting(),
        "config/change",
        json!({"ops": [{"path": ["sessions"]}], "revision": read["result"]["revision"]}),
    );
    assert_eq!(stale["error"]["code"], super::files::CHANGED, "{stale}");
    let plain = key("sha256:bb", &[Grant::ConfigWrite, Grant::ConfigRead]);
    for (method, params) in [
        ("config/layers", json!({})),
        ("config/change", json!({"ops": []})),
    ] {
        assert_eq!(
            ask(&control, &plain, method, params)["error"]["code"],
            DENIED,
            "{method}"
        );
    }
}
