//! Telemetry export: the host's traces, metrics and events, delivered to the exporters the person named, when the person turned that
//! on and accepted the list of what is recorded.
//!
//! Nothing here records free text: [`catalog`] is the one list of what is recorded and what never is, and [`signal::Signal`] has a
//! variant for each entry whose attributes are numbers, words of a closed set, or a short name cut to a safe character set. The
//! observer ([`watch`]) reads the ACP messages that pass through a connection and emits those signals. [`hub`] queues them for each
//! exporter, one thread of its own each, so no exporter can delay a session. The exporters are providers of the `telemetry_exporter`
//! kind: the host carries `file` ([`file`]), and a library or a source adds others.
//!
//! Export runs only while `[host.telemetry] enabled` is true, `acknowledged` is the catalog's digest and an exporter is named;
//! otherwise nothing is queued. A change to the catalog changes the digest, which pauses export until the person accepts the new
//! list. The settings are read when the host starts.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads and clocks are std's"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa::telemetry::{Attr, Batch, ExporterInfo, LogRecord, Signals, TelemetryExporter};
use coqui_acp_api::guatiao::Alloc;
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Map, Value, json};

use crate::config::{Config, TelemetryConfig, TelemetryExporterConfig};

pub(crate) mod catalog;
pub(crate) mod file;
mod hub;
pub(crate) mod signal;
#[cfg(test)]
mod tests;
pub(crate) mod watch;

use hub::{Hub, LaneOptions, now_ns};
use signal::Signal;

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-telemetry",
    version = env!("CARGO_PKG_VERSION"),
    providers = [file::FileExporter]
);

/// The origin of the providers the host itself carries.
const BUILT_IN: &str = "built-in";

/// One host's telemetry: the hub its connections tell, and what it needs to make an exporter. Each host makes its own, so two hosts
/// in one process keep their signals, exporters and files apart.
#[derive(Debug, Default)]
pub(crate) struct Telemetry {
    /// The host's home, which `telemetry/` of an exporter that names no folder is in.
    home: Option<PathBuf>,
    /// The hub and the longest [`Telemetry::stop`] waits for it, while export runs.
    running: Mutex<Option<(Arc<Hub>, Duration)>>,
    /// Why each exporter that was named did not start.
    failed: Mutex<BTreeMap<String, String>>,
}

/// Whether export runs, and if not why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// `enabled` is false.
    Off,
    /// The person turned it on, and something is missing.
    Paused(String),
    /// Signals are queued for the exporters.
    Running,
}

/// What `config` makes of the switch, the acknowledgement and the exporters.
pub(crate) fn state_of(config: &TelemetryConfig) -> State {
    if !config.enabled {
        State::Off
    } else if config.acknowledged.is_empty() {
        State::Paused("the list of what is recorded has not been accepted".to_owned())
    } else if config.acknowledged != catalog::digest() {
        State::Paused(
            "the list of what is recorded changed since it was accepted: read it and accept it again"
                .to_owned(),
        )
    } else if config.exporters.is_empty() {
        State::Paused("no exporter is named".to_owned())
    } else {
        State::Running
    }
}

fn groups_of(config: &TelemetryConfig) -> Vec<Signals> {
    [Signals::Spans, Signals::Metrics, Signals::Logs]
        .into_iter()
        .filter(|group| config.record.iter().any(|word| word == group.word()))
        .collect()
}

fn resource() -> Vec<Attr> {
    vec![
        Attr::text("service.name", canoa::brand::PROGRAM),
        Attr::text("service.version", env!("CARGO_PKG_VERSION")),
        Attr::text("service.instance.id", format!("{:016x}", watch::random())),
        Attr::text("os.type", std::env::consts::OS),
        Attr::text("host.arch", std::env::consts::ARCH),
    ]
}

fn built_in() -> Option<Registry> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("canoa-telemetry", library)
        .ok()?
        .loaded()?;
    Some(registry)
}

