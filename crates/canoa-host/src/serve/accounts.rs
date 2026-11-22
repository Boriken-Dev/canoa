//! Harness provider settings and account selection helpers for serving entries.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use std::ffi::OsString;

use crate::accounts::matching::{Catalogs, fits};
use crate::config::{Config, Entry, HarnessOptions, Settings};
use crate::kind;
use crate::sources;

/// Requests a harness provider's connection works on at once, unless the entry's `settings` say otherwise.
pub(super) const HARNESS_MAX_REQUESTS: u64 = 64;

/// Shared built-in provider settings: explicit or Store-installed `executable`, `allow_bypass`, and `modes`, each added where
/// the provider's schema declares it and not already set in `own`.
pub(crate) fn builtin_provider_settings(
    registry: &coqui_acp_api::guatiao::library::Registry,
    harness: &str,
    executable: Option<&OsString>,
    own: Option<&Settings>,
) -> Settings {
    let mut map = own.map(|own| own.as_map().clone()).unwrap_or_default();
    if let Some(executable) = executable {
        map.insert(
            "executable".to_owned(),
            serde_json::Value::String(executable.to_string_lossy().into_owned()),
        );
    } else if crate::bundled::runs_managed_by_own_name(harness)
        && !map.contains_key("executable")
        && let Some((path, _)) = sources::managed_program(harness)
    {
        // Nothing names the program: the one the Store installed runs, found as it is now.
        map.insert(
            "executable".to_owned(),
            serde_json::Value::String(path.to_string_lossy().into_owned()),
        );
    }
    let takes = |key: &str| crate::kind::declares_setting(registry, harness, key);
    // Every mode the agent has is offered, Bypass included: the person's own account runs it, and a paired key is theirs.
    if takes("allow_bypass") {
        map.entry("allow_bypass".to_owned())
            .or_insert(serde_json::Value::Bool(true));
    }
    if takes("modes") {
        map.entry("modes".to_owned()).or_insert_with(|| {
            serde_json::Value::from(
                coqui_acp_harness::Mode::ALL
                    .iter()
                    .map(|mode| mode.id())
                    .collect::<Vec<_>>(),
            )
        });
    }
    Settings::new(map)
}

/// The settings a harness entry's provider is configured with: the entry's `settings`, then the entry's own keys of the
/// same names, and for the built-in `claude`, `codex` and `pi` what is the same for every agent: the standard access, which a
/// session's profile replaces through the request's `_meta`, and every mode the agent has, Bypass included. A setting the
/// entry's `settings` names is the person's own and stays.
pub(crate) fn harness_settings(
    registry: &coqui_acp_api::guatiao::library::Registry,
    entry: &Entry,
    harness: &str,
    executable: Option<&OsString>,
    asks: &HarnessOptions,
    own: Option<&Settings>,
) -> Settings {
    // A provider that takes none of the access, mode or setting-source keys (one that runs a stdio agent as it is) is given only
    // the program and the limits of a connection: each key below goes in where the provider's schema declares it.
    let takes = |key: &str| crate::kind::declares_setting(registry, harness, key);
    let mut map = builtin_provider_settings(registry, harness, executable, own)
        .as_map()
        .clone();
    if let Some(sources) = &asks.setting_sources
        && takes("setting_sources")
    {
        map.insert(
            "setting_sources".to_owned(),
            serde_json::Value::from(sources.clone()),
        );
    }
    if let Some(modes) = map.get("modes") {
        tracing::info!(
            "host.agents.{}.settings.modes is {modes}: a paired client may choose those besides the entry's own mode and stricter ones",
            entry.name
        );
    }
    // The access is the profile's, the same for every harness: this is the built-in layer's, for a session whose entry offers
    // no profile. A session that has one is given its own through `_meta`, which the scope has stripped of a client's.
    if takes("access") {
        map.entry("access".to_owned())
            .or_insert_with(|| coqui_acp_harness::Access::STANDARD.to_json());
    }
    if takes("allow_client_access") {
        map.insert(
            "allow_client_access".to_owned(),
            serde_json::Value::Bool(true),
        );
    }
    // Requests a connection works on at once: what the host always allowed, not the provider's smaller default.
    map.entry("max_requests".to_owned())
        .or_insert_with(|| serde_json::Value::from(HARNESS_MAX_REQUESTS));
    Settings::new(map)
}

/// Whether an offered account's credential reaches the agent in its environment, for a kind that names the model's address:
/// the login kinds keep theirs in a file, and every other kind with an address is a key or a token. An account another
/// provider made, that provides an API standard, is a key whatever its kind says of where it rests.
pub(crate) fn keeps_key_in_environment(
    makers: &crate::makers::Makers,
    listed: &[kind::Offered],
    runs: &str,
) -> bool {
    listed.iter().any(|(_, account, info)| {
        let borrowed = crate::accounts::matching::borrowed(makers, account, runs)
            && info
                .provides
                .iter()
                .any(|standard| standard.ends_with("-api"));
        borrowed
            || (!info.state_dir
                && info
                    .endpoints
                    .iter()
                    .any(|slot| slot.purpose == coqui_acp_api::MODEL_API && slot.key.is_some()))
    })
}

