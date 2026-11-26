//! `store/catalog_set` and `providers/switch` through the control channel, over a canoa.cfg of the test's own.

use std::path::PathBuf;

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

fn row(answer: &Value, label: &str) -> Value {
    answer["result"]["catalogs"]
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["label"]
                    .as_str()
                    .is_some_and(|each| each.eq_ignore_ascii_case(label))
            })
        })
        .unwrap_or_else(|| panic!("no catalog {label} in {answer}"))
        .clone()
}

fn set(control: &Control, caller: &Caller, kind: &str, label: &str, entry: Value) -> Value {
    ask(
        control,
        caller,
        "store/catalog_set",
        json!({"type": kind, "label": label, "entry": entry}),
    )
}

#[test]
fn a_catalog_is_replaced_added_turned_off_and_taken_out_through_the_host_and_the_file_says_it() {
    let rig = Rig::new("catalog-set");
    let control = rig.control("[store.skills.catalogs]\nmine = \"owner/repo\"\n");

    // Off: the default stays listed, off.
    let off = set(
        &control,
        &Caller::Owner,
        "overlays",
        "boriken",
        json!(false),
    );
    assert_eq!(row(&off, "boriken")["enabled"], false, "{off}");
    assert_eq!(row(&off, "boriken")["origin"], "default", "{off}");
    assert!(rig.text().contains("boriken = false"), "{}", rig.text());
    assert!(
        rig.text().contains("mine = \"owner/repo\""),
        "another type's catalog is left as it was: {}",
        rig.text()
    );

    // Replaced: an entry with the default's label is the catalog.
    let replaced = set(
        &control,
        &Caller::Owner,
        "overlays",
        "Boriken",
        json!("https://example.org/b.git@main#c.toml"),
    );
    let shown = row(&replaced, "boriken");
    assert_eq!(shown["origin"], "replaced", "{replaced}");
    assert_eq!(shown["enabled"], true);
    assert!(
        !rig.text().contains("boriken = false"),
        "one entry for one label, whatever its case: {}",
        rig.text()
    );

    // Added, with the host's keys.
    let added = set(
        &control,
        &Caller::Owner,
        "overlays",
        "team",
        json!({"provider": "read", "source": "https://example.org/t.git@main#c.toml", "pin": "abc", "ttl": 60}),
    );
    let team = row(&added, "team");
    assert_eq!(team["origin"], "added");
    assert_eq!(
        (team["pin"].as_str(), team["ttl"].as_u64()),
        (Some("abc"), Some(60))
    );

    // Taken out: the entry is gone, and the default is back as the type's own.
    set(&control, &Caller::Owner, "overlays", "team", Value::Null);
    let back = set(&control, &Caller::Owner, "overlays", "boriken", Value::Null);
    assert_eq!(row(&back, "boriken")["origin"], "default", "{back}");
    assert!(
        !rig.text().contains("[store.overlays"),
        "an emptied table is not left behind: {}",
        rig.text()
    );
    assert!(rig.text().contains("mine = \"owner/repo\""));
}

#[test]
fn a_catalog_set_is_refused_for_what_the_host_would_refuse_and_for_the_hosts_own() {
    let rig = Rig::new("catalog-set-refused");
    let control = rig.control("");
    let unknown = set(&control, &Caller::Owner, "toys", "x", json!("a"));
    assert_eq!(unknown["error"]["code"], NOT_FOUND, "{unknown}");
    let bad_label = set(
        &control,
        &Caller::Owner,
        "skills",
        "not a label",
        json!("a"),
    );
    assert!(bad_label["error"].is_object(), "{bad_label}");
    let bad_key = set(
        &control,
        &Caller::Owner,
        "skills",
        "x",
        json!({"provider": "read", "source": "a", "ttl": "soon"}),
    );
    assert!(bad_key["error"].is_object(), "{bad_key}");
    let number = set(&control, &Caller::Owner, "skills", "x", json!(3));
    assert!(number["error"].is_object(), "{number}");
    assert_eq!(rig.text(), "", "nothing was written");
}