/// Every registry that may offer an exporter: the host's own, the libraries of the configuration and the one `exporter` names.
fn registries_for(
    config: &Config,
    exporter: Option<&TelemetryExporterConfig>,
) -> Vec<(String, Arc<Registry>)> {
    let mut found = Vec::new();
    if let Some(registry) = built_in() {
        found.push((BUILT_IN.to_owned(), Arc::new(registry)));
    }
    found.extend(crate::control::providers::loaded(config).iter().cloned());
    if let Some(origin) = exporter.and_then(|each| each.origin.as_ref()) {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        match crate::provider::open(&mut registry, origin) {
            Ok(()) => found.push(("exporter".to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("a telemetry exporter's provider is not loaded: {why}"),
        }
    }
    found
}

fn guatiao_of(settings: &Map<String, Value>) -> Result<coqui_acp_api::guatiao::Value, String> {
    guatiao_serde::adapt::to_value_in(&Value::Object(settings.clone()), Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())
}

/// The exporter `each` names, made with its settings.
fn make(
    config: &Config,
    each: &TelemetryExporterConfig,
    home: Option<&Path>,
) -> Result<Arc<dyn TelemetryExporter>, String> {
    let registries = registries_for(config, Some(each));
    for (_, registry) in &registries {
        let keys: Vec<String> = registry
            .offers::<dyn TelemetryExporter>()
            .filter_map(|offer| offer.key().map(str::to_owned))
            .collect();
        for key in keys {
            let Some(Ok(offer)) = registry.offer::<dyn TelemetryExporter>(&key) else {
                continue;
            };
            if offer.id() != each.provider {
                continue;
            }
            return if offer.builds_instances() {
                let value = guatiao_of(&file::with_home(
                    &each.provider,
                    each.settings.as_map(),
                    home,
                ))?;
                let built = offer
                    .instantiate(&value)
                    .map_err(|error| error.to_string())?;
                Ok(Arc::new(built))
            } else if each.settings.as_map().is_empty() {
                offer
                    .shared()
                    .ok_or_else(|| format!("{} needs settings", each.provider))
            } else {
                Err(format!("{} takes no settings", each.provider))
            };
        }
    }
    Err(format!(
        "this host offers no telemetry exporter {}",
        each.provider
    ))
}

impl Telemetry {
    /// Export that does not run, for a host that has none: exporters made through it still find `home`.
    pub(crate) fn off(home: Option<&Path>) -> Self {
        Self {
            home: home.map(Path::to_path_buf),
            ..Self::default()
        }
    }

    /// Starts exporting as `config` says, in `home`. Logs at `warn` why export is paused when the person turned it on and it
    /// cannot run.
    pub(crate) fn start(config: &Config, home: Option<&Path>) -> Self {
        let this = Self::off(home);
        match state_of(&config.telemetry) {
            State::Off => return this,
            State::Paused(why) => {
                tracing::warn!("telemetry export is paused: {why}");
                return this;
            }
            State::Running => {}
        }
        let record = groups_of(&config.telemetry);
        let mut lanes = Vec::new();
        for (name, each) in &config.telemetry.exporters {
            let made = this.make(config, each).and_then(|exporter| {
                let info: ExporterInfo = exporter.describe().map_err(|error| error.to_string())?;
                Ok((exporter, info.signals))
            });
            match made {
                Ok((exporter, takes)) => lanes.push((
                    LaneOptions {
                        name: name.clone(),
                        queue: each.queue,
                        batch: Duration::from_secs(each.batch_seconds),
                        sample: f64::from(each.sample)
                            / f64::from(crate::config::TELEMETRY_SAMPLE_ALL),
                        first_pause: hub::FIRST_PAUSE,
                    },
                    exporter,
                    takes,
                )),
                Err(why) => {
                    tracing::warn!("telemetry exporter {name} is not started: {why}");
                    this.failed
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(name.clone(), why);
                }
            }
        }
        if lanes.is_empty() {
            return this;
        }
        let hub = Hub::new(lanes, record, resource());
        *this.running.lock().unwrap_or_else(PoisonError::into_inner) = Some((
            Arc::clone(&hub),
            Duration::from_secs(config.telemetry.stop_wait_seconds),
        ));
        hub.emit(Signal::HostStarted {
            version: signal::Text::new(env!("CARGO_PKG_VERSION")),
            at: now_ns(),
        });
        this
    }

    /// Export that runs through `hub`.
    #[cfg(test)]
    pub(crate) fn over(hub: Arc<Hub>) -> Self {
        let this = Self::off(None);
        *this.running.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((hub, Duration::from_secs(10)));
        this
    }

    /// The hub, while export runs.
    pub(crate) fn hub(&self) -> Option<Arc<Hub>> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|(hub, _)| Arc::clone(hub))
    }

    /// A watch on a connection of `harness`, when export runs.
    pub(crate) fn watch(&self, harness: &str) -> Option<watch::Watch> {
        self.hub().map(|hub| watch::Watch::with_hub(hub, harness))
    }

    /// A notice of `event` was delivered to `notifier`, or `failed`.
    pub(crate) fn notice_told(&self, event: &str, notifier: &str, failed: bool) {
        self.emit(|at| Signal::Notifications {
            event: signal::Text::new(event),
            notifier: signal::Text::new(notifier),
            result: if failed {
                signal::NoticeResult::Failed
            } else {
                signal::NoticeResult::Sent
            },
            at,
        });
    }

    /// The host started again after an update.
    pub(crate) fn host_updated(&self) {
        self.emit(|at| Signal::HostRestarted {
            reason: signal::RestartReason::Updated,
            at,
        });
    }

    /// An install or an update failed. The reason is not told apart where it is noticed, so it is `other`.
    pub(crate) fn update_failed(&self) {
        self.emit(|at| Signal::UpdateFailed {
            reason: signal::FailReason::Other,
            at,
        });
    }

    fn emit(&self, signal: impl FnOnce(i64) -> Signal) {
        if let Some(hub) = self.hub() {
            hub.emit(signal(now_ns()));
        }
    }

    /// Ends export: what waits is delivered once and the exporters are asked to finish, for as long as `stop_wait_seconds` allows.
    pub(crate) fn stop(&self) {
        let held = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((hub, wait)) = held {
            hub.emit(Signal::HostStopped { at: now_ns() });
            hub.stop(wait);
        }
    }

    /// The exporter `each` names, made with its settings. The `file` exporter that names no folder writes in this host's home.
    pub(crate) fn make(
        &self,
        config: &Config,
        each: &TelemetryExporterConfig,
    ) -> Result<Arc<dyn TelemetryExporter>, String> {
        make(config, each, self.home.as_deref())
    }
}

