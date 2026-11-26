//! `providers/list`: the agents this host has loaded, as what each says of itself. Nothing here names an agent: a row is made of the
//! `acp_harness`, `account_provider` and `agent_vendor` providers the bundled libraries and the libraries and sources of
//! canoa.cfg's entries and extensions offer, so an agent that is not loaded is not listed, and one an outside library adds is
//! listed the same way as one Canoa carries.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use canoa::usage_reader::UsageReader;
use canoa::vendor::{AgentVendor, VendorInfo};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountProvider, AccountsLink, AcpHarness};
use serde_json::{Value, json};

use crate::bundled::{self, Bundle};
use crate::config::{Config, EntryKind, Origin, Settings};
use crate::kind::{account_instance, account_provider, instance};
use crate::makers::{ACCOUNTS_SUFFIX, Makers, makers_in};

pub(crate) type LoadedRegistries = Arc<[(String, Arc<Registry>)]>;

/// Cached opened provider registries per [`Config`]; cloned configs start empty so modified entries/extensions reload cleanly.
#[derive(Default)]
pub(crate) struct Cell {
    libraries: OnceLock<LoadedRegistries>,
    loaded: OnceLock<LoadedRegistries>,
}

impl Clone for Cell {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl core::fmt::Debug for Cell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Cell").finish_non_exhaustive()
    }
}

impl PartialEq for Cell {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Cell {}

/// What the loaded providers say of one agent, from every registry that offers a part of it.
#[derive(Default)]
struct Row {
    /// Where its providers come from: `bundled`, or the name of the entry or extension that loads them.
    origin: String,
    vendor: Option<VendorInfo>,
    harness: Option<String>,
    accounts: Option<Value>,
    /// The id of the account provider the agent has that `[host.disabled]` leaves off: no account of it is offered, and the
    /// accounts canoa.cfg holds for it are shown as off.
    accounts_off: Option<String>,
    /// The title the harness or account provider gives, when no vendor description does.
    title: Option<String>,
}

/// `{"providers": [...]}`: one row per agent, in the order the bundled agents are offered, then the loaded ones' as they were found,
/// with the readers of an agent's quota, the gateways and the forms of an agent entry (`agent_entry`, `local_agent`).
pub(crate) fn list(config: &Config) -> Value {
    let registries = loaded(config);
    let loaded: Vec<(&str, &Registry)> = registries
        .iter()
        .map(|(origin, registry)| (origin.as_str(), registry.as_ref()))
        .collect();
    json!({
        "providers": rows_of(&loaded, bundled::ALL, &Makers::of(config)),
        "usage_readers": usage_readers_of(&loaded),
        "gateways": gateways_of(&loaded),
        "agent_entry": super::agent_entries::agent_entry(),
        "local_agent": super::agent_entries::local_agent(),
    })
}

/// The agents `providers/list` lists, as `(id, title)`: the one list every page that offers an agent starts from.
pub(crate) fn agents(config: &Config) -> Vec<(String, String)> {
    list(config)["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            Some((
                id.to_owned(),
                row["title"].as_str().unwrap_or(id).to_owned(),
            ))
        })
        .collect()
}

/// Every provider the loaded libraries register, as `(kind, id)`, a provider that serves several kinds once for each.
pub(crate) fn pairs(config: &Config) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for (_, registry) in loaded(config).iter() {
        for provider in registry.all() {
            for kind in provider.kinds() {
                let pair = (kind.clone(), provider.id().to_owned());
                if !found.contains(&pair) {
                    found.push(pair);
                }
            }
        }
    }
    found
}

/// `providers/registered`: every provider the loaded libraries register, by kind, with whether `[host.disabled]` leaves it off, and
/// what `[host.disabled]` names that no library offers. `{"providers": [{"kind", "id", "title", "library", "origin", "off",
/// "kinds"?}], "unknown": [text], "disabled": {kind: [id]}}`; `kinds`, for an account provider, are the account kinds it makes.
pub(crate) fn registered(config: &Config) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    for (origin, registry) in loaded(config).iter() {
        for provider in registry.all() {
            for kind in provider.kinds() {
                let pair = (kind.clone(), provider.id().to_owned());
                if seen.contains(&pair) {
                    continue;
                }
                seen.push(pair);
                let mut row = json!({
                    "kind": kind,
                    "id": provider.id(),
                    "title": provider.display_name(),
                    "library": provider.library(),
                    "origin": origin,
                    "off": config.disabled.is_off(kind, provider.id()),
                });
                if kind
                    == <dyn AccountProvider as coqui_acp_api::guatiao::library::kind::Kind>::NAME
                    && let Ok(Some(made)) = crate::kind::account_instance_unchecked(
                        registry,
                        provider.id(),
                        &[],
                        &Settings::default(),
                        &Settings::default(),
                    )
                    && let Ok(offered) = made.offers()
                {
                    row["kinds"] = json!(
                        offered
                            .kinds
                            .iter()
                            .map(|kind| kind.id.clone())
                            .collect::<Vec<_>>()
                    );
                }
                rows.push(row);
            }
        }
    }
    rows.sort_by(|a, b| {
        (a["kind"].as_str(), a["id"].as_str()).cmp(&(b["kind"].as_str(), b["id"].as_str()))
    });
    json!({
        "providers": rows,
        "unknown": config.disabled.unknown(&seen),
        "disabled": config.disabled.to_value(),
    })
}

