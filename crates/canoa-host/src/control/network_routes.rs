//! Network routes on the control channel: `routes/list`, `routes/offers`, `routes/set`, `routes/remove`, `routes/default`,
//! `routes/test` and `accounts/set_route`. A route is a provider of the `network_route` kind and its settings, written as
//! `[network.routes.NAME]`; the host's own is `[network] route`, an account's is `[host.accounts.NAME] route`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use std::time::Instant;

use canoa::config_store::{Changes, ConfigStore, Op, value_of};
use serde_json::{Map, Value, json};

use super::params::{Refusal, text};
use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::config::{Config, DIRECT_ROUTE, SYSTEM_ROUTE};
use crate::network::routes::{self, RouteCatalog};

/// Whether `name` is a route of `config`'s host.
fn has(config: &Config, name: &str) -> bool {
    name == SYSTEM_ROUTE || name == DIRECT_ROUTE || config.network.routes.contains_key(name)
}

fn write(store: &dyn ConfigStore, ops: Vec<Op>) -> Result<(), Refusal> {
    store
        .change(Changes::of(ops))
        .map(|_| ())
        .map_err(|error| (REFUSED, error.to_string()))
}

/// The profiles whose `network` item names the route `name`.
fn profiles_using(config: &Config, name: &str) -> Vec<String> {
    config
        .local_profiles
        .presets
        .iter()
        .filter(|(_, preset)| {
            preset
                .items
                .get("network")
                .and_then(|item| item.get("routes"))
                .and_then(Value::as_array)
                .is_some_and(|routes| {
                    routes
                        .iter()
                        .any(|each| each.get("route").and_then(Value::as_str) == Some(name))
                })
        })
        .map(|(profile, _)| profile.clone())
        .collect()
}

/// Where each route is used: `{host, accounts[], profiles[]}`.
fn uses_of(config: &Config, name: &str) -> Value {
    let accounts: Vec<&String> = config
        .accounts
        .iter()
        .filter(|(_, account)| account.route.as_deref() == Some(name))
        .map(|(account, _)| account)
        .collect();
    json!({
        "host": config.network.default_route() == name,
        "accounts": accounts,
        "profiles": profiles_using(config, name),
    })
}

/// `routes/list` `{}`: the routes of this host, `system` and `direct` first, each with its provider, whether it is the host's, where
/// it is used, how many leases are open and what is wrong with it.
pub(super) fn list(config: &Config) -> Value {
    let catalog = RouteCatalog::of(config);
    let providers = catalog.list();
    let rows: Vec<Value> = routes::names(config)
        .into_iter()
        .map(|name| {
            let provider = config
                .network
                .routes
                .get(&name)
                .map_or(name.as_str(), |route| route.provider.as_str());
            let listed = providers.iter().find(|each| each.id == provider);
            let standing = routes::standing(&catalog, config, &name);
            json!({
                "name": name,
                "provider": provider,
                "title": listed.map(|each| each.info.title.clone()),
                "default": config.network.default_route() == name,
                "builtin": name == SYSTEM_ROUTE || name == DIRECT_ROUTE,
                "sharing": listed.map(|each| format!("{:?}", each.info.sharing)),
                "settings": config.network.routes.get(&name).map(|route| route.settings.clone()),
                "uses": uses_of(config, &name),
                "leases": standing.leases,
                "state": standing.status,
                "problems": standing.problems,
            })
        })
        .collect();
    json!({"routes": rows, "default": config.network.default_route()})
}

/// `routes/offers` `{}`: the providers of the kind, each with its schema and form, from which a route is made.
pub(super) fn offers(config: &Config) -> Value {
    let rows: Vec<Value> = RouteCatalog::of(config)
        .list()
        .into_iter()
        .map(|each| {
            json!({
                "provider": each.id,
                "origin": each.origin,
                "title": each.info.title,
                "description": each.info.description,
                "sharing": format!("{:?}", each.info.sharing),
                "secrets": each.info.secrets,
                "schema": each.schema,
                "form": each.form,
            })
        })
        .collect();
    json!({"offers": rows})
}

