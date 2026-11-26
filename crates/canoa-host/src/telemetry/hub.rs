//! Delivery: each exporter has a lane, a bounded queue the host's threads push to without waiting, and a thread of its own that
//! takes a batch every few seconds (or when enough wait), calls the exporter and, when it fails, keeps the batch and tries again
//! after a pause that doubles from a second to a minute. A full queue lets go of its oldest signal and counts it; an exporter that
//! stalls holds its own thread and nothing else.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, locks and clocks are std's"
)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use canoa::telemetry::{Attr, Batch, Metric, Signals, TelemetryExporter, is_retried};
use serde_json::{Value, json};

use super::signal::{ExporterReason, Record, Signal, Text};

/// How many signals make a batch, and how many waiting send it before its time.
pub(crate) const BATCH_MAX: usize = 512;
/// The pause after a first failure, and the longest one.
pub(crate) const FIRST_PAUSE: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The current time in Unix nanoseconds.
pub(crate) fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(0)
}

/// What the configuration says of one exporter.
#[derive(Clone, Debug)]
pub(crate) struct LaneOptions {
    pub(crate) name: String,
    pub(crate) queue: usize,
    pub(crate) batch: Duration,
    pub(crate) sample: f64,
    /// The pause after a batch first fails, which doubles for each try after it up to a minute.
    pub(crate) first_pause: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    Idle,
    Retrying,
    Rejected,
}

impl Health {
    const fn word(self) -> &'static str {
        match self {
            Self::Idle => "ok",
            Self::Retrying => "retrying",
            Self::Rejected => "rejected",
        }
    }
}

struct LaneState {
    queue: VecDeque<Record>,
    /// Signals let go of since the last batch, which the next batch reports.
    unreported: u64,
    dropped: u64,
    exported: u64,
    health: Health,
    last_error: Option<String>,
    stopping: bool,
    finished: bool,
}

/// One exporter's queue and thread.
pub(crate) struct Lane {
    options: LaneOptions,
    takes: Vec<Signals>,
    exporter: Arc<dyn TelemetryExporter>,
    state: Mutex<LaneState>,
    wake: Condvar,
}

impl Lane {
    fn push(&self, record: Record) {
        let mut state = lock(&self.state);
        if state.queue.len() >= self.options.queue.max(1) {
            state.queue.pop_front();
            state.unreported += 1;
            state.dropped += 1;
        }
        state.queue.push_back(record);
        if state.queue.len() >= BATCH_MAX {
            self.wake.notify_all();
        }
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        lock(&self.state).queue.len()
    }

    fn status(&self) -> Value {
        let state = lock(&self.state);
        let mut row = json!({
            "name": self.options.name,
            "state": state.health.word(),
            "queued": state.queue.len(),
            "dropped": state.dropped,
            "exported": state.exported,
        });
        if let (Some(why), Some(object)) = (&state.last_error, row.as_object_mut()) {
            object.insert("last_error".to_owned(), json!(why));
        }
        row
    }

