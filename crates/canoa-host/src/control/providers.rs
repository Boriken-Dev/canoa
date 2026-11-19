//! `providers/list`: the agents this host has loaded, as what each says of itself. Nothing here names an agent: a row is made of the
//! `acp_harness`, `account_provider` and `agent_vendor` providers the bundled libraries and the libraries and sources of
//! canoa.cfg's entries and extensions offer, so an agent that is not loaded is not listed, and one an outside library adds is
//! listed the same way as one Canoa carries.

use std::collections::BTreeMap;

use canoa::usage_reader::UsageReader;
use canoa::vendor::{AgentVendor, VendorInfo};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountProvider, AcpHarness};
use serde_json::{Value, json};

use crate::bundled::{self, Bundle};
use crate::config::{Config, EntryKind, Origin, Settings};
use crate::kind::{account_provider, instance};
use crate::makers::ACCOUNTS_SUFFIX;

/// What the loaded providers say of one agent, from every registry that offers a part of it.
#[derive(Default)]
struct Row {
    /// Where its providers come from: `bundled`, or the name of the entry or extension that loads them.
    origin: String,
    vendor: Option<VendorInfo>,
    harness: Option<String>,
    accounts: Option<Value>,
    /// The title the harness or account provider gives, when no vendor description does.
    title: Option<String>,
}

/// `{"providers": [...]}`: one row per agent, in the order the bundled agents are offered, then the loaded ones' as they were found.
pub(crate) fn list(config: &Config) -> Value {
    let registries = loaded(config);
    let loaded: Vec<(&str, &Registry)> = registries
        .iter()
        .map(|(origin, registry)| (origin.as_str(), registry))
        .collect();
    json!({
        "providers": rows_of(&loaded, bundled::ALL),
        "usage_readers": usage_readers_of(&loaded),
        "gateways": gateways_of(),
    })
}

/// The gateways the bundled libraries offer for the sessions of accounts that run a model on this computer: each `{kind, vendor,
/// provider, title}`, the account kind it serves, the vendor of what runs the model, the account provider that makes the kind and
/// the kind's name.
pub(crate) fn gateways_of() -> Vec<Value> {
    crate::gateways::offered()
        .iter()
        .map(|offer| {
            json!({
                "kind": offer.info.kind,
                "vendor": offer.info.vendor,
                "provider": offer.provider,
                "title": offer.kind().map(|kind| kind.title.clone()),
            })
        })
        .collect()
}

/// The `usage_reader` providers `registries` offer, each `{id, origin}` (`origin` as a row's): the ids an agent entry's `usage` may
/// name, bundled first. An id is listed once, as the first registry that offers it, which is the one the host reads through.
pub(crate) fn usage_readers_of(registries: &[(&str, &Registry)]) -> Vec<Value> {
    let mut readers: Vec<Value> = Vec::new();
    for (origin, registry) in registries {
        for offer in registry.offers::<dyn UsageReader>() {
            let Some(info) = offer
                .default_instance()
                .and_then(|reader| reader.describe().ok())
                .filter(|info| info.problem().is_none())
            else {
                continue;
            };
            if readers.iter().all(|each| each["id"] != info.id.as_str()) {
                readers.push(json!({ "id": info.id, "origin": origin }));
            }
        }
    }
    readers
}