/// The accounts an entry of `harness` offers, each with its kind in the catalog of the provider that made it, once canoa.cfg's
/// settings for it are checked against that kind's schema. An account another provider made is offered when what its kind
/// provides meets what `harness` accepts.
pub(crate) fn offered_accounts(
    config: &Config,
    harness: &str,
    names: &[String],
    provider: &kind::Harness,
) -> Result<Vec<kind::Offered>, String> {
    let catalog = provider
        .catalog()
        .ok_or_else(|| "the provider did not list the accounts it takes".to_owned())?;
    let catalogs = Catalogs::with(
        config,
        harness,
        (catalog, provider.listing()),
        provider.local_account().is_some(),
    );
    let mut listed = Vec::new();
    for name in names {
        if name == coqui_acp_api::LOCAL_ACCOUNT {
            let Some(local) = provider.local_account() else {
                return Err(format!(
                    "account {name}: {harness} has no login of its own to offer as it"
                ));
            };
            listed.push(local);
            continue;
        }
        let Some(account) = config.accounts.get(name) else {
            return Err(format!("account {name} is not in [host.accounts]"));
        };
        let Some(info) = catalogs.kind(account) else {
            return Err(format!(
                "account {name}: the provider has no account kind {}",
                account.kind
            ));
        };
        fits(
            &crate::makers::Makers::of(config),
            catalog,
            harness,
            account,
            &info,
        )
        .map_err(|why| format!("account {name} cannot run on {harness}: {why}"))?;
        crate::accounts::check_settings(&info, account.settings.as_ref())
            .map_err(|why| format!("account {name}: {why}"))?;
        listed.push((name.clone(), account.clone(), info));
    }
    Ok(listed)
}

/// The accounts an entry of `harness` offers: those it names, else the automatic `local` account when the harness has a login of
/// its own. `None` when it offers none.
///
/// # Errors
///
/// As [`offered_accounts`].
pub(crate) fn entry_accounts(
    config: &Config,
    harness: &str,
    names: &[String],
    provider: &kind::Harness,
) -> Result<Option<Vec<kind::Offered>>, String> {
    if !names.is_empty() {
        // A pool the entry names offers the accounts it holds that can run here.
        let accounts = match provider.catalog() {
            Some(catalog) => crate::pools::expand(
                config,
                &Catalogs::with(
                    config,
                    harness,
                    (catalog, provider.listing()),
                    provider.local_account().is_some(),
                ),
                harness,
                names,
            )?,
            None => names.to_vec(),
        };
        return offered_accounts(config, harness, &accounts, provider).map(Some);
    }
    Ok(provider.local_account().map(|local| vec![local]))
}

/// The accounts, of `harness` or of another provider whose standards it accepts, that an entry which does not list them can
/// still run a session on, for a key trusted with accounts. One whose key reaches the agent in its environment is left out:
/// only an entry that lists it limits the settings files a repository could use to redirect that key, so it has to be listed.
pub(crate) fn other_accounts(
    config: &Config,
    harness: &str,
    listed: &[kind::Offered],
    provider: &kind::Harness,
) -> Vec<kind::Offered> {
    let Some(catalog) = provider.catalog() else {
        return Vec::new();
    };
    let catalogs = Catalogs::with(
        config,
        harness,
        (catalog, provider.listing()),
        provider.local_account().is_some(),
    );
    let mut others = Vec::new();
    for (name, account) in &config.accounts {
        if listed.iter().any(|(each, ..)| each == name) {
            continue;
        }
        let Some(info) = catalogs.kind(account) else {
            continue;
        };
        if fits(
            &crate::makers::Makers::of(config),
            catalog,
            harness,
            account,
            &info,
        )
        .is_err()
        {
            continue;
        }
        if crate::accounts::check_settings(&info, account.settings.as_ref()).is_err() {
            continue;
        }
        let one = (name.clone(), account.clone(), info);
        if keeps_key_in_environment(
            &crate::makers::Makers::of(config),
            core::slice::from_ref(&one),
            harness,
        ) {
            continue;
        }
        others.push(one);
    }
    others
}

/// The accounts of other providers that `provider`'s harness cannot run a session on, each with why: the standards the account
/// provides and the ones the harness accepts.
pub(crate) fn unfit_accounts(
    config: &Config,
    harness: &str,
    provider: &kind::Harness,
) -> Vec<(String, String)> {
    let Some(catalog) = provider.catalog() else {
        return Vec::new();
    };
    let catalogs = Catalogs::with(
        config,
        harness,
        (catalog, provider.listing()),
        provider.local_account().is_some(),
    );
    config
        .accounts
        .iter()
        .filter_map(|(name, account)| {
            let info = catalogs.kind(account)?;
            fits(
                &crate::makers::Makers::of(config),
                catalog,
                harness,
                account,
                &info,
            )
            .err()
            .map(|why| (name.clone(), why))
        })
        .collect()
}