    /// The pause between two tries, cut short by a stop.
    fn pause(&self, length: Duration) {
        let until = Instant::now() + length;
        let mut state = lock(&self.state);
        while !state.stopping {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .wake
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn take(&self, resource: &[Attr]) -> Option<(Batch, usize)> {
        let (records, reported) = {
            let mut state = lock(&self.state);
            let end = state.queue.len().min(BATCH_MAX);
            let records: Vec<Record> = state.queue.drain(..end).collect();
            (records, core::mem::take(&mut state.unreported))
        };
        let mut batch = Batch::new(resource.to_vec());
        let mut count = records.len();
        for record in records {
            add(&mut batch, record);
        }
        if reported > 0 && self.takes.contains(&Signals::Metrics) {
            add(
                &mut batch,
                Signal::Dropped {
                    exporter: Text::new(&self.options.name),
                    count: reported,
                    at: now_ns(),
                }
                .into_record(),
            );
            count += 1;
        }
        (count > 0).then_some((batch, count))
    }

    fn run(&self, resource: &[Attr]) {
        loop {
            {
                let mut state = lock(&self.state);
                let until = Instant::now() + self.options.batch;
                while !state.stopping && state.queue.len() < BATCH_MAX {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    state = self
                        .wake
                        .wait_timeout(state, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
            let Some((batch, count)) = self.take(resource) else {
                if lock(&self.state).stopping {
                    break;
                }
                continue;
            };
            self.deliver(&batch, count);
        }
        if let Err(error) = self.exporter.flush() {
            self.note_failure(&error, false);
        }
        lock(&self.state).finished = true;
    }

    fn deliver(&self, batch: &Batch, count: usize) {
        let mut pause = self.options.first_pause;
        let mut failing = false;
        loop {
            match self.exporter.export(batch.clone()) {
                Ok(()) => {
                    let mut state = lock(&self.state);
                    state.exported += count as u64;
                    state.health = Health::Idle;
                    state.last_error = None;
                    return;
                }
                Err(error) => {
                    let again = is_retried(&error);
                    if !failing {
                        // One event for a streak of failures: not another for each batch an exporter that keeps failing refuses.
                        let was_healthy = lock(&self.state).health == Health::Idle;
                        self.note_failure(&error, was_healthy);
                    }
                    failing = true;
                    let mut state = lock(&self.state);
                    state.last_error = Some(error.message().to_owned());
                    if !again {
                        state.health = Health::Rejected;
                        state.dropped += count as u64;
                        return;
                    }
                    state.health = Health::Retrying;
                    if state.stopping {
                        state.dropped += count as u64;
                        return;
                    }
                    drop(state);
                    self.pause(pause);
                    pause = (pause * 2).min(LONGEST_PAUSE);
                }
            }
        }
    }

    /// Queues the one `exporter.failed` event of a failing streak, for the exporter itself to carry once it can.
    fn note_failure(&self, error: &coqui_acp_api::guatiao::library::ProviderError, queue: bool) {
        let mut state = lock(&self.state);
        state.last_error = Some(error.message().to_owned());
        drop(state);
        if queue && self.takes.contains(&Signals::Logs) {
            let reason = if error.message().starts_with("auth: ") {
                ExporterReason::Auth
            } else if is_retried(error) {
                ExporterReason::Unavailable
            } else {
                ExporterReason::Rejected
            };
            self.push(
                Signal::ExporterFailed {
                    reason,
                    at: now_ns(),
                }
                .into_record(),
            );
        }
    }
}

fn add(batch: &mut Batch, record: Record) {
    match record {
        Record::Span(span) => batch.spans.push(span),
        Record::Log(log) => batch.logs.push(log),
        Record::Metric {
            name,
            unit,
            kind,
            point,
        } => match batch.metrics.iter_mut().find(|each| each.name == name) {
            Some(each) => each.points.push(point),
            None => batch
                .metrics
                .push(Metric::new(name, unit, kind, vec![point])),
        },
    }
}

/// The exporters of a running host.
pub(crate) struct Hub {
    lanes: Vec<Arc<Lane>>,
    record: Vec<Signals>,
    stopping: AtomicBool,
    /// Sessions open now, by agent.
    active: Mutex<std::collections::BTreeMap<String, u64>>,
}

impl core::fmt::Debug for Hub {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hub")
            .field("lanes", &self.lanes.len())
            .finish_non_exhaustive()
    }
}

/// What `from` says an exporter takes, kept to what the person records.
fn takes_of(from: &[Signals], record: &[Signals]) -> Vec<Signals> {
    from.iter()
        .copied()
        .filter(|group| record.contains(group))
        .collect()
}

impl Hub {
    /// A hub delivering to `lanes`, each as its options say, what the person records being `record`; each lane's thread starts.
    pub(crate) fn new(
        lanes: Vec<(LaneOptions, Arc<dyn TelemetryExporter>, Vec<Signals>)>,
        record: Vec<Signals>,
        resource: Vec<Attr>,
    ) -> Arc<Self> {
        let lanes: Vec<Arc<Lane>> = lanes
            .into_iter()
            .map(|(options, exporter, takes)| {
                Arc::new(Lane {
                    takes: takes_of(&takes, &record),
                    options,
                    exporter,
                    state: Mutex::new(LaneState {
                        queue: VecDeque::new(),
                        unreported: 0,
                        dropped: 0,
                        exported: 0,
                        health: Health::Idle,
                        last_error: None,
                        stopping: false,
                        finished: false,
                    }),
                    wake: Condvar::new(),
                })
            })
            .collect();
        for lane in &lanes {
            let lane = Arc::clone(lane);
            let resource = resource.clone();
            let name = format!("canoa-telemetry-{}", lane.options.name);
            if let Err(error) = thread::Builder::new()
                .name(name)
                .spawn(move || lane.run(&resource))
            {
                tracing::warn!("a telemetry exporter's thread could not start: {error}");
            }
        }
        Arc::new(Self {
            lanes,
            record,
            stopping: AtomicBool::new(false),
            active: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    /// Whether the host is stopping, so an open session ends as `host_stopped`.
    pub(crate) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// A session of `harness` opened (`up`) or ended, and how many are open now.
    pub(crate) fn sessions_changed(&self, harness: &str, up: bool) -> u64 {
        let mut active = lock(&self.active);
        let count = active.entry(harness.to_owned()).or_insert(0);
        *count = if up {
            *count + 1
        } else {
            count.saturating_sub(1)
        };
        *count
    }

    /// Hands `signal` to every exporter that takes its group. It never waits on an exporter.
    pub(crate) fn emit(&self, signal: Signal) {
        let group = signal.group();
        if !self.record.contains(&group) {
            return;
        }
        let key = signal.sample_key();
        let mut record = None;
        for lane in &self.lanes {
            if !lane.takes.contains(&group) {
                continue;
            }
            if let Some(key) = key
                && !sampled(key, lane.options.sample)
            {
                continue;
            }
            let record = record.get_or_insert_with(|| signal.clone().into_record());
            lane.push(record.clone());
        }
    }

    /// Each exporter's name, health and counts.
    pub(crate) fn lanes(&self) -> Vec<Value> {
        self.lanes.iter().map(|lane| lane.status()).collect()
    }

    /// How many signals wait in every queue.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        self.lanes.iter().map(|lane| lane.queued()).sum()
    }

    /// The signals waiting in the queue of the exporter `name`, as batch records.
    #[cfg(test)]
    pub(crate) fn waiting(&self, name: &str) -> Vec<Record> {
        self.lanes
            .iter()
            .find(|lane| lane.options.name == name)
            .map(|lane| lock(&lane.state).queue.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Ends every lane: what waits is delivered once (no retry), each exporter is asked to flush, and the call returns when they have
    /// or after `wait`.
    pub(crate) fn stop(&self, wait: Duration) {
        self.stopping.store(true, Ordering::SeqCst);
        for lane in &self.lanes {
            lock(&lane.state).stopping = true;
            lane.wake.notify_all();
        }
        let until = Instant::now() + wait;
        while Instant::now() < until && self.lanes.iter().any(|lane| !lock(&lane.state).finished) {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Whether the session with `key` is among the fraction `sample` of them.
fn sampled(key: u64, sample: f64) -> bool {
    if sample >= 1.0 {
        return true;
    }
    if sample <= 0.0 {
        return false;
    }
    // The key is random, so its place in 0..=u64::MAX is a fair draw that stays the same for the whole session.
    #[allow(clippy::cast_precision_loss, reason = "a fraction, not an exact count")]
    let place = key as f64 / u64::MAX as f64;
    place < sample
}

#[cfg(test)]
pub(crate) fn sampled_for_test(key: u64, sample: f64) -> bool {
    sampled(key, sample)
}
