//! Every key the host's reader accepts has a page: the generic editor's tables (`control::advanced::TABLES`), a dedicated page
//! (`DEDICATED`) or a reason the host owns it (`OWNED`). The keys are read from the reader itself: a table with
//! `deny_unknown_fields` names the keys it knows when it refuses one.

use crate::control::advanced::{DEDICATED, OWNED, Shape, TABLES};

/// The keys the host's reader names for the struct `name` of `raw.rs`: its fields, but for `rest`, which holds what the reader
/// passes on whole.
fn keys_of(name: &str) -> Vec<String> {
    let source = include_str!("raw.rs");
    let start = source
        .find(&format!("struct {name} {{"))
        .unwrap_or_else(|| panic!("raw.rs has no struct {name}"));
    let body = &source[start..];
    let body = &body[..body.find("\n}").expect("the struct ends")];
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("pub(super) "))
        .filter_map(|field| field.split_once(':'))
        .map(|(field, _)| field.trim().trim_start_matches("r#").to_owned())
        .filter(|field| field != "rest")
        .collect()
}

/// Every table the host reads, as the generic editor and the inventory name it, with the keys it accepts.
fn tables() -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("(top level)", keys_of("RawFile")),
        ("host", keys_of("RawHost")),
        ("assist_harness", keys_of("RawAssistHarness")),
        ("assist_pool", keys_of("RawAssistPool")),
        ("schedules", keys_of("RawSchedule")),
        ("settings", keys_of("RawSettings")),
        ("settings.power", keys_of("RawPower")),
        ("permissions", keys_of("RawPermissions")),
        ("permissions.approvers.NAME", keys_of("RawApprover")),
        ("usage", keys_of("RawUsage")),
        ("notifications", keys_of("RawNotifications")),
        ("notifications.notifiers.NAME", keys_of("RawNotifier")),
        ("host.agents.NAME", keys_of("RawAgent")),
        ("host.providers.NAME", keys_of("RawProvider")),
        ("host.accounts.NAME", keys_of("RawAccount")),
        ("host.pools.NAME", keys_of("RawPool")),
        ("host.roles.NAME", keys_of("RawRole")),
        ("host.extensions.NAME", keys_of("RawExtension")),
        ("host.environments.NAME", keys_of("RawEnvironment")),
        ("host.openers", keys_of("RawOpener")),
        ("host.web", keys_of("RawWeb")),
        ("host.control", keys_of("RawControl")),
        ("host.native", keys_of("RawNative")),
        ("host.store", keys_of("RawStore")),
        ("host.content", keys_of("RawContent")),
        ("host.delegate", keys_of("RawDelegate")),
        ("host.kb", keys_of("RawKb")),
        ("host.kb.embedder", keys_of("RawKbEmbedder")),
        ("host.kb.ranker", keys_of("RawKbRanker")),
        ("host.kb.search", keys_of("RawKbSearch")),
        ("host.mcp_list", keys_of("RawMcpList")),
        ("host.usage", keys_of("RawHostUsage")),
        ("host.usage.limits.NAME", keys_of("RawLimit")),
        ("host.worktrees", keys_of("RawWorktrees")),
        ("host.history", keys_of("RawHistory")),
        ("network", keys_of("RawNetwork")),
        ("network.routes.NAME", keys_of("RawRoute")),
        ("host.telemetry", keys_of("RawTelemetry")),
        (
            "host.telemetry.exporters.NAME",
            keys_of("RawTelemetryExporter"),
        ),
    ]
}

/// The name an editor table has in [`tables`]: its path, and `.NAME` for a table of entries.
fn named(table: &crate::control::advanced::Table) -> String {
    let path = table.path.join(".");
    if matches!(table.shape, Shape::Entries | Shape::Pairs) && table.path.len() > 1 {
        format!("{path}.NAME")
    } else {
        path
    }
}

/// The keys of `table` that have no page.
fn without_page(table: &str, keys: &[String]) -> Vec<String> {
    keys.iter()
        .filter(|key| {
            let in_editor = TABLES.iter().any(|each| {
                named(each) == table && each.fields.iter().any(|(name, _)| name == &key.as_str())
            });
            let is_editor_table = TABLES.iter().any(|each| {
                let own = named(each);
                own == format!("{table}.{key}") || own == format!("{table}.{key}.NAME")
            });
            let dedicated = DEDICATED
                .iter()
                .any(|(name, listed)| *name == table && listed.contains(&key.as_str()));
            let owned = OWNED
                .iter()
                .any(|(name, owned, _)| *name == table && owned == &key.as_str());
            !(in_editor || is_editor_table || dedicated || owned)
        })
        .cloned()
        .collect()
}

#[test]
fn every_key_the_host_reads_has_a_page_or_is_owned_by_the_host() {
    let mut missing = Vec::new();
    for (table, keys) in tables() {
        assert!(!keys.is_empty(), "{table} lists no keys");
        for key in without_page(table, &keys) {
            missing.push(format!("{table}.{key}"));
        }
    }
    assert!(
        missing.is_empty(),
        "these keys have no page: add each to control::advanced::TABLES or DEDICATED, or to OWNED with the reason: {missing:?}"
    );
}

/// The control: a key the host started reading and nobody wrote a page for is found.
#[test]
fn a_key_with_no_page_is_found() {
    let mut keys = keys_of("RawWeb");
    keys.push("a_new_setting".to_owned());
    assert_eq!(without_page("host.web", &keys), ["a_new_setting"]);
    let mut keys = keys_of("RawFile");
    keys.push("a_new_table".to_owned());
    assert_eq!(without_page("(top level)", &keys), ["a_new_table"]);
}

/// Every key an editor table lists is one the host reads, so the editor never offers a key the host would refuse.
#[test]
fn the_editor_offers_only_keys_the_host_reads() {
    let known = tables();
    for table in TABLES {
        let name = named(table);
        let Some((_, keys)) = known.iter().find(|(each, _)| *each == name) else {
            // Free-form entries ([host.tools.NAME]) are read by hand, `[harnesses]` is the application's, and `[portal]` is read by the
            // portal's own settings (`portal_page_lists_every_key_the_portal_reads`), not a struct of this reader.
            assert!(
                matches!(name.as_str(), "host.tools.NAME" | "harnesses" | "portal"),
                "{name} is not a table the host's reader lists"
            );
            continue;
        };
        for (field, _) in table.fields {
            assert!(
                keys.iter().any(|key| key == field),
                "{name} offers {field}, which the host does not read"
            );
        }
    }
}

/// `[portal]` is read by the portal's own reader, so the page that shows it lists the keys that reader names.
#[test]
fn portal_page_lists_every_key_the_portal_reads() {
    let source = include_str!("../portal/settings.rs");
    let start = source
        .find("struct Raw {")
        .expect("the portal's raw settings");
    let body = &source[start..];
    let body = &body[..body.find("\n}").expect("the struct ends")];
    let read: Vec<&str> = body
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().split_once(':').map(|(key, _)| key.trim()))
        .collect();
    let page = TABLES
        .iter()
        .find(|table| table.path == ["portal"])
        .expect("the editor lists [portal]");
    let offered: Vec<&str> = page.fields.iter().map(|(name, _)| *name).collect();
    for key in &read {
        assert!(
            offered.contains(key),
            "the portal reads {key}, which its page does not list"
        );
    }
    for key in &offered {
        assert!(
            read.contains(key),
            "the page lists {key}, which the portal does not read"
        );
    }
}
