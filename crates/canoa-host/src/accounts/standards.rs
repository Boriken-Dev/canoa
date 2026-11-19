//! An account kind whose accounts each say which standards they provide.
//!
//! A kind's `provides` is the same for every account of it. A kind that stands for an address that speaks whichever standards the
//! person says (a proxy, a gateway) marks one property of its settings schema `"x-provides": true`: that property, an array of
//! standard ids, is the standards the account provides, in place of the kind's. The host reads it here and nowhere else, so
//! everything that asks what an account provides (the harnesses it runs on, its pools, the grant a harness is given) agrees, and the
//! harness is given the settings without it, since a harness takes only the address of an account of a standard.
//!
//! The host names no kind and no standard.

use coqui_acp_api::AccountKindInfo;
use coqui_acp_api::guatiao::Value;
use coqui_acp_api::standard::is_standard;

use crate::config::Settings;
use crate::provider::map_of;

/// The annotation of a property of a kind's settings schema that makes the property the account's standards.
pub(crate) const PROVIDES: &str = "x-provides";

/// The name of the property of `info`'s settings that holds the standards an account provides, when the kind says one does.
pub(crate) fn property(info: &AccountKindInfo) -> Option<String> {
    let schema = guatiao_serde::adapt::from_value::<serde_json::Value>(&info.settings).ok()?;
    schema
        .get("properties")?
        .as_object()?
        .iter()
        .find(|(_, field)| field.get(PROVIDES).and_then(serde_json::Value::as_bool) == Some(true))
        .map(|(name, _)| name.clone())
}

/// The standards the account with `settings` provides: those its property names when the kind has one and the account sets it, else
/// the kind's own. A value that is not a list of standard ids is left out, so an account whose settings are wrong provides
/// nothing; `check_settings` is what tells the person.
pub(crate) fn provided(info: &AccountKindInfo, settings: Option<&Settings>) -> Vec<String> {
    let Some(key) = property(info) else {
        return info.provides.clone();
    };
    match settings.and_then(|settings| settings.as_map().get(&key)) {
        Some(serde_json::Value::Array(listed)) => listed
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|id| is_standard(id))
            .fold(Vec::new(), |mut found, id| {
                if !found.iter().any(|each| each == id) {
                    found.push(id.to_owned());
                }
                found
            }),
        Some(_) => Vec::new(),
        None => info.provides.clone(),
    }
}

/// `info` as it is for the account with `settings`: the same kind, providing what the account provides.
pub(crate) fn of_account(info: AccountKindInfo, settings: Option<&Settings>) -> AccountKindInfo {
    let provides = provided(&info, settings);
    info.provides(provides)
}

/// The settings a harness is given for the account: without the property that holds the standards.
pub(crate) fn for_harness(info: &AccountKindInfo, settings: Option<&Settings>) -> Value {
    let Some(settings) = settings else {
        return Value::from(coqui_acp_api::guatiao::Map::new());
    };
    let mut map = settings.as_map().clone();
    if let Some(key) = property(info) {
        map.remove(&key);
    }
    Value::from(map_of(&Settings::new(map)).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn kind() -> AccountKindInfo {
        let mut kind = AccountKindInfo::new("gateway", "Gateway").provides(["openai-api"]);
        kind.settings = guatiao_serde::from_serde(json!({
            "type": "object",
            "properties": {
                "base_url": {"type": "string"},
                "standards": {"type": "array", "items": {"type": "string"}, "x-provides": true},
            },
        }))
        .expect("a schema");
        kind
    }

    fn settings(value: serde_json::Value) -> Settings {
        Settings::new(value.as_object().cloned().expect("a table"))
    }

    #[test]
    fn the_standards_an_account_names_are_what_it_provides_and_the_harness_is_not_told_them() {
        let info = kind();
        assert_eq!(property(&info).as_deref(), Some("standards"));
        let one =
            settings(json!({"base_url": "https://g", "standards": ["a-api", "b-api", "a-api"]}));
        assert_eq!(provided(&info, Some(&one)), ["a-api", "b-api"]);
        assert_eq!(
            of_account(info.clone(), Some(&one)).provides,
            ["a-api", "b-api"]
        );
        let given = for_harness(&info, Some(&one));
        let given =
            guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(&given)
                .expect("a table");
        assert_eq!(given.keys().collect::<Vec<_>>(), ["base_url"]);
    }

    #[test]
    fn a_kind_without_the_property_provides_what_it_says_and_an_account_without_it_does_too() {
        let plain = AccountKindInfo::new("k", "K").provides(["x-api"]);
        let any = settings(json!({"standards": ["y-api"]}));
        assert_eq!(provided(&plain, Some(&any)), ["x-api"]);
        let info = kind();
        assert_eq!(provided(&info, None), ["openai-api"]);
        // A value that is not a list of ids provides nothing, never something else.
        let wrong = settings(json!({"standards": "openai-api"}));
        assert!(provided(&info, Some(&wrong)).is_empty());
        let bad = settings(json!({"standards": ["Not An Id", "ok-api"]}));
        assert_eq!(provided(&info, Some(&bad)), ["ok-api"]);
    }
}
