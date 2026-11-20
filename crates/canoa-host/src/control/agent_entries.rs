//! What an agent entry asks for, described the way every form is: a guatiao schema of the keys the entry takes and the guatiao-intake
//! form that says how to show them. `providers/list` answers it, so the application draws the form that adds an entry from what this
//! host reads, and a key the host learns shows up there without a change to the application.
#![allow(clippy::std_instead_of_alloc, reason = "the host is std's")]

use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::{Value, json};

use super::speech::{form_of, schema_of};

/// An entry of `[host.agents]`: an agent this host serves to the clients that are paired to it. It names a program to run or a
/// harness, not both; a program takes `args` and `usage`, and a harness takes the rest.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(
    section(id = "run", label = "What it runs"),
    section(id = "where", label = "Where it works"),
    section(id = "accounts", label = "Accounts and profiles"),
    section(id = "library", label = "Library")
)]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct AgentEntry {
    /// What the entry is called, which a client names it by.
    #[schema(section = "run", title = "Name")]
    pub name: String,
    /// A program that speaks ACP on its standard streams, a name found on the search path or a path. Name this or a harness.
    #[schema(section = "run", title = "Program")]
    #[form(placeholder = "my-agent")]
    pub program: Option<String>,
    /// The program's arguments, one per line.
    #[map(default)]
    #[schema(section = "run", title = "Arguments")]
    pub args: Vec<String>,
    /// The harness a library provides, which the host runs in place of a program. Name this or a program.
    #[schema(section = "run", title = "Harness")]
    #[form(widget = "harness")]
    pub harness: Option<String>,
    /// What a client is shown for the entry in place of its name.
    #[schema(section = "run", title = "Title")]
    pub title: Option<String>,
    /// How the quota of a program is read; the program's own command decides when none is chosen.
    #[schema(section = "run", title = "Read its quota with")]
    #[form(widget = "usage_reader")]
    pub usage: Option<String>,
    /// The folders a session of this entry may be given, each a whole path. Without one it works wherever the client asks.
    #[map(default)]
    #[schema(section = "where", title = "Only in these workspaces")]
    pub working_roots: Vec<String>,
    /// Whether a session works in a worktree of its folder.
    #[schema(section = "where", title = "Works in a worktree")]
    pub worktrees: Option<bool>,
    /// The accounts, by name, that a client may start a session on.
    #[map(default)]
    #[schema(section = "accounts", title = "Accounts")]
    pub accounts: Vec<String>,
    /// The profiles, by name, that a client may start a session with.
    #[map(default)]
    #[schema(section = "accounts", title = "Profiles")]
    pub profiles: Vec<String>,
    /// Whether the entry has a profile of its own beside the ones it offers.
    #[schema(advanced, section = "accounts", title = "Has its own profile")]
    pub own_profile: Option<bool>,
    /// A harness's library file, which provides it.
    #[schema(advanced, section = "library", title = "Library")]
    pub library: Option<String>,
    /// A harness's source file, a script that provides it.
    #[schema(advanced, section = "library", title = "Source")]
    pub source: Option<String>,
    /// The harness's own executable, when it is not the one it finds.
    #[schema(advanced, section = "library", title = "Executable")]
    pub executable: Option<String>,
    /// Whether one session serves every client of the entry, which can then watch and answer in it together.
    #[schema(advanced, section = "where", title = "One session for every client")]
    pub shared: Option<bool>,
}

/// An entry of `[agents]`: an agent command of the person's own, which Canoa runs on this computer for itself and no client reaches.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct LocalAgent {
    /// What the agent is called in the list of sources.
    #[schema(title = "Name")]
    pub name: String,
    /// A program that speaks ACP on its standard streams, a name found on the search path or a path.
    #[schema(title = "Program")]
    #[form(placeholder = "my-agent")]
    pub program: String,
    /// The program's arguments, one per line.
    #[map(default)]
    #[schema(title = "Arguments")]
    pub args: Vec<String>,
}

/// `{schema, form}` of `S`, or null when it cannot be described.
fn described<S: Schema + guatiao_intake::Screen>() -> Value {
    match (schema_of::<S>(), form_of::<S>()) {
        (Ok(schema), Ok(form)) => json!({ "schema": schema, "form": form }),
        (Err(why), _) | (_, Err(why)) => {
            tracing::warn!("an agent entry's form is not described: {why}");
            Value::Null
        }
    }
}

/// What an `[host.agents]` entry takes, as `providers/list` answers it.
pub(crate) fn agent_entry() -> Value {
    described::<AgentEntry>()
}

/// What an `[agents]` entry takes, as `providers/list` answers it.
pub(crate) fn local_agent() -> Value {
    described::<LocalAgent>()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::config::Config;

    /// A value that has the shape the schema member `field` says, for a probe of whether the host reads the key.
    fn probe(field: &Value) -> &'static str {
        let kind = field["type"].as_array().map_or_else(
            || field["type"].as_str().unwrap_or("string"),
            |kinds| {
                kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|kind| *kind != "null")
                    .unwrap_or("string")
            },
        );
        match kind {
            "array" => "[\"x\"]",
            "boolean" => "true",
            _ => "\"x\"",
        }
    }

    fn members(entry: &Value) -> Vec<(String, &'static str)> {
        entry["schema"]["properties"]
            .as_object()
            .expect("a schema of an object")
            .iter()
            .filter(|(name, _)| *name != "name")
            .map(|(name, field)| (name.clone(), probe(field)))
            .collect()
    }

    /// Whether the host refuses `key` as no key of an entry, in the words it refuses one with.
    fn unknown(key: &str, value: &str) -> bool {
        let text = format!(
            "[host.agents.probe]\nprogram = \"p\"\nlisten = \"127.0.0.1:0\"\n{key} = {value}\n"
        );
        Config::from_str_in(&text, std::path::Path::new(".")).is_err_and(|why| {
            why.to_string()
                .contains("a setting this host does not know")
        })
    }

    #[test]
    fn every_key_of_a_described_host_entry_is_one_canoa_cfg_reads() {
        assert!(
            unknown("no_such_key", "\"x\""),
            "the probe recognises a refusal"
        );
        let entry = super::agent_entry();
        let members = members(&entry);
        assert!(
            members.len() > 5,
            "the entry describes its keys: {members:?}"
        );
        for (key, value) in members {
            assert!(!unknown(&key, value), "{key} is not a key of an entry");
        }
    }

    #[test]
    fn every_key_of_a_described_local_agent_is_one_canoa_cfg_reads() {
        let entry = super::local_agent();
        for (key, value) in members(&entry)
            .into_iter()
            .filter(|(key, _)| key != "program")
        {
            let text = format!("[agents.probe]\nprogram = \"p\"\n{key} = {value}\n");
            Config::from_str_in(&text, std::path::Path::new("."))
                .unwrap_or_else(|why| panic!("{key} is not a key of [agents.NAME]: {why}"));
        }
    }
}
