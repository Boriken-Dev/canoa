//! Antigravity's and Grok's accounts over the control channel, as the desktop app uses them: the account providers and the sources
//! they make listed. The program is a stand-in; every folder an agent keeps its login in is this test's own, so no test reads the
//! person's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::sync::Once;

use canoa_host::Link;
use linked::Hosted;
use serde_json::{Value, json};
use support::{example, quoted};

/// Points every variable an agent's login folder is found by at a folder of this test's, before any host starts.
fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let folder = std::env::temp_dir().join(format!("canoa-agy-logins-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        for name in [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "PI_CODING_AGENT_DIR",
            "GROK_HOME",
        ] {
            // SAFETY: set once, before this test's hosts start, and every test of this file calls `isolate` first.
            unsafe { std::env::set_var(name, folder.join(name)) };
        }
    });
}

fn plain_call(link: &Link, id: u64, method: &str, params: Value) -> Result<Value, String> {
    linked::ask(link, id, method, params).map(|(result, _)| result)
}

#[test]
fn grok_and_antigravity_account_providers_and_sources_are_listed() {
    isolate();
    let agent = example("stdio_agent", false);
    let host = Hosted::start_with_setup(
        |dir| {
            let grok = dir.join(format!("grok{}", std::env::consts::EXE_SUFFIX));
            let agy = dir.join(format!("agy{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&agent, &grok).expect("grok stand-in copied");
            std::fs::copy(&agent, &agy).expect("agy stand-in copied");
            format!(
                "[secrets]\naccounts = \"file\"\n\n\
                 [host.agents.Grok]\nharness = \"grok\"\nexecutable = {}\n\n\
                 [host.agents.Antigravity]\nharness = \"antigravity\"\nexecutable = {}\n",
                quoted(&grok.to_string_lossy()),
                quoted(&agy.to_string_lossy()),
            )
        },
        &[],
    );
    let link = host.local.open("@control").expect("a link");
    let listed = plain_call(&link, 1, "providers/list", json!({})).expect("providers");
    let agents = listed["providers"].as_array().expect("providers array");
    for (id, accounts_id) in [
        ("grok", "grok_accounts"),
        ("antigravity", "antigravity_accounts"),
    ] {
        let entry = agents
            .iter()
            .find(|agent| agent["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing from {listed}"));
        assert_eq!(entry["harness"], true, "{entry}");
        assert_eq!(entry["accounts"]["provider"], accounts_id, "{entry}");
        let catalog =
            plain_call(&link, 2, "accounts/catalog", json!({"harness": id})).expect("catalog");
        assert!(catalog["kinds"].is_array(), "{id}: {catalog}");
        assert!(catalog["logins"].is_array(), "{id}: {catalog}");
    }
    let accounts = plain_call(&link, 3, "accounts/list", json!({})).expect("accounts");
    let rows = accounts["accounts"].as_array().expect("accounts array");
    for harness in ["grok", "antigravity"] {
        assert!(
            rows.iter()
                .any(|row| row["harness"] == harness && row["name"] == "local"),
            "missing local account for {harness} in {accounts}"
        );
    }
    let entries = plain_call(&link, 4, "accounts/entries", json!({})).expect("entries");
    let entry_rows = entries["entries"].as_array().expect("entries array");
    for name in ["Grok", "Antigravity"] {
        assert!(
            entry_rows.iter().any(|entry| entry["name"] == name),
            "{entries}"
        );
    }
    host.stop();
}
