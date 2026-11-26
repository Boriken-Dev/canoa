//! Telling the person what happened on the host. The host decides which events happen and who is told: each event has a list of
//! notifiers, a notifier is a provider of the `notifier` kind, and the host builds every notice from fixed words and ids. The
//! application shows a toast only as one notifier, `desktop`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the hub is the host's"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::config_store::ConfigStore;

pub(crate) mod desktop;
pub(crate) mod directory;
pub(crate) mod events;
pub(crate) mod hub;
pub(crate) mod observer;
pub(crate) mod run;
pub(crate) mod words;

#[cfg(test)]
mod tests;

pub(crate) use directory::Directory;
pub(crate) use hub::{Hub, Occurrence};
pub(crate) use observer::Observed;

use crate::config::Config;

/// The directory of a host whose `[notifications]` is in `store`: read again whenever the store's revision changes, so a change in
/// Settings reaches the next event.
pub(crate) fn watching(store: Arc<dyn ConfigStore>, home: Option<PathBuf>) -> hub::Source {
    let cached: Mutex<Option<(String, Arc<Directory>)>> = Mutex::new(None);
    Box::new(move || {
        let revision = store.revision().map_err(|error| error.to_string())?;
        let mut held = cached.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((seen, directory)) = held.as_ref()
            && *seen == revision
        {
            return Ok(Arc::clone(directory));
        }
        let config = Arc::new(Config::from_store(&*store).map_err(|error| error.to_string())?);
        let directory = Arc::new(Directory::of(&config, keeping(&config, home.clone())));
        *held = Some((revision, Arc::clone(&directory)));
        Ok(directory)
    })
}

/// The directory of a host with no store to follow: what `config` said when it started.
pub(crate) fn fixed(config: &Config, home: Option<PathBuf>) -> hub::Source {
    let config = Arc::new(config.clone());
    let directory = Arc::new(Directory::of(&config, keeping(&config, home)));
    Box::new(move || Ok(Arc::clone(&directory)))
}

/// Where a notifier's sensitive members are kept: the host's secrets, under `notifiers/NAME/MEMBER`.
fn keeping(config: &Arc<Config>, home: Option<PathBuf>) -> Option<directory::Keeping> {
    let home = home?;
    let config = Arc::clone(config);
    Some(Arc::new(move |name: &str| {
        crate::control::settings_sections::Keeper::new(&config, Some(&home))
            .get_named(name)
            .ok()
            .flatten()
            .map(|secret| secret.to_string())
    }))
}