/// The gateways `registries` offer for the sessions of accounts that run a model on this computer: each `{kind, vendor,
/// provider, title}`, the account kind it serves, the vendor of what runs the model, the account provider that makes the kind and
/// the kind's name.
pub(crate) fn gateways_of(registries: &[(&str, &Registry)]) -> Vec<Value> {
    crate::gateways::offered_in(registries)
        .into_iter()
        .map(|offer| {
            let title = offer.kind().map(|kind| kind.title.clone());
            json!({
                "kind": offer.info.kind,
                "vendor": offer.info.vendor,
                "provider": offer.provider,
                "title": title,
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
            if crate::disabled::off::<dyn UsageReader>(offer.id()) {
                continue;
            }
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

/// The loaded registries of `config`: the bundled agents' and each entry's and extension's own, cached per config.
pub(crate) fn loaded(config: &Config) -> LoadedRegistries {
    Arc::clone(config.providers.loaded.get_or_init(|| {
        let mut registries: Vec<(String, Arc<Registry>)> = Vec::new();
        if let Some(registry) = crate::vendors::bundled_providers_arc() {
            registries.push(("bundled".to_owned(), registry));
        }
        registries.extend(libraries(config).iter().cloned());
        registries.into()
    }))
}

/// The registries available for the session services' backends and the speech providers: `loaded(config)` when `Some`, else the carried libraries alone.
pub(crate) fn registries_of(config: Option<&Config>) -> Vec<Arc<Registry>> {
    if let Some(config) = config {
        return loaded(config)
            .iter()
            .map(|(_, registry)| Arc::clone(registry))
            .collect();
    }
    let mut carried = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
    if let Err(why) = crate::bundled::register_included(&mut carried) {
        tracing::warn!("the carried libraries could not be loaded: {why}");
    }
    vec![Arc::new(carried)]
}

/// The registries of the libraries and sources `config` names, each with the name of the entry or extension that loads it: the
/// agent entries' and the extensions' (an extension verified as the Store verifies it), each origin once, cached per config.
pub(crate) fn libraries(config: &Config) -> LoadedRegistries {
    Arc::clone(config.providers.libraries.get_or_init(|| {
        let mut registries: Vec<(String, Arc<Registry>)> = Vec::new();
        let mut seen: Vec<Origin> = Vec::new();
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
            )
            .chain(
                config
                    .notifications
                    .notifiers
                    .iter()
                    .filter_map(|(name, notifier)| {
                        notifier
                            .origin
                            .as_ref()
                            .map(|origin| (name.as_str(), origin, true))
                    }),
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
            let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
            match crate::provider::open(&mut registry, origin) {
                Ok(()) => {
                    crate::vendors::note(&registry);
                    registries.push((name.to_owned(), Arc::new(registry)));
                }
                Err(why) => tracing::warn!(entry = name, "its providers are not listed: {why}"),
            }
        }
        registries.into()
    }))
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
    let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
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
pub(crate) fn rows_of(
    registries: &[(&str, &Registry)],
    bundles: &[Bundle],
    makers: &Makers,
) -> Vec<Value> {
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for (origin, registry) in registries {
        // The account providers the harnesses of this registry declare, with the harness each belongs to.
        let mut declared: BTreeMap<String, (String, AccountsLink)> = BTreeMap::new();
        for offer in registry.offers::<dyn AgentVendor>() {
            if crate::disabled::off::<dyn AgentVendor>(offer.id()) {
                continue;
            }
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
            if crate::disabled::off::<dyn AcpHarness>(offer.id()) {
                continue;
            }
            let id = touch(&mut rows, &mut order, offer.id(), origin);
            let Some(slot) = rows.get_mut(&id) else {
                continue;
            };
            slot.harness = Some(offer.id().to_owned());
            if let Ok((harness, _)) = instance(registry, offer.id(), &Settings::default())
                && let Ok(info) = harness.describe()
            {
                slot.title.get_or_insert(info.title);
                if let Some(link) = info.accounts {
                    declared.insert(link.provider.clone(), (offer.id().to_owned(), link));
                }
            }
        }
        let made: Vec<String> = makers_in(registry)
            .into_iter()
            .map(|maker| maker.provider)
            .collect();
        for offer in registry.offers::<dyn AccountProvider>() {
            let provider_id = offer.id().to_owned();
            // The kinds of a maker's accounts join the listings of the harnesses that can run them: no agent is theirs.
            if made.contains(&provider_id) || makers.ids().contains(&provider_id) {
                continue;
            }
            // A harness names its account provider; one no harness names belongs to the agent its id says (`<agent>_accounts`).
            let (harness, provider) = match declared.get(&provider_id) {
                Some((harness, link)) => (
                    harness.clone(),
                    account_provider(registry, link, &Settings::default(), &Settings::default())
                        .ok()
                        .flatten(),
                ),
                None => {
                    let agent = provider_id
                        .strip_suffix(ACCOUNTS_SUFFIX)
                        .unwrap_or(&provider_id)
                        .to_owned();
                    let provider = if provider_id.ends_with(ACCOUNTS_SUFFIX) {
                        account_instance(
                            registry,
                            &provider_id,
                            &[],
                            &Settings::default(),
                            &Settings::default(),
                        )
                        .ok()
                        .flatten()
                    } else {
                        offer.shared()
                    };
                    (agent, provider)
                }
            };
            let id = touch(&mut rows, &mut order, &harness, origin);
            let Some(slot) = rows.get_mut(&id) else {
                continue;
            };
            if crate::disabled::off::<dyn AccountProvider>(&provider_id) {
                slot.accounts_off = Some(provider_id);
                continue;
            }
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
                "accounts_off": row.accounts_off,
                "source": source,
                "launch": launch,
                "origin": row.origin,
            })
        })
        .collect()
}
