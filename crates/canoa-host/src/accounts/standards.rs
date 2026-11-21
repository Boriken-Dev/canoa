//! An account kind whose accounts each say which standards they provide.
//!
//! A kind's `provides` is the same for every account of it. A kind that stands for an address that speaks whichever standards the
//! person says (a proxy, a gateway) marks one property of its settings schema `"x-provides": true`: that property, an array of
//! standard ids, is the standards the account provides, in place of the kind's. The host reads it here and nowhere else, so
//! everything that asks what an account provides (the harnesses it runs on, its pools, the grant a harness is given) agrees, and the
//! harness is given the settings without it, since a harness takes only the address of an account of a standard.
//!
//! A kind may also say that an account names, for each standard it provides, the address a program that applies the account as that
//! standard is given and the form its key takes there: the schema marks a property `"x-by-standard": true`, an object whose keys are
//! standard ids and whose values hold `base_url` and `credential`. A gateway that serves its standards at different addresses, or
//! wants a bearer token for one and a key for another, is one account.
//!
//! A kind may instead say that the value of one of its settings picks the standards: the property carries
//! `coqui_acp_api::PROVIDES_BY_VALUE`, an object from each value to its standards (a provider key whose variable names the API it is
//! a key of). The kind's own `provides` is what an account holding any other value provides, and a harness that is not the kind's own
//! is not given that setting, since it takes only the address of an account of a standard.
//!
//! The host names no kind and no standard.

use std::collections::BTreeSet;

use coqui_acp_api::AccountKindInfo;
use coqui_acp_api::PROVIDES_BY_VALUE;
use coqui_acp_api::guatiao::Value;
use coqui_acp_api::standard::is_standard;

use crate::config::Settings;
use crate::provider::map_of;

/// The annotation of a property of a kind's settings schema that makes the property the account's standards.
pub(crate) const PROVIDES: &str = "x-provides";

/// The annotation of a property that holds, for each standard the account provides, the address and the credential key a program
/// that applies the account as that standard is given.
pub(crate) const BY_STANDARD: &str = "x-by-standard";

/// The name of the property of `info`'s settings that carries the annotation `note`.
fn annotated(info: &AccountKindInfo, note: &str) -> Option<String> {
    let schema = guatiao_serde::adapt::from_value::<serde_json::Value>(&info.settings).ok()?;
    schema
        .get("properties")?
        .as_object()?
        .iter()
        .find(|(_, field)| field.get(note).and_then(serde_json::Value::as_bool) == Some(true))
        .map(|(name, _)| name.clone())
}

/// The property of `info`'s settings whose value picks the standards, and what each value picks.
fn by_value(
    info: &AccountKindInfo,
) -> Option<(String, serde_json::Map<String, serde_json::Value>)> {
    let schema = guatiao_serde::adapt::from_value::<serde_json::Value>(&info.settings).ok()?;
    schema
        .get("properties")?
        .as_object()?
        .iter()
        .find_map(|(name, field)| {
            let picked = field.get(PROVIDES_BY_VALUE)?.as_object()?;
            Some((name.clone(), picked.clone()))
        })
}

/// Whether the standards an account provides are picked by the value of one of its settings.
pub(crate) fn picks_by_value(info: &AccountKindInfo) -> bool {
    by_value(info).is_some()
}

/// The standards the account with `settings` provides, for a kind whose settings carry [`PROVIDES_BY_VALUE`]: those its value
/// lists, else the kind's own. A listed name that is not a standard id is left out.
fn provided_by_value(info: &AccountKindInfo, settings: Option<&Settings>) -> Vec<String> {
    let listed = by_value(info).and_then(|(key, picked)| {
        let held = settings?.as_map().get(&key)?.as_str()?.to_owned();
        picked.get(&held)?.as_array().cloned()
    });
    let Some(listed) = listed else {
        return info.provides.clone();
    };
    listed
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|id| is_standard(id))
        .fold(Vec::new(), |mut found, id| {
            if !found.iter().any(|each| each == id) {
                found.push(id.to_owned());
            }
            found
        })
}

/// The name of the property of `info`'s settings that holds the standards an account provides, when the kind says one does.
pub(crate) fn property(info: &AccountKindInfo) -> Option<String> {
    annotated(info, PROVIDES)
}

/// Whether accounts of the kind `info` say how they are applied for each standard, so that a harness is given settings of its own.
pub(crate) fn names_per_standard(info: &AccountKindInfo) -> bool {
    annotated(info, BY_STANDARD).is_some()
}

/// What an account says of how it is applied as one standard: the address a program is given in place of the account's own, and the
/// key the credential is given under.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    /// The address for the model API, when the account names one for the standard.
    pub(crate) address: Option<String>,
    /// The credential key the harness is given the account's one credential under, when the account names one for the standard.
    pub(crate) credential: Option<String>,
}