/// `routes/set` `{"name", "provider", "settings"?}`: makes the route `name`, or changes it. The provider judges the settings first, and
/// a route it finds wrong is refused with its words. The host reads its routes when it starts.
pub(super) fn set(
    store: &dyn ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    let provider = text(params, "provider")?;
    if let Some(why) = canoa::config_store::id_problem(name) {
        return Err((BAD_PARAMS, why));
    }
    if name == SYSTEM_ROUTE || name == DIRECT_ROUTE {
        return Err((
            REFUSED,
            format!(
                "{name} is a route every host has: it needs no table, and its name is not given to another"
            ),
        ));
    }
    let settings = match params.get("settings") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(settings @ Value::Object(_)) => settings.clone(),
        Some(_) => return Err((BAD_PARAMS, "settings is an object".to_owned())),
    };
    let catalog = RouteCatalog::of(config);
    let problems = routes::problems_of(&catalog, config, provider, &settings);
    if !problems.is_empty() {
        return Err((REFUSED, problems.join("; ")));
    }
    let body = json!({"provider": provider, "settings": settings});
    write(
        store,
        vec![Op::set(["network", "routes", name], value_of(&body))],
    )?;
    Ok(json!({"route": {"name": name, "provider": provider}, "restart": true}))
}

/// `routes/remove` `{"name"}`: refused while it is the host's route or an account or profile names it, saying where.
pub(super) fn remove(
    store: &dyn ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    if name == SYSTEM_ROUTE || name == DIRECT_ROUTE {
        return Err((REFUSED, format!("{name} is a route every host has")));
    }
    if !config.network.routes.contains_key(name) {
        return Err((NOT_FOUND, format!("no route called {name}")));
    }
    let uses = uses_of(config, name);
    if uses["host"] == true {
        return Err((
            REFUSED,
            format!("{name} is the host's route: make another the default first"),
        ));
    }
    let accounts = uses["accounts"].as_array().cloned().unwrap_or_default();
    if let Some(account) = accounts.first().and_then(Value::as_str) {
        return Err((REFUSED, format!("the account {account} uses {name}")));
    }
    let profiles = uses["profiles"].as_array().cloned().unwrap_or_default();
    if let Some(profile) = profiles.first().and_then(Value::as_str) {
        return Err((REFUSED, format!("the profile {profile} uses {name}")));
    }
    write(store, vec![Op::remove(["network", "routes", name])])?;
    Ok(json!({"removed": true, "restart": true}))
}

/// `routes/default` `{"name"}`: the route the host's own traffic leaves by.
pub(super) fn make_default(
    store: &dyn ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    if !has(config, name) {
        return Err((NOT_FOUND, format!("no route called {name}")));
    }
    write(
        store,
        vec![Op::set(["network", "route"], value_of(&json!(name)))],
    )?;
    Ok(json!({"default": name, "restart": true}))
}

/// `accounts/set_route` `{"account", "route"?}`: the route the account's own requests leave by; absent or null takes it back to the
/// host's.
pub(super) fn set_account_route(
    store: &dyn ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let account = text(params, "account")?;
    if !config.accounts.contains_key(account) {
        return Err((NOT_FOUND, format!("no account called {account}")));
    }
    let key = ["host", "accounts", account, "route"];
    match params.get("route") {
        None | Some(Value::Null) => write(store, vec![Op::remove(key)])?,
        Some(Value::String(route)) => {
            if !has(config, route) {
                return Err((NOT_FOUND, format!("no route called {route}")));
            }
            write(store, vec![Op::set(key, value_of(&json!(route)))])?;
        }
        Some(_) => return Err((BAD_PARAMS, "route is a name".to_owned())),
    }
    Ok(
        json!({"account": account, "route": params.get("route").cloned().unwrap_or(Value::Null), "restart": true}),
    )
}

/// `routes/test` `{"name", "url"?}`: opens the route and says how it would reach `url` (the first of `[network] endpoints` when
/// absent): the proxy chosen, the time, and the failure in words.
pub(super) fn test(config: &Config, params: &Value) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    if !has(config, name) {
        return Err((NOT_FOUND, format!("no route called {name}")));
    }
    let url = match params.get("url").and_then(Value::as_str) {
        Some(url) if !url.is_empty() => url.to_owned(),
        _ => config.network.endpoints.first().cloned().ok_or((
            BAD_PARAMS,
            "url names an address to try: the host has no `endpoints` to default to".to_owned(),
        ))?,
    };
    let started = Instant::now();
    let network = match routes::network(Some(name)) {
        Ok(network) => network,
        Err(failure) => {
            return Ok(
                json!({"ok": false, "why": failure.to_string(), "kind": format!("{:?}", failure.kind)}),
            );
        }
    };
    let chosen = network.resolver().lookup(&url);
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(match chosen {
        Ok(found) => json!({
            "ok": true,
            "url": url,
            "via": found
                .iter()
                .map(|route| match route {
                    canoa::network::ProxyRoute::Proxy(proxy) => proxy.to_string(),
                    _ => "direct".to_owned(),
                })
                .collect::<Vec<_>>(),
            "direct": network.goes_direct(&url),
            "millis": millis,
        }),
        Err(error) => json!({"ok": false, "url": url, "why": error.to_string(), "millis": millis}),
    })
}
