//! `[notifications]`: for each event the names of the notifiers it goes to, and the notifier entries the names refer to.
//!
//! The keys other than `notifiers` are event ids, taken from the host's catalog (`crate::notify::events`); a list replaces the
//! default of its event and `[]` turns the event off. A value that is a bool is the older form of the table and is refused with the key
//! named, as are an unknown event and a name that is no notifier. Errors name a key, never a value: a setting may be a credential.

use std::path::Path;

use indexmap::IndexMap;

use super::entries_parse::{OriginKeys, origin_of};
use super::raw::{RawNotifications, RawNotifier};
use super::sections::directory_of;
use super::types::{NotificationsConfig, NotifierConfig};
use super::{ConfigError, profiles::settings_of};
use crate::notify::events;

fn refused(what: String) -> ConfigError {
    ConfigError::Invalid {
        entry: None,
        what: format!("notifications: {what}"),
    }
}

/// `[notifications]` as the host reads it, with relative `library` and `source` paths taken from `base`.
pub(super) fn notifications_of(
    raw: Option<RawNotifications>,
    base: &Path,
) -> Result<NotificationsConfig, ConfigError> {
    let Some(raw) = raw else {
        return Ok(NotificationsConfig::default());
    };
    let directory = directory_of(base);
    let mut notifiers = IndexMap::new();
    for (name, entry) in raw.notifiers {
        if !events::is_notifier_name(&name) {
            return Err(refused(
                "a notifier name is 1 to 32 lower-case letters, digits, - and _".to_owned(),
            ));
        }
        if name == events::DESKTOP {
            return Err(refused(format!(
                "{name} is the notifier every host has and cannot be defined"
            )));
        }
        let notifier = notifier_of(&name, entry, base, &directory)?;
        notifiers.insert(name, notifier);
    }
    let mut routes = IndexMap::new();
    for (event, value) in raw.rest {
        if events::find(&event).is_none() {
            return Err(refused(format!("{event} is not an event of this host")));
        }
        let names = match value {
            toml::Value::Array(items) => items
                .into_iter()
                .map(|item| match item {
                    toml::Value::String(name) => Ok(name),
                    _ => Err(refused(format!("{event} lists notifier names, as text"))),
                })
                .collect::<Result<Vec<String>, _>>()?,
            toml::Value::Boolean(_) => {
                return Err(refused(format!(
                    "{event} is a list of notifier names, not true or false"
                )));
            }
            _ => return Err(refused(format!("{event} is a list of notifier names"))),
        };
        for (index, name) in names.iter().enumerate() {
            if name != events::DESKTOP && !notifiers.contains_key(name) {
                return Err(refused(format!(
                    "{event} names {name}, which is no notifier: define it in [notifications.notifiers.{name}]"
                )));
            }
            if names[..index].contains(name) {
                return Err(refused(format!("{event} names {name} twice")));
            }
        }
        routes.insert(event, names);
    }
    Ok(NotificationsConfig { routes, notifiers })
}

fn notifier_of(
    name: &str,
    raw: RawNotifier,
    base: &Path,
    directory: &Path,
) -> Result<NotifierConfig, ConfigError> {
    let Some(provider) = raw.provider.filter(|provider| !provider.is_empty()) else {
        return Err(refused(format!("{name}: provider is missing or empty")));
    };
    if !canoa::notifier::is_word(&provider) {
        return Err(refused(format!(
            "{name}: provider is 1 to 32 lower-case letters, digits and _"
        )));
    }
    let origin = origin_of(
        name,
        base,
        directory,
        OriginKeys {
            library: raw.library.as_deref(),
            source: raw.source.as_deref(),
            source_settings: raw.source_settings,
            import_search: raw.import_search.as_deref(),
        },
    )?;
    let settings = settings_of(name, raw.settings.unwrap_or_default())?;
    Ok(NotifierConfig {
        provider,
        origin,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::super::Config;

    fn parse(text: &str) -> Result<Config, String> {
        Config::from_str_in(text, std::path::Path::new(".")).map_err(|error| error.to_string())
    }

    #[test]
    fn a_table_that_names_nothing_leaves_every_event_to_the_catalog() {
        let config = parse("").expect("parses");
        assert!(
            config.notifications.routes.is_empty() && config.notifications.notifiers.is_empty()
        );
        let config = parse("[notifications]\n").expect("parses");
        assert!(config.notifications.routes.is_empty());
    }

    #[test]
    fn an_event_takes_a_list_of_notifiers_and_an_empty_list_turns_it_off() {
        let config = parse(
            r#"
[notifications]
permission = ["desktop", "phone"]
finished = []

[notifications.notifiers.phone]
provider = "ntfy"
settings = { server = "https://ntfy.example", topic = "t", timeout_seconds = 5 }
"#,
        )
        .expect("parses");
        assert_eq!(
            config.notifications.routes["permission"],
            ["desktop", "phone"]
        );
        assert!(config.notifications.routes["finished"].is_empty());
        let phone = &config.notifications.notifiers["phone"];
        assert_eq!(phone.provider, "ntfy");
        assert_eq!(phone.settings.as_map()["topic"], "t");
        assert!(phone.origin.is_none());
    }

    #[test]
    fn the_old_bool_form_is_refused_naming_the_key() {
        for text in [
            "[notifications]\npermission = true\n",
            "[notifications]\nquestion = false\n",
            "[notifications]\nfinished = true\n",
        ] {
            let why = parse(text).expect_err("a bool is no list");
            assert!(why.contains("not true or false"), "{why}");
            let key = text.lines().nth(1).and_then(|line| line.split(' ').next());
            assert!(why.contains(key.expect("a key")), "{why}");
        }
    }

    #[test]
    fn an_unknown_event_a_name_that_is_no_notifier_and_a_bad_entry_are_refused() {
        let why = parse("[notifications]\nbogus = []\n").expect_err("unknown event");
        assert!(why.contains("bogus"), "{why}");
        let why = parse("[notifications]\nfinished = [\"phone\"]\n").expect_err("no such notifier");
        assert!(
            why.contains("phone") && why.contains("no notifier"),
            "{why}"
        );
        let why =
            parse("[notifications]\nfinished = [\"desktop\", \"desktop\"]\n").expect_err("twice");
        assert!(why.contains("twice"), "{why}");
        let why = parse("[notifications.notifiers.desktop]\nprovider = \"ntfy\"\n")
            .expect_err("cannot be redefined");
        assert!(why.contains("desktop"), "{why}");
        let why = parse("[notifications.notifiers.Phone]\nprovider = \"ntfy\"\n")
            .expect_err("a bad name");
        assert!(why.contains("lower-case"), "{why}");
        let why =
            parse("[notifications.notifiers.phone]\nsettings = {}\n").expect_err("no provider");
        assert!(why.contains("provider"), "{why}");
        let why = parse("[notifications.notifiers.phone]\nprovider = \"ntfy\"\nbogus = 1\n")
            .expect_err("a key it does not know");
        assert!(
            why.contains("phone") && why.contains("does not know"),
            "{why}"
        );
    }

    #[test]
    fn a_notifier_entry_names_where_its_code_is_as_an_environment_does() {
        let config = parse(
            "[notifications.notifiers.hook]\nprovider = \"webhook\"\nlibrary = \"hooks.dll\"\n",
        )
        .expect("parses");
        assert!(config.notifications.notifiers["hook"].origin.is_some());
        let why = parse(
            "[notifications.notifiers.hook]\nprovider = \"webhook\"\nlibrary = \"a\"\nsource = \"b\"\n",
        )
        .expect_err("both");
        assert!(why.contains("exclude"), "{why}");
    }
}