/// The loaded registries of `config`: the bundled agents' and each entry's and extension's own.
pub(crate) fn loaded(config: &Config) -> Vec<(String, Registry)> {
    let mut registries: Vec<(String, Registry)> = Vec::new();
    let mut seen: Vec<Origin> = Vec::new();
    match bundled_registry(bundled::ALL) {
        Ok(registry) => registries.push(("bundled".to_owned(), registry)),
        Err(why) => tracing::warn!("the bundled agents are not listed: {why}"),
    }
    let origins = config
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Harness {
                origin: Some(origin),
                ..
            } => Some((entry.name.as_str(), origin, true)),
            _ => None,
        })
        .chain(
            config
                .extensions
                .iter()
                .map(|extension| (extension.name.as_str(), &extension.origin, false)),
        );
    for (name, origin, entry) in origins {
        if seen.contains(origin) {
            continue;
        }
        if !entry
            && let Some(extension) = config.extensions.iter().find(|each| each.name == name)
            && let Err(why) = super::stores::verify_extension(extension)
        {
            tracing::warn!(extension = name, "not listed: {why}");
            continue;
        }
        seen.push(origin.clone());
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        match crate::provider::open(&mut registry, origin) {
            Ok(()) => registries.push((name.to_owned(), registry)),
            Err(why) => tracing::warn!(entry = name, "its providers are not listed: {why}"),
        }
    }
    registries
}

/// The look documents the loaded agents carry, each a JSON object with an `id`, in the order the agents were found.
pub(crate) fn looks_of(registries: &[(&str, &Registry)]) -> Vec<Value> {
    let mut looks: Vec<Value> = Vec::new();
    for (_, registry) in registries {
        for offer in registry.offers::<dyn AgentVendor>() {
            let Some(info) = offer
                .default_instance()
                .and_then(|vendor| vendor.describe().ok())
                .filter(|info| info.problem().is_none())
            else {
                continue;
            };
            let Some(document) = info
                .look
                .as_deref()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
            else {
                continue;
            };
            let id = document
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if id.is_some()
                && !looks
                    .iter()
                    .any(|each| each.get("id").and_then(Value::as_str) == id.as_deref())
            {
                looks.push(document);
            }
        }
    }
    looks
}

/// A registry holding the libraries of `bundles`, and the ones this build carries that are no agent.
pub(crate) fn bundled_registry(bundles: &[Bundle]) -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    bundled::register(&mut registry, bundles)?;
    bundled::register_included(&mut registry)?;
    Ok(registry)
}

/// The row of agent `id`, made when no registry has said of it yet.
fn touch(
    rows: &mut BTreeMap<String, Row>,
    order: &mut Vec<String>,
    id: &str,
    origin: &str,
) -> String {
    if !rows.contains_key(id) {
        order.push(id.to_owned());
        rows.insert(
            id.to_owned(),
            Row {
                origin: origin.to_owned(),
                ..Row::default()
            },
        );
    }
    id.to_owned()
}

