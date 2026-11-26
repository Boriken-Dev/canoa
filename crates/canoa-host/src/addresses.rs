//! What the person is told of the addresses canoa.cfg names: each one that is plain http beyond this computer. Nothing here refuses
//! an address; it is the person's network, and the cost is said where they look (`canoa check`, `canoa accounts list`, the log
//! when the host starts, and the application's account rows).

use crate::accounts::{self, matching::Catalogs};
use crate::config::Config;

/// The notes for every address `config` names, each in words that start with what names it (`account work: base_url`).
pub(crate) fn warnings(config: &Config) -> Vec<String> {
    let catalogs = Catalogs::new(config);
    let mut notes = Vec::new();
    for (name, account) in &config.accounts {
        if let Some(info) = catalogs.kind(account) {
            notes.extend(accounts::address_notes(
                name,
                &info,
                account.settings.as_ref(),
            ));
        }
    }
    notes.extend(service_and_sdk_warnings(config));
    notes
}

/// The notes for the addresses of the session services' tables and the `[sdk]` providers, which need no account catalog.
pub(crate) fn service_and_sdk_warnings(config: &Config) -> Vec<String> {
    let mut notes = Vec::new();
    for (service, text) in &config.services {
        if let Ok(table) = serde_json::from_str::<serde_json::Value>(text) {
            plain_http_in(&format!("host.services.{service}"), &table, &mut notes);
        }
    }
    for (name, table) in &config.assist.sdk {
        if let Some(address) = &table.address {
            notes.extend(canoa::address::plain_http_note(
                &format!("[sdk.{name}] address"),
                address,
            ));
        }
    }
    notes
}

/// Notes each text value of `value`, at any depth, that is a plain http address, under the name of the table that holds it.
fn plain_http_in(path: &str, value: &serde_json::Value, notes: &mut Vec<String>) {
    let Some(table) = value.as_object() else {
        return;
    };
    for (key, value) in table {
        match value {
            serde_json::Value::String(address)
                if address
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("http://") =>
            {
                notes.extend(canoa::address::plain_http_note(
                    &format!("[{path}] {key}"),
                    address,
                ));
            }
            serde_json::Value::Object(_) => plain_http_in(&format!("{path}.{key}"), value, notes),
            _ => {}
        }
    }
}

/// Logs each note at `info` as a non-default setting.
pub(crate) fn log(config: &Config) {
    for note in warnings(config) {
        tracing::info!("non-default: {note}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(text: &str) -> Config {
        Config::from_str_in(text, &std::env::temp_dir()).expect("canoa.cfg parses")
    }

    #[test]
    fn plain_http_beyond_this_computer_is_noted_in_a_services_table_and_an_sdk_address_and_nowhere_else()
     {
        let quiet = config_of(
            "[host.services.mail.accounts.a]\nbackend = \"ews\"\nserver = \"https://mail.example/EWS/Exchange.asmx\"\nuser = \"u\"\npassword_env = \"P\"\n\
             [host.services.mail.accounts.b]\nbackend = \"graph\"\nclient_id = \"c\"\nserver = \"http://127.0.0.1:9/v1.0\"\n\
             [sdk.local]\nprovider = \"server\"\naddress = \"http://localhost:60570\"\n",
        );
        assert!(warnings(&quiet).is_empty(), "{:?}", warnings(&quiet));
        let loud = config_of(
            "[host.services.mail.accounts.a]\nbackend = \"ews\"\nserver = \"http://192.0.2.9/EWS/Exchange.asmx\"\nuser = \"u\"\npassword_env = \"P\"\n\
             [sdk.remote]\nprovider = \"server\"\naddress = \"http://192.0.2.9:60570\"\n",
        );
        let notes = warnings(&loud);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0]
                .starts_with("[host.services.mail.accounts.a] server is plain http to 192.0.2.9"),
            "{notes:?}"
        );
        assert!(
            notes[1].starts_with("[sdk.remote] address is plain http to 192.0.2.9"),
            "{notes:?}"
        );
    }
}