#[test]
fn a_key_without_hosting_sets_a_plain_catalog_but_not_one_that_runs_a_program() {
    let rig = Rig::new("catalog-set-guard");
    let control = rig.control("");
    let writer = key("sha256:aa", &[Grant::ConfigWrite, Grant::ConfigRead]);
    let refused = set(
        &control,
        &writer,
        "skills",
        "made",
        json!({"provider": "command", "command": ["gen"]}),
    );
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let plain = set(&control, &writer, "skills", "mine", json!("owner/repo"));
    assert!(plain["result"].is_object(), "{plain}");
    let nothing = key("sha256:bb", &[Grant::ConfigRead]);
    let denied = set(&control, &nothing, "skills", "mine", json!("owner/repo"));
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
}

#[test]
fn a_provider_is_switched_off_and_on_in_host_disabled_and_one_nobody_offers_is_refused() {
    let rig = Rig::new("providers-switch");
    let control = rig.control("[host]\nshell = \"sh\"\n");
    let off = ask(
        &control,
        &Caller::Owner,
        "providers/switch",
        json!({"kind": "account_provider", "id": "claude_accounts", "off": true}),
    );
    assert_eq!(off["result"]["off"], true, "{off}");
    assert_eq!(off["result"]["restart"], true);
    assert_eq!(
        off["result"]["disabled"],
        json!({"account_provider": ["claude_accounts"]})
    );
    let written = rig.text();
    assert!(
        written.contains("[host.disabled]") && written.contains("claude_accounts"),
        "{written}"
    );
    assert!(
        written.contains("shell = \"sh\""),
        "the rest of [host] stays: {written}"
    );

    let listed = ask(&control, &Caller::Owner, "providers/registered", json!({}));
    let rows = listed["result"]["providers"].as_array().expect("rows");
    let claude = rows
        .iter()
        .find(|row| row["kind"] == "account_provider" && row["id"] == "claude_accounts")
        .expect("listed");
    assert_eq!(claude["off"], true, "{claude}");
    assert!(
        claude["kinds"]
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "claude-login")),
        "an account provider says the kinds its accounts have: {claude}"
    );
    let harness = rows
        .iter()
        .find(|row| row["kind"] == "acp_harness" && row["id"] == "claude")
        .expect("the harness is a provider of its own");
    assert_eq!(harness["off"], false, "{harness}");
    assert!(
        listed["result"]["unknown"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let unknown = ask(
        &control,
        &Caller::Owner,
        "providers/switch",
        json!({"kind": "account_provider", "id": "no_such_accounts", "off": true}),
    );
    assert!(
        unknown["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("claude_accounts")),
        "the refusal lists the ones that exist: {unknown}"
    );

    let on = ask(
        &control,
        &Caller::Owner,
        "providers/switch",
        json!({"kind": "account_provider", "id": "claude_accounts", "off": false}),
    );
    assert_eq!(on["result"]["disabled"], json!({}), "{on}");
    assert!(!rig.text().contains("disabled"), "{}", rig.text());
    let denied = ask(
        &control,
        &key("sha256:cc", &[Grant::ConfigWrite]),
        "providers/switch",
        json!({"kind": "account_provider", "id": "claude_accounts", "off": true}),
    );
    assert_eq!(denied["error"]["code"], DENIED, "{denied}");
}

#[test]
fn a_provider_that_no_library_offers_but_the_file_names_is_reported_and_can_be_turned_on() {
    let rig = Rig::new("providers-unknown");
    let control = rig.control(
        "[host.disabled]\naccount_provider = [\"gone_accounts\"]\nno_such_kind = [\"x\"]\n",
    );
    let listed = ask(&control, &Caller::Owner, "providers/registered", json!({}));
    let unknown = listed["result"]["unknown"].as_array().expect("lines");
    assert_eq!(unknown.len(), 2, "{listed}");
    assert!(
        unknown[0]
            .as_str()
            .is_some_and(|line| line.contains("gone_accounts") && line.contains("claude_accounts"))
    );
    assert!(
        unknown[1]
            .as_str()
            .is_some_and(|line| line.contains("no_such_kind"))
    );
    let on = ask(
        &control,
        &Caller::Owner,
        "providers/switch",
        json!({"kind": "account_provider", "id": "gone_accounts", "off": false}),
    );
    assert!(on["result"].is_object(), "{on}");
    assert!(!rig.text().contains("gone_accounts"), "{}", rig.text());
}