/// The rows of `registries` (each with the origin it is named by), the sources of `bundles` joined to the agents they start.
pub(crate) fn rows_of(registries: &[(&str, &Registry)], bundles: &[Bundle]) -> Vec<Value> {
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for (origin, registry) in registries {
        for offer in registry.offers::<dyn AgentVendor>() {
            let Some(info) = offer
                .default_instance()
                .and_then(|vendor| vendor.describe().ok())
            else {
                continue;
            };
            if let Some(problem) = info.problem() {
                tracing::warn!(
                    provider = offer.id(),
                    "its description is not used: {problem}"
                );
                continue;
            }
            let id = touch(&mut rows, &mut order, &info.agent, origin);
            if let Some(slot) = rows.get_mut(&id) {
                slot.vendor.get_or_insert(info);
            }
        }
        for offer in registry.offers::<dyn AcpHarness>() {
            let id = touch(&mut rows, &mut order, offer.id(), origin);
            let Some(slot) = rows.get_mut(&id) else {
                continue;
            };
            slot.harness = Some(offer.id().to_owned());
            if let Ok((harness, _)) = instance(registry, offer.id(), &Settings::default())
                && let Ok(info) = harness.describe()
            {
                slot.title.get_or_insert(info.title);
            }
        }
        for offer in registry.offers::<dyn AccountProvider>() {
            let provider_id = offer.id().to_owned();
            // The kinds of a maker's accounts join the listings of the harnesses that can run them: no agent is theirs.
            if crate::makers::ids().contains(&provider_id) {
                continue;
            }
            let harness = provider_id
                .strip_suffix(ACCOUNTS_SUFFIX)
                .unwrap_or(&provider_id)
                .to_owned();
            let provider = if provider_id.ends_with(ACCOUNTS_SUFFIX) {
                account_provider(
                    registry,
                    &harness,
                    &Settings::default(),
                    &Settings::default(),
                )
                .ok()
                .flatten()
            } else {
                offer.shared()
            };
            let id = touch(&mut rows, &mut order, &harness, origin);
            let Some(slot) = rows.get_mut(&id) else {
                continue;
            };
            if let Some(provider) = provider
                && let Ok(about) = provider.describe()
                && let Ok(offered) = provider.offers()
            {
                slot.title.get_or_insert_with(|| about.title.clone());
                slot.accounts = Some(json!({
                    "provider": provider_id,
                    "title": about.title,
                    "description": about.description,
                    "kinds": offered.kinds.iter().map(|kind| json!({"id": kind.id, "title": kind.title})).collect::<Vec<_>>(),
                    "logins": offered.logins.iter().map(|login| json!({"id": login.id, "title": login.title, "produces": login.produces})).collect::<Vec<_>>(),
                    "usage": offered.usage,
                    "local": offered.local,
                }));
            }
        }
    }
    // The bundled agents come in the order their crates are listed, and the others after them as they were found: a registry
    // answers by id, which is not an order anyone chose.
    let place = |id: &String| {
        bundles
            .iter()
            .position(|bundle| bundle.preset.agent_id() == *id)
            .unwrap_or(usize::MAX)
    };
    order.sort_by_key(place);
    order
        .iter()
        .filter_map(|id| rows.get(id).map(|row| (id, row)))
        .map(|(id, row)| {
            let preset = bundles
                .iter()
                .map(|bundle| &bundle.preset)
                .find(|preset| preset.agent_id() == *id);
            let source = preset.map(|preset| preset.name);
            // How the host starts a program of the agent that reports its quota itself, so a command a person writes that is the
            // same program is read the same way.
            let launch = preset
                .filter(|preset| preset.usage.is_some())
                .map(|preset| json!({"program": preset.program, "args": preset.args, "usage": preset.usage}));
            let vendor = row.vendor.as_ref();
            let title = vendor
                .map(|vendor| vendor.title.clone())
                .or_else(|| row.title.clone())
                .unwrap_or_else(|| id.clone());
            json!({
                "id": id,
                "title": title,
                "program": vendor.and_then(|vendor| vendor.program.clone()).unwrap_or_else(|| title.clone()),
                "vendor": vendor.and_then(|vendor| vendor.vendor.clone()),
                "aos": vendor.and_then(|vendor| vendor.aos.clone()),
                "model_variable": vendor.and_then(|vendor| vendor.model_variable.clone()),
                "standards": vendor.map(|vendor| vendor.standards.iter().map(|standard| json!({"id": standard.id, "title": standard.title})).collect::<Vec<_>>()).unwrap_or_default(),
                "names": vendor.map(|vendor| vendor.names.clone()).unwrap_or_default(),
                "modes": vendor.map(|vendor| vendor.modes.iter().map(|mode| json!({"id": mode.id, "title": mode.title})).collect::<Vec<_>>()).unwrap_or_default(),
                "mark": vendor.and_then(|vendor| vendor.mark.clone()),
                "look": vendor.and_then(|vendor| vendor.look.as_deref()).and_then(|text| serde_json::from_str::<Value>(text).ok()).and_then(|look| look.get("id").cloned()),
                "harness": row.harness.is_some(),
                "imports": super::import::imports(id),
                "markers": vendor.map(|vendor| vendor.markers.clone()).unwrap_or_default(),
                "accounts": row.accounts,
                "source": source,
                "launch": launch,
                "origin": row.origin,
            })
        })
        .collect()
}