/// What the account with `settings` says of how it is applied as `standard`; nothing when its kind has no such property or the
/// account names nothing for the standard.
pub(crate) fn applied_as(
    info: &AccountKindInfo,
    settings: Option<&Settings>,
    standard: &str,
) -> Applied {
    let named = |field: &str| {
        let key = annotated(info, BY_STANDARD)?;
        settings?
            .as_map()
            .get(&key)?
            .get(standard)?
            .get(field)?
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Applied {
        address: named(crate::accounts::matching::BASE_URL),
        credential: named("credential"),
    }
}

/// The key of the account's settings that holds its address for the model API: where `applied`'s address goes.
fn address_key(info: &AccountKindInfo) -> String {
    info.endpoints
        .iter()
        .find(|slot| slot.purpose == coqui_acp_api::MODEL_API)
        .and_then(|slot| slot.key.clone())
        .unwrap_or_else(|| crate::accounts::matching::BASE_URL.to_owned())
}

/// The standards the account with `settings` provides: those its property names when the kind has one and the account sets it, else
/// the kind's own. A value that is not a list of standard ids is left out, so an account whose settings are wrong provides
/// nothing; `check_settings` is what tells the person.
pub(crate) fn provided(info: &AccountKindInfo, settings: Option<&Settings>) -> Vec<String> {
    let Some(key) = property(info) else {
        return provided_by_value(info, settings);
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

/// The settings the account's maker is given: the account's own, without the property that holds the standards (the grant carries
/// them as what it provides).
pub(crate) fn for_maker(info: &AccountKindInfo, settings: Option<&Settings>) -> Value {
    let Some(settings) = settings else {
        return Value::from(coqui_acp_api::guatiao::Map::new());
    };
    let mut map = settings.as_map().clone();
    if let Some(key) = property(info) {
        map.remove(&key);
    }
    Value::from(map_of(&Settings::new(map)).unwrap_or_default())
}

/// The settings a harness is given for the account: without the properties that hold the standards and what each says, and with the
/// address `applied` names, in place of the account's own, when it names one. A harness that applies the account as one of a
/// standard (`borrowed`) is not given the setting whose value picked the standards either: it is the kind's own.
pub(crate) fn for_harness(
    info: &AccountKindInfo,
    settings: Option<&Settings>,
    applied: &Applied,
    borrowed: bool,
) -> Value {
    let Some(settings) = settings else {
        return Value::from(coqui_acp_api::guatiao::Map::new());
    };
    let mut map = settings.as_map().clone();
    for key in [property(info), annotated(info, BY_STANDARD)]
        .into_iter()
        .flatten()
    {
        map.remove(&key);
    }
    if borrowed && let Some((key, _)) = by_value(info) {
        map.remove(&key);
    }
    if let Some(address) = &applied.address {
        map.insert(
            address_key(info),
            serde_json::Value::String(address.clone()),
        );
    }
    Value::from(map_of(&Settings::new(map)).unwrap_or_default())
}

/// The keys of `credentials` a harness is given for an account applied as one of `provides`, when `borrowed` and the account holds
/// keys no standard it provides names: those the well-known standards name. `None` when the harness is given them all: the account
/// is one of the harness's own kinds, a standard it provides is not a well-known one (its keys are unknown), or it holds nothing
/// more than the standards name.
pub(crate) fn keys_for(
    provides: &[String],
    borrowed: bool,
    credentials: &coqui_acp_api::Credentials,
) -> Option<BTreeSet<String>> {
    if !borrowed {
        return None;
    }
    let mut named = BTreeSet::new();
    for standard in provides {
        let known = coqui_acp_api::standard::well_known(standard)?;
        named.extend(known.credentials.iter().map(|key| (*key).to_owned()));
    }
    let held: BTreeSet<String> = credentials.keys().cloned().collect();
    let kept: BTreeSet<String> = held.intersection(&named).cloned().collect();
    (!kept.is_empty() && kept != held).then_some(kept)
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
        let given = for_harness(&info, Some(&one), &Applied::default(), false);
        let given =
            guatiao_serde::adapt::from_value::<serde_json::Map<String, serde_json::Value>>(&given)
                .expect("a table");
        assert_eq!(given.keys().collect::<Vec<_>>(), ["base_url"]);
    }

    #[test]
    fn a_harness_is_given_the_keys_its_standard_names_and_the_maker_the_rest() {
        let held = coqui_acp_api::Credentials::from_iter([
            ("api_key", "k"),
            ("refresh_token", "r"),
            ("client_id", "c"),
        ]);
        let both = ["openai-api".to_owned(), "anthropic-api".to_owned()];
        let kept = keys_for(&both, true, &held).expect("a refresh token is not the harness's");
        assert_eq!(kept.into_iter().collect::<Vec<_>>(), ["api_key"]);
        assert!(
            keys_for(&both, false, &held).is_none(),
            "a kind of its own harness is given all"
        );
        let only = coqui_acp_api::Credentials::from_iter([("api_key", "k")]);
        assert!(
            keys_for(&both, true, &only).is_none(),
            "nothing to leave out"
        );
        let own = ["a-standard-nobody-named".to_owned()];
        assert!(
            keys_for(&own, true, &held).is_none(),
            "its keys are not known"
        );
        let none = coqui_acp_api::Credentials::from_iter([("refresh_token", "r")]);
        assert!(
            keys_for(&both, true, &none).is_none(),
            "the harness says why it cannot"
        );
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

    fn by_value_kind() -> AccountKindInfo {
        let mut kind = AccountKindInfo::new("key", "Key");
        kind.settings = guatiao_serde::from_serde(json!({
            "type": "object",
            "properties": {
                "variable": {
                    "type": "string",
                    "x-provides-by-value": {"A_KEY": ["a-api"], "AB_KEY": ["a-api", "b-api", "a-api", "Not An Id"]},
                },
            },
        }))
        .expect("a schema");
        kind
    }

    #[test]
    fn the_value_of_a_setting_picks_the_standards_an_account_provides_and_another_value_picks_the_kinds_own()
     {
        let info = by_value_kind();
        assert!(picks_by_value(&info));
        assert!(!picks_by_value(&kind()));
        let held = |variable: &str| settings(json!({ "variable": variable }));
        assert_eq!(provided(&info, Some(&held("A_KEY"))), ["a-api"]);
        assert_eq!(
            provided(&info, Some(&held("AB_KEY"))),
            ["a-api", "b-api"],
            "once each, and only ids"
        );
        assert!(provided(&info, Some(&held("OTHER_KEY"))).is_empty());
        assert!(provided(&info, None).is_empty());
        assert_eq!(
            of_account(info.clone(), Some(&held("A_KEY"))).provides,
            ["a-api"]
        );
        let own = by_value_kind().provides(["own-api"]);
        assert_eq!(provided(&own, Some(&held("OTHER_KEY"))), ["own-api"]);
        assert_eq!(provided(&own, Some(&held("A_KEY"))), ["a-api"]);
    }

    #[test]
    fn a_harness_that_applies_the_account_as_a_standard_is_not_given_the_setting_that_picked_it() {
        let info = by_value_kind();
        let account = settings(json!({"variable": "A_KEY"}));
        let given = |borrowed: bool| {
            guatiao_serde::adapt::from_value::<serde_json::Value>(&for_harness(
                &info,
                Some(&account),
                &Applied::default(),
                borrowed,
            ))
            .expect("JSON")
        };
        assert_eq!(
            given(false),
            json!({"variable": "A_KEY"}),
            "the kind's own harness"
        );
        assert_eq!(given(true), json!({}));
    }

    fn by_standard_kind() -> AccountKindInfo {
        let mut kind = kind();
        kind.settings = guatiao_serde::from_serde(json!({
            "type": "object",
            "properties": {
                "base_url": {"type": "string"},
                "standards": {"type": "array", "items": {"type": "string"}, "x-provides": true},
                "by_standard": {"type": "object", "x-by-standard": true},
            },
        }))
        .expect("a schema");
        kind.endpoints =
            vec![coqui_acp_api::EndpointSlot::new(coqui_acp_api::MODEL_API).key("base_url")];
        kind
    }

    #[test]
    fn an_account_names_the_address_and_the_key_form_for_each_standard_and_a_harness_is_given_the_one_it_applies()
     {
        let info = by_standard_kind();
        let account = settings(json!({
            "base_url": "https://g/default",
            "standards": ["a-api", "b-api", "c-api"],
            "by_standard": {
                "a-api": {"base_url": "https://g/v1"},
                "b-api": {"credential": "auth_token", "base_url": ""},
            },
        }));
        assert_eq!(
            applied_as(&info, Some(&account), "a-api"),
            Applied {
                address: Some("https://g/v1".to_owned()),
                credential: None
            }
        );
        assert_eq!(
            applied_as(&info, Some(&account), "b-api"),
            Applied {
                address: None,
                credential: Some("auth_token".to_owned())
            },
            "an empty address names none"
        );
        assert_eq!(
            applied_as(&info, Some(&account), "c-api"),
            Applied::default()
        );
        // A kind with no such property says nothing, whatever the account holds.
        assert_eq!(
            applied_as(&kind(), Some(&account), "a-api"),
            Applied::default()
        );
        let given = |applied: &Applied| {
            guatiao_serde::adapt::from_value::<serde_json::Value>(&for_harness(
                &info,
                Some(&account),
                applied,
                false,
            ))
            .expect("JSON")
        };
        assert_eq!(
            given(&applied_as(&info, Some(&account), "a-api")),
            json!({"base_url": "https://g/v1"})
        );
        assert_eq!(
            given(&applied_as(&info, Some(&account), "c-api")),
            json!({"base_url": "https://g/default"}),
            "its own address where it names none"
        );
    }
}