/// `telemetry/catalog`: what is recorded and what never is.
pub(crate) fn catalog_answer() -> Value {
    catalog::json()
}

/// The exporter providers the host offers, as `{id, title, schema}`.
fn providers_of(config: &Config) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    for (_, registry) in registries_for(config, None) {
        for key in registry
            .offers::<dyn TelemetryExporter>()
            .filter_map(|offer| offer.key().map(str::to_owned))
            .collect::<Vec<_>>()
        {
            let Some(Ok(offer)) = registry.offer::<dyn TelemetryExporter>(&key) else {
                continue;
            };
            if rows.iter().any(|row| row["id"] == offer.id()) {
                continue;
            }
            let schema = offer
                .config_schema()
                .and_then(|schema| crate::forms::json_of(schema).ok());
            rows.push(json!({
                "id": offer.id(),
                "title": offer.display_name(),
                "schema": schema,
            }));
        }
    }
    rows
}

/// `telemetry/status`: whether export runs, why not when it does not, and how each exporter is doing.
pub(crate) fn status_answer(this: &Telemetry, config: &Config) -> Value {
    let telemetry = &config.telemetry;
    let state = state_of(telemetry);
    let lanes = this.hub().map(|hub| hub.lanes()).unwrap_or_default();
    let started = this
        .failed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let exporters: Vec<Value> = telemetry
        .exporters
        .iter()
        .map(|(name, each)| {
            let mut row = lanes
                .iter()
                .find(|lane| lane["name"] == name.as_str())
                .cloned()
                .unwrap_or_else(|| {
                    json!({"name": name, "state": "stopped", "queued": 0, "dropped": 0, "exported": 0})
                });
            if let Some(object) = row.as_object_mut() {
                object.insert("provider".to_owned(), json!(each.provider));
                if let Some(why) = started.get(name) {
                    object.insert("last_error".to_owned(), json!(why));
                }
            }
            row
        })
        .collect();
    json!({
        "enabled": telemetry.enabled,
        "state": match &state { State::Off => "off", State::Paused(_) => "paused", State::Running => "running" },
        "paused": match &state { State::Paused(why) => Some(why.clone()), _ => None },
        "digest": catalog::digest(),
        "acknowledged": telemetry.acknowledged,
        "record": telemetry.record,
        "exporters": exporters,
        "providers": providers_of(config),
    })
}

/// `telemetry/test` `{name}`: one `host.test` record through the exporter `name`, made as the configuration says, running or not.
pub(crate) fn test_answer(this: &Telemetry, config: &Config, name: &str) -> Value {
    let Some(each) = config.telemetry.exporters.get(name) else {
        return json!({"ok": false, "why": format!("no exporter {name}")});
    };
    let sent = this.make(config, each).and_then(|exporter| {
        let mut batch = Batch::new(resource());
        batch
            .logs
            .push(LogRecord::new(now_ns(), "host.test", Vec::new()));
        exporter
            .export(batch)
            .map_err(|error| error.message().to_owned())
    });
    match sent {
        Ok(()) => json!({"ok": true}),
        Err(why) => json!({"ok": false, "why": why}),
    }
}
