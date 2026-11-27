//! Telemetry export: the catalog and the signals agree, nothing a session says can be an attribute, export is off until the person
//! turned it on and accepted the list, a full or stalled exporter never holds a turn, and the files the `file` exporter writes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use canoa::telemetry::{
    Batch, ExporterError, ExporterInfo, MetricKind, Signals, TelemetryExporter,
};
use coqui_acp_api::guatiao::library::ProviderError;
use serde_json::{Value, json};

use super::catalog::{self, CATALOG, Kind, Ty};
use super::hub::{Hub, LaneOptions};
use super::signal::{self, Record, Signal, Text};
use super::watch::Watch;
use super::{State, Telemetry, state_of};
use crate::config::{Config, TelemetryConfig};

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "canoa-telemetry-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < limit, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn config(text: &str) -> Config {
    Config::from_str_in(text, Path::new("")).expect("parses")
}

// ---- the catalog and the signals ----

#[test]
fn every_signal_is_in_the_catalog_with_its_attributes_and_every_entry_of_it_has_a_signal() {
    let samples = signal::samples();
    let named: BTreeSet<&str> = samples.iter().map(Signal::name).collect();
    assert_eq!(named.len(), samples.len(), "a signal is sampled twice");
    let listed: BTreeSet<&str> = CATALOG.iter().map(|def| def.name).collect();
    assert_eq!(
        named, listed,
        "a signal the catalog lacks, or an entry no signal carries"
    );
    for sample in samples {
        let def = catalog::find(sample.name()).expect("listed");
        let carried: Vec<String> = sample.attrs().into_iter().map(|attr| attr.key).collect();
        let listed: Vec<&str> = def.attrs.iter().map(|attr| attr.name).collect();
        assert_eq!(
            carried, listed,
            "{} carries other attributes than the catalog lists",
            def.name
        );
        assert_eq!(sample.group(), def.group, "{}", def.name);
        match (sample.into_record(), def.kind) {
            (Record::Span(_), Kind::Span) | (Record::Log(_), Kind::Event) => {}
            (Record::Metric { kind, .. }, Kind::Counter | Kind::Histogram | Kind::Gauge) => {
                let want = match def.kind {
                    Kind::Counter => MetricKind::Counter,
                    Kind::Histogram => MetricKind::Histogram,
                    _ => MetricKind::Gauge,
                };
                assert_eq!(kind, want, "{}", def.name);
            }
            _ => panic!(
                "{} is another kind of signal than the catalog says",
                def.name
            ),
        }
    }
}

#[test]
fn a_word_of_a_closed_set_is_one_the_catalog_lists_and_the_other_way_round() {
    fn same<T: Copy>(
        listed: &[&str],
        all: &[T],
        word: impl Fn(T) -> &'static str,
        parse: impl Fn(&str) -> Option<T>,
    ) {
        let words: Vec<&str> = all.iter().map(|each| word(*each)).collect();
        assert_eq!(words, listed);
        for each in listed {
            assert!(parse(each).is_some(), "{each}");
        }
    }
    use signal::*;
    same(
        StopReason::listed(),
        StopReason::ALL,
        StopReason::word,
        StopReason::parse,
    );
    same(
        SessionEnd::listed(),
        SessionEnd::ALL,
        SessionEnd::word,
        SessionEnd::parse,
    );
    same(
        ToolKind::listed(),
        ToolKind::ALL,
        ToolKind::word,
        ToolKind::parse,
    );
    same(
        ToolStatus::listed(),
        ToolStatus::ALL,
        ToolStatus::word,
        ToolStatus::parse,
    );
    same(
        Outcome::listed(),
        Outcome::ALL,
        Outcome::word,
        Outcome::parse,
    );
    same(
        DecidedBy::listed(),
        DecidedBy::ALL,
        DecidedBy::word,
        DecidedBy::parse,
    );
    same(
        TokenType::listed(),
        TokenType::ALL,
        TokenType::word,
        TokenType::parse,
    );
    same(Bucket::listed(), Bucket::ALL, Bucket::word, Bucket::parse);
    same(
        NoticeResult::listed(),
        NoticeResult::ALL,
        NoticeResult::word,
        NoticeResult::parse,
    );
    same(
        RestartReason::listed(),
        RestartReason::ALL,
        RestartReason::word,
        RestartReason::parse,
    );
    same(
        FailReason::listed(),
        FailReason::ALL,
        FailReason::word,
        FailReason::parse,
    );
    same(Scope::listed(), Scope::ALL, Scope::word, Scope::parse);
    same(
        ExporterReason::listed(),
        ExporterReason::ALL,
        ExporterReason::word,
        ExporterReason::parse,
    );
}

/// Whether an attribute name could carry what a session says: the words a name must not hold, except the token and cost counters.
fn says_what_a_session_says(name: &str) -> bool {
    const WORDS: [&str; 16] = [
        "text",
        "prompt",
        "content",
        "path",
        "file",
        "command",
        "input",
        "output",
        "cwd",
        "title",
        "account",
        "email",
        "secret",
        "token",
        "key",
        "host.name",
    ];
    // The counters of tokens and of cost are numbers an agent reported, and the kind of token is one of four words.
    let counter = name.starts_with("canoa.tokens.") || name == "canoa.token.type";
    !counter && WORDS.iter().any(|word| name.contains(word))
}

#[test]
fn no_attribute_name_could_carry_what_a_session_says() {
    let mut names: Vec<&str> = CATALOG
        .iter()
        .flat_map(|def| def.attrs.iter().map(|attr| attr.name))
        .collect();
    names.extend(catalog::RESOURCE);
    // The metric that counts tokens is named for them; it carries a number, and its attributes are listed above.
    for name in names {
        assert!(!says_what_a_session_says(name), "{name}");
    }
    // The control: the test finds names that would.
    for name in [
        "canoa.prompt.text",
        "canoa.tool.input",
        "canoa.file.path",
        "canoa.account.name",
        "canoa.user.email",
        "canoa.session.title",
        "canoa.working.cwd",
        "canoa.api.key",
        "host.name",
        "canoa.secret",
    ] {
        assert!(says_what_a_session_says(name), "{name}");
    }
}

#[test]
fn a_text_attribute_is_cut_and_cleaned_of_everything_outside_a_safe_set() {
    assert_eq!(
        Text::new("claude-sonnet-4.5:beta/x_1").as_str(),
        "claude-sonnet-4.5:beta/x_1"
    );
    assert_eq!(
        Text::new("two words\nand a line").as_str(),
        "two_words_and_a_line"
    );
    assert_eq!(
        Text::new("C:\\Users\\me\\secret.txt").as_str(),
        "C:_Users_me_secret.txt"
    );
    assert_eq!(Text::new("caf\u{e9} \u{1f600}").as_str(), "caf___");
    assert_eq!(Text::new(&"a".repeat(200)).as_str().len(), signal::MAX_TEXT);
    assert_eq!(Text::new("").as_str(), "unknown");
    assert!(Text::maybe("  ").is_none());
}

#[test]
fn the_digest_is_fixed_for_a_catalog_and_moves_with_any_change_to_it() {
    let digest = catalog::digest();
    assert_eq!(digest.len(), 16);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(catalog::digest(), digest);
    assert_ne!(
        catalog::digest_after(|text| text.push_str("  canoa.new|text|false\n")),
        digest
    );
    assert_ne!(
        catalog::digest_after(|text| *text = text.replace("never|", "never|The ")),
        digest
    );
    let listed = catalog::json();
    assert_eq!(listed["digest"], digest.as_str());
    assert_eq!(
        listed["signals"].as_array().expect("a list").len(),
        CATALOG.len()
    );
    assert!(
        listed["never"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|line| line.as_str().is_some_and(|line| line.contains("e-mail")))
    );
}

#[test]
fn a_text_attribute_of_the_catalog_is_exactly_the_places_a_short_name_is_taken() {
    let text: Vec<&str> = CATALOG
        .iter()
        .flat_map(|def| def.attrs.iter())
        .filter(|attr| matches!(attr.ty, Ty::Text))
        .map(|attr| attr.name)
        .collect();
    for name in [
        "canoa.harness",
        "canoa.model",
        "canoa.tool.name",
        "canoa.session.ref",
        "canoa.event",
        "canoa.notifier",
        "canoa.exporter",
        "canoa.version",
    ] {
        assert!(text.contains(&name), "{name}");
    }
    assert_eq!(
        text.iter().collect::<BTreeSet<_>>().len(),
        8,
        "a new text attribute takes a decision, and a place in this list"
    );
}

// ---- off by default, acknowledged, paused ----

#[test]
fn export_is_off_without_a_table_and_on_only_with_the_switch_the_digest_and_an_exporter() {
    let off = config("");
    assert_eq!(state_of(&off.telemetry), State::Off);
    assert_eq!(off.telemetry, TelemetryConfig::default());
    let on = |extra: &str| {
        config(&format!(
            "[host.telemetry]\nenabled = true\n{extra}\n[host.telemetry.exporters.local]\nprovider = \"file\"\n"
        ))
        .telemetry
    };
    assert!(matches!(state_of(&on("")), State::Paused(why) if why.contains("not been accepted")));
    assert!(
        matches!(state_of(&on("acknowledged = \"0123456789abcdef\"")), State::Paused(why) if why.contains("changed"))
    );
    assert_eq!(
        state_of(&on(&format!("acknowledged = \"{}\"", catalog::digest()))),
        State::Running
    );
    let none = config(&format!(
        "[host.telemetry]\nenabled = true\nacknowledged = \"{}\"\n",
        catalog::digest()
    ));
    assert!(matches!(state_of(&none.telemetry), State::Paused(why) if why.contains("no exporter")));
    let disabled = config(&format!(
        "[host.telemetry]\nenabled = false\nacknowledged = \"{}\"\n[host.telemetry.exporters.local]\nprovider = \"file\"\n",
        catalog::digest()
    ));
    assert_eq!(state_of(&disabled.telemetry), State::Off);
}

#[test]
fn the_table_refuses_what_it_does_not_know_and_a_value_that_makes_no_sense() {
    for text in [
        "[host.telemetry]\nunknown = 1\n",
        "[host.telemetry]\nrecord = [\"traces\"]\n",
        "[host.telemetry.exporters.Local]\nprovider = \"file\"\n",
        "[host.telemetry.exporters.local]\n",
        "[host.telemetry.exporters.local]\nprovider = \"file\"\nqueue = 0\n",
        "[host.telemetry.exporters.local]\nprovider = \"file\"\nbatch_seconds = 0\n",
        "[host.telemetry.exporters.local]\nprovider = \"file\"\nsample = 1.5\n",
        "[host.telemetry.exporters.local]\nprovider = \"file\"\nother = 1\n",
    ] {
        assert!(Config::from_str_in(text, Path::new("")).is_err(), "{text}");
    }
    let held = config(
        "[host.telemetry]\nrecord = [\"logs\"]\nstop_wait_seconds = 2\n[host.telemetry.exporters.a-b_1]\nprovider = \"file\"\nsettings = { keep_days = 3 }\nqueue = 10\nbatch_seconds = 1\nsample = 0.25\n",
    )
    .telemetry;
    assert_eq!(held.record, ["logs"]);
    assert_eq!(held.stop_wait_seconds, 2);
    let exporter = &held.exporters["a-b_1"];
    assert_eq!(
        (exporter.queue, exporter.batch_seconds, exporter.sample),
        (10, 1, 250_000)
    );
}

#[test]
fn nothing_is_installed_queued_or_watched_while_it_is_off_or_paused() {
    let home = scratch("off");
    let off = Telemetry::start(&config(""), Some(&home));
    assert!(off.hub().is_none());
    assert!(off.watch("claude").is_none());
    // Turned on and not accepted, and accepted for another list: the same.
    for acknowledged in ["", "ffffffffffffffff"] {
        let folder = home.join("out");
        let paused = config(&format!(
            "[host.telemetry]\nenabled = true\nacknowledged = \"{acknowledged}\"\n[host.telemetry.exporters.local]\nprovider = \"file\"\nsettings = {{ folder = {:?} }}\n",
            folder.display().to_string()
        ));
        let held = Telemetry::start(&paused, Some(&home));
        assert!(held.hub().is_none());
        assert!(held.watch("claude").is_none());
        let status = super::status_answer(&held, &paused);
        assert_eq!(status["state"], "paused", "{status}");
        assert!(status["paused"].is_string());
        assert!(!folder.exists(), "nothing was written");
    }
    assert_eq!(super::status_answer(&off, &config(""))["state"], "off");
}

#[test]
fn the_file_exporter_writes_what_the_hub_hands_it_once_the_list_is_accepted() {
    let home = scratch("on");
    let folder = home.join("out");
    let accepted = config(&format!(
        "[host.telemetry]\nenabled = true\nacknowledged = \"{}\"\n[host.telemetry.exporters.local]\nprovider = \"file\"\nsettings = {{ folder = {:?} }}\n",
        catalog::digest(),
        folder.display().to_string()
    ));
    let telemetry = Telemetry::start(&accepted, Some(&home));
    let hub = telemetry.hub().expect("running");
    hub.emit(Signal::HostTest {
        at: super::hub::now_ns(),
    });
    hub.emit(Signal::Turns {
        harness: Text::new("claude"),
        stop: signal::StopReason::EndTurn,
        at: super::hub::now_ns(),
    });
    assert_eq!(
        super::status_answer(&telemetry, &accepted)["state"],
        "running"
    );
    telemetry.stop();
    assert!(telemetry.hub().is_none());
    let mut lines = Vec::new();
    for entry in std::fs::read_dir(&folder).expect("the folder") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(name.ends_with(".jsonl"), "{name}");
        for line in std::fs::read_to_string(entry.path()).expect("read").lines() {
            lines.push((
                name.clone(),
                serde_json::from_str::<Value>(line).expect("a line is JSON"),
            ));
        }
    }
    let logs: Vec<&Value> = lines
        .iter()
        .filter(|(name, _)| name.starts_with("logs-"))
        .map(|(_, line)| line)
        .collect();
    let events: Vec<&str> = logs
        .iter()
        .flat_map(|line| {
            line["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|record| record["eventName"].as_str())
        .collect();
    assert!(
        events.contains(&"host.started") && events.contains(&"host.test"),
        "{events:?}"
    );
    assert!(events.contains(&"host.stopped"), "{events:?}");
    let metrics = lines
        .iter()
        .find(|(name, _)| name.starts_with("metrics-"))
        .expect("a metrics file");
    assert_eq!(
        metrics.1["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"],
        "canoa.turns"
    );
    let resource = logs[0]["resourceLogs"][0]["resource"]["attributes"].to_string();
    for key in catalog::RESOURCE {
        assert!(resource.contains(key), "{key} in {resource}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_default_folder_is_inside_the_hosts_home_and_a_folder_is_private_to_this_user() {
    let home = scratch("home");
    let accepted = config(&format!(
        "[host.telemetry]\nenabled = true\nacknowledged = \"{}\"\n[host.telemetry.exporters.local]\nprovider = \"file\"\n",
        catalog::digest()
    ));
    Telemetry::start(&accepted, Some(&home)).stop();
    let folder = home.join("telemetry");
    assert!(folder.is_dir(), "the exporter made telemetry/ in the home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&folder)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn old_files_of_the_exporter_are_removed_and_nothing_else_is() {
    let home = scratch("keep");
    for name in [
        "spans-2000-01-01.jsonl",
        "logs-2000-01-01.jsonl",
        "metrics-2000-01-01.jsonl",
        "notes-2000-01-01.jsonl",
        "readme.txt",
    ] {
        std::fs::write(home.join(name), "x").expect("written");
    }
    let exporter = super::file::FileExporter::in_folder(&home, 14);
    let mut batch = Batch::new(Vec::new());
    batch
        .logs
        .push(canoa::telemetry::LogRecord::new(1, "host.test", Vec::new()));
    exporter.export(batch).expect("written");
    let left: BTreeSet<String> = std::fs::read_dir(&home)
        .expect("a folder")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        left.contains("notes-2000-01-01.jsonl") && left.contains("readme.txt"),
        "{left:?}"
    );
    assert!(
        !left
            .iter()
            .any(|name| name.ends_with("2000-01-01.jsonl") && name != "notes-2000-01-01.jsonl"),
        "{left:?}"
    );
    assert_eq!(
        left.len(),
        3,
        "the two that stay and today's logs: {left:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// ---- the hub ----

/// A stand-in exporter: what it was handed, a failure to give, and a stall to hold.
#[derive(Default)]
struct Standin {
    batches: Mutex<Vec<Batch>>,
    failures: Mutex<VecDeque<ExporterError>>,
    calls: AtomicUsize,
    entered: AtomicBool,
    flushed: AtomicBool,
    stall: Option<Arc<(Mutex<bool>, Condvar)>>,
}

impl TelemetryExporter for Standin {
    fn describe(&self) -> Result<ExporterInfo, ProviderError> {
        Ok(ExporterInfo::new("a stand-in"))
    }

    fn export(&self, batch: Batch) -> Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(stall) = &self.stall {
            self.entered.store(true, Ordering::SeqCst);
            let (released, wake) = &**stall;
            let mut held = released.lock().unwrap_or_else(PoisonError::into_inner);
            while !*held {
                held = wake.wait(held).unwrap_or_else(PoisonError::into_inner);
            }
        }
        if let Some(failure) = self
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            return Err(failure.into_provider());
        }
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(batch);
        Ok(())
    }

    fn flush(&self) -> Result<(), ProviderError> {
        self.flushed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn lane(queue: usize, batch: Duration, sample: f64) -> LaneOptions {
    LaneOptions {
        name: "local".to_owned(),
        queue,
        batch,
        sample,
        first_pause: Duration::from_millis(20),
    }
}

fn all() -> Vec<Signals> {
    vec![Signals::Spans, Signals::Metrics, Signals::Logs]
}

fn hub_of(standin: &Arc<Standin>, options: LaneOptions, record: Vec<Signals>) -> Arc<Hub> {
    let exporter: Arc<dyn TelemetryExporter> = Arc::clone(standin) as Arc<dyn TelemetryExporter>;
    Hub::new(vec![(options, exporter, all())], record, Vec::new())
}

fn a_log() -> Signal {
    Signal::HostTest { at: 1 }
}

fn a_span(session: u64) -> Signal {
    Signal::Session {
        ctx: signal::Ctx {
            trace: 1,
            span: 2,
            parent: None,
            session,
        },
        harness: Text::new("claude"),
        end: signal::SessionEnd::Closed,
        start_ns: 1,
        end_ns: 2,
    }
}

fn delivered(standin: &Standin) -> Vec<Batch> {
    standin
        .batches
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[test]
fn a_full_queue_lets_go_of_its_oldest_and_counts_it_and_the_count_is_exported() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(3, Duration::from_secs(600), 1.0), all());
    for at in 1..=5 {
        hub.emit(Signal::HostRestarted {
            reason: signal::RestartReason::Updated,
            at,
        });
    }
    assert_eq!(hub.queued(), 3);
    let kept: Vec<i64> = hub
        .waiting("local")
        .into_iter()
        .filter_map(|record| match record {
            Record::Log(log) => Some(log.time_ns),
            _ => None,
        })
        .collect();
    assert_eq!(kept, [3, 4, 5], "the oldest two went");
    assert_eq!(hub.lanes()[0]["dropped"], 2);
    hub.stop(Duration::from_secs(10));
    let batches = delivered(&standin);
    let metrics: Vec<_> = batches.iter().flat_map(|batch| &batch.metrics).collect();
    let dropped = metrics
        .iter()
        .find(|metric| metric.name == "canoa.telemetry.dropped")
        .expect("the count is a metric");
    assert!((dropped.points[0].value - 2.0).abs() < f64::EPSILON);
    assert_eq!(
        dropped.points[0].attrs[0].text.as_deref(),
        Some("local"),
        "by exporter"
    );
    assert!(
        standin.flushed.load(Ordering::SeqCst),
        "asked to flush at the stop"
    );
}

#[test]
fn an_exporter_that_stalls_holds_its_own_thread_and_never_the_one_that_emits() {
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let standin = Arc::new(Standin {
        stall: Some(Arc::clone(&release)),
        ..Standin::default()
    });
    let hub = hub_of(
        &standin,
        lane(100_000, Duration::from_millis(5), 1.0),
        all(),
    );
    hub.emit(a_log());
    until("the exporter was called", || {
        standin.entered.load(Ordering::SeqCst)
    });
    // The control: the exporter really is stuck inside `export`, and has not returned.
    assert!(delivered(&standin).is_empty());
    let began = Instant::now();
    for session in 0..20_000 {
        hub.emit(a_span(session));
        hub.emit(Signal::Turns {
            harness: Text::new("claude"),
            stop: signal::StopReason::EndTurn,
            at: 1,
        });
    }
    let took = began.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "40000 emits took {took:?} behind a stalled exporter"
    );
    assert!(delivered(&standin).is_empty(), "still stuck");
    *release.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
    release.1.notify_all();
    hub.stop(Duration::from_secs(20));
    assert!(
        !delivered(&standin).is_empty(),
        "what waited was delivered once it was free"
    );
}

#[test]
fn a_batch_the_exporter_could_not_take_is_kept_and_tried_again_until_it_is_taken() {
    let standin = Arc::new(Standin::default());
    standin.failures.lock().expect("unlocked").extend([
        ExporterError::Unavailable("down".to_owned()),
        ExporterError::Auth("401".to_owned()),
    ]);
    let hub = hub_of(&standin, lane(10, Duration::from_millis(5), 1.0), all());
    hub.emit(a_log());
    until("the batch is delivered", || !delivered(&standin).is_empty());
    assert_eq!(
        standin.calls.load(Ordering::SeqCst),
        3,
        "two failures and the try that worked"
    );
    assert_eq!(delivered(&standin)[0].logs[0].name, "host.test");
    assert_eq!(hub.lanes()[0]["state"], "ok");
    hub.stop(Duration::from_secs(5));
    // One event says the streak, and it too was delivered.
    let events: Vec<String> = delivered(&standin)
        .iter()
        .flat_map(|batch| batch.logs.iter().map(|log| log.name.clone()))
        .collect();
    assert!(
        events.iter().any(|name| name == "exporter.failed"),
        "{events:?}"
    );
}

#[test]
fn a_batch_the_exporter_refuses_for_good_is_dropped_counted_and_not_tried_again() {
    let standin = Arc::new(Standin::default());
    standin
        .failures
        .lock()
        .expect("unlocked")
        .extend((0..50).map(|_| ExporterError::Rejected("400".to_owned())));
    let hub = hub_of(&standin, lane(10, Duration::from_millis(5), 1.0), all());
    hub.emit(a_log());
    until("it was asked", || standin.calls.load(Ordering::SeqCst) >= 1);
    until("it is marked", || hub.lanes()[0]["state"] == "rejected");
    std::thread::sleep(Duration::from_millis(300));
    let row = hub.lanes()[0].clone();
    assert_eq!(row["state"], "rejected", "{row}");
    assert_eq!(row["last_error"], "400", "{row}");
    assert!(
        row["dropped"].as_u64().is_some_and(|dropped| dropped >= 1),
        "{row}"
    );
    // The batch was asked once, and the one event that says so, once: a batch refused is not asked again, and a refusal begets no chain.
    assert!(
        standin.calls.load(Ordering::SeqCst) <= 2,
        "{} calls",
        standin.calls.load(Ordering::SeqCst)
    );
    assert!(delivered(&standin).is_empty());
    hub.stop(Duration::from_secs(5));
}

#[test]
fn an_exporter_is_handed_only_the_groups_it_takes_and_the_person_records() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(
        &standin,
        lane(10, Duration::from_secs(600), 1.0),
        vec![Signals::Logs],
    );
    hub.emit(a_span(1));
    hub.emit(Signal::Turns {
        harness: Text::new("claude"),
        stop: signal::StopReason::EndTurn,
        at: 1,
    });
    assert_eq!(hub.queued(), 0, "spans and metrics are not recorded");
    hub.emit(a_log());
    assert_eq!(hub.queued(), 1);
    hub.stop(Duration::from_secs(5));
    let exporter: Arc<dyn TelemetryExporter> = Arc::new(Standin::default());
    let spans_only = Hub::new(
        vec![(
            lane(10, Duration::from_secs(600), 1.0),
            exporter,
            vec![Signals::Spans],
        )],
        all(),
        Vec::new(),
    );
    spans_only.emit(a_log());
    assert_eq!(
        spans_only.queued(),
        0,
        "an exporter that takes spans is not handed events"
    );
    spans_only.emit(a_span(1));
    assert_eq!(spans_only.queued(), 1);
    spans_only.stop(Duration::from_secs(5));
}

#[test]
fn a_session_is_sampled_whole_by_its_key_and_only_spans_are_sampled() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(100, Duration::from_secs(600), 0.0), all());
    hub.emit(a_span(5));
    hub.emit(a_log());
    hub.emit(Signal::Turns {
        harness: Text::new("claude"),
        stop: signal::StopReason::EndTurn,
        at: 1,
    });
    assert_eq!(hub.queued(), 2, "the event and the metric, not the span");
    hub.stop(Duration::from_secs(5));
    let kept = (0..10_000_u64)
        .map(|index| index.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .filter(|key| super::hub::sampled_for_test(*key, 0.25))
        .count();
    assert!(
        (2_000..3_000).contains(&kept),
        "{kept} of 10000 at a quarter"
    );
    assert!(super::hub::sampled_for_test(7, 1.0) && !super::hub::sampled_for_test(7, 0.0));
    assert_eq!(
        super::hub::sampled_for_test(123_456_789, 0.5),
        super::hub::sampled_for_test(123_456_789, 0.5)
    );
}

// ---- what is watched ----

#[test]
fn a_notice_an_update_failure_and_a_start_after_an_update_are_exported_by_name_and_result() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(1000, Duration::from_secs(600), 1.0), all());
    let telemetry = Telemetry::over(Arc::clone(&hub));
    telemetry.notice_told("permission", "desktop", false);
    telemetry.notice_told("finished", "phone", true);
    telemetry.update_failed();
    telemetry.host_updated();
    hub.stop(Duration::from_secs(10));
    let text = format!("{:?}", delivered(&standin));
    for wanted in [
        "canoa.notifications",
        "permission",
        "phone",
        "failed",
        "update.failed",
        "host.restarted",
        "updated",
    ] {
        assert!(text.contains(wanted), "{wanted} in {text}");
    }
    // A host that exports nothing tells nothing and does not fail.
    let off = Telemetry::off(None);
    off.notice_told("permission", "desktop", false);
    off.update_failed();
    off.host_updated();
}

fn leaks(batches: &[Batch], canaries: &[&str]) -> Vec<String> {
    let text = format!("{batches:?}");
    canaries
        .iter()
        .filter(|canary| text.contains(**canary))
        .map(|canary| (*canary).to_owned())
        .collect()
}

#[test]
fn a_session_through_the_watch_exports_its_shape_and_none_of_its_words() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(1000, Duration::from_secs(600), 1.0), all());
    {
        let watch = Watch::with_hub(Arc::clone(&hub), "claude");
        let client = |message: Value| watch.heard_client(message.to_string().as_bytes());
        let agent = |message: Value| watch.heard_agent(message.to_string().as_bytes());
        client(
            json!({"jsonrpc": "2.0", "id": 1, "method": "session/new", "params": {"cwd": "/home/CANARY_FOLDER/project", "mcpServers": []}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"sessionId": "s-CANARY_SESSION", "models": {"currentModelId": "model/x"}}}),
        );
        client(
            json!({"jsonrpc": "2.0", "id": 2, "method": "session/prompt", "params": {"sessionId": "s-CANARY_SESSION", "prompt": [{"type": "text", "text": "CANARY_PROMPT sk-CANARY_SECRET"}]}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-CANARY_SESSION", "update": {
            "sessionUpdate": "tool_call", "toolCallId": "c-CANARY_CALL", "title": "Edit CANARY_PATH/calc.js", "kind": "edit", "status": "pending",
            "rawInput": {"file_path": "CANARY_PATH/calc.js", "command": "CANARY_COMMAND"}, "locations": [{"path": "CANARY_PATH/calc.js"}],
            "_meta": {"claudeCode": {"toolName": "Edit"}}}}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "id": 100, "method": "session/request_permission", "params": {"sessionId": "s-CANARY_SESSION",
            "toolCall": {"toolCallId": "c-CANARY_CALL", "title": "Edit CANARY_PATH/calc.js", "kind": "edit"},
            "options": [{"optionId": "yes", "name": "Allow CANARY_OPTION", "kind": "allow_once"}, {"optionId": "no", "name": "Reject", "kind": "reject_once"}]}}),
        );
        client(
            json!({"jsonrpc": "2.0", "id": 100, "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-CANARY_SESSION", "update": {
            "sessionUpdate": "tool_call_update", "toolCallId": "c-CANARY_CALL", "status": "completed",
            "content": [{"type": "content", "content": {"type": "text", "text": "CANARY_OUTPUT"}}]}}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s-CANARY_SESSION", "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "CANARY_REPLY"}}}}),
        );
        agent(
            json!({"jsonrpc": "2.0", "id": 2, "result": {"stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "cachedReadTokens": 3}}}),
        );
        client(
            json!({"jsonrpc": "2.0", "id": 3, "method": "session/close", "params": {"sessionId": "s-CANARY_SESSION"}}),
        );
        agent(json!({"jsonrpc": "2.0", "id": 3, "result": {}}));
    }
    hub.stop(Duration::from_secs(10));
    let batches = delivered(&standin);
    let canaries = [
        "CANARY",
        "calc.js",
        "/home/",
        "project",
        "claudeCode",
        "s-CANARY",
        "sk-",
    ];
    assert_eq!(leaks(&batches, &canaries), Vec::<String>::new());
    // The control: the search does find a canary that was put there.
    let mut leaky = batches.clone();
    leaky[0].logs.push(canoa::telemetry::LogRecord::new(
        1,
        "x",
        vec![canoa::telemetry::Attr::text("canoa.x", "the CANARY_PROMPT")],
    ));
    assert_eq!(leaks(&leaky, &["CANARY_PROMPT"]), ["CANARY_PROMPT"]);
    let spans: Vec<&canoa::telemetry::Span> =
        batches.iter().flat_map(|batch| &batch.spans).collect();
    let named = |name: &str| {
        spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("{name} in {spans:?}"))
    };
    let attr = |span: &canoa::telemetry::Span, key: &str| {
        span.attrs.iter().find(|attr| attr.key == key).cloned()
    };
    let turn = named("canoa.turn");
    assert_eq!(
        attr(turn, "canoa.stop_reason")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("end_turn")
    );
    assert_eq!(
        attr(turn, "canoa.model")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("model/x")
    );
    assert_eq!(
        attr(turn, "canoa.tokens.input").and_then(|attr| attr.int),
        Some(10)
    );
    assert_eq!(
        attr(turn, "canoa.tokens.output").and_then(|attr| attr.int),
        Some(5)
    );
    assert_eq!(
        attr(turn, "canoa.tokens.cache_read").and_then(|attr| attr.int),
        Some(3)
    );
    assert!(
        attr(turn, "canoa.tokens.cache_write").is_none(),
        "absent when the agent did not report it"
    );
    assert!(attr(turn, "canoa.cost.micro_usd").is_none());
    let call = named("canoa.tool_call");
    assert_eq!(
        attr(call, "canoa.tool.name")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("Edit")
    );
    assert_eq!(
        attr(call, "canoa.tool.kind")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("edit")
    );
    assert_eq!(
        attr(call, "canoa.tool.status")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("completed")
    );
    let permission = named("canoa.permission");
    assert_eq!(
        attr(permission, "canoa.permission.outcome")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("allow")
    );
    let session = named("canoa.session");
    assert_eq!(
        attr(session, "canoa.session.end")
            .and_then(|attr| attr.text)
            .as_deref(),
        Some("closed")
    );
    assert_eq!(turn.trace_id, session.trace_id, "one trace a session");
    assert_eq!(turn.parent_id.as_deref(), Some(session.span_id.as_str()));
    let names: BTreeSet<&str> = batches
        .iter()
        .flat_map(|batch| batch.metrics.iter().map(|metric| metric.name.as_str()))
        .collect();
    for metric in [
        "canoa.turns",
        "canoa.turn.duration",
        "canoa.tokens",
        "canoa.tool_calls",
        "canoa.permissions",
        "canoa.sessions.active",
    ] {
        assert!(names.contains(metric), "{metric} in {names:?}");
    }
}

#[test]
fn a_session_left_open_when_the_connection_ends_is_closed_and_a_refused_one_is_a_failure() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(1000, Duration::from_secs(600), 1.0), all());
    {
        let watch = Watch::with_hub(Arc::clone(&hub), "claude");
        watch.heard_client(
            br#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/x"}}"#,
        );
        watch.heard_agent(br#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"a"}}"#);
        watch.heard_client(
            br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/x"}}"#,
        );
        watch.heard_agent(br#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"CANARY_ERROR at /home/x"}}"#);
    }
    hub.stop(Duration::from_secs(10));
    let batches = delivered(&standin);
    assert!(leaks(&batches, &["CANARY_ERROR", "/home/"]).is_empty());
    let ended = batches
        .iter()
        .flat_map(|batch| &batch.spans)
        .find(|span| span.name == "canoa.session")
        .expect("a session span");
    assert!(
        ended
            .attrs
            .iter()
            .any(|attr| attr.key == "canoa.session.end" && attr.text.as_deref() == Some("closed"))
    );
    let failed = batches
        .iter()
        .flat_map(|batch| &batch.logs)
        .find(|log| log.name == "session.failed")
        .expect("an event");
    assert_eq!(failed.attrs[0].text.as_deref(), Some("other"));
}

#[test]
fn a_tool_name_an_agent_gives_is_cut_to_the_safe_set() {
    let standin = Arc::new(Standin::default());
    let hub = hub_of(&standin, lane(1000, Duration::from_secs(600), 1.0), all());
    {
        let watch = Watch::with_hub(Arc::clone(&hub), "x");
        watch.heard_client(br#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{}}"#);
        watch.heard_agent(br#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"a"}}"#);
        watch.heard_agent(
            "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"a\",\"update\":{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"t\",\"kind\":\"other\",\"status\":\"failed\",\"_meta\":{\"v\":{\"toolName\":\"my tool\\nwith words\"}}}}}"
                .as_bytes(),
        );
    }
    hub.stop(Duration::from_secs(10));
    let batches = delivered(&standin);
    let call = batches
        .iter()
        .flat_map(|batch| &batch.spans)
        .find(|span| span.name == "canoa.tool_call")
        .expect("a call");
    let name = call
        .attrs
        .iter()
        .find(|attr| attr.key == "canoa.tool.name")
        .and_then(|attr| attr.text.clone());
    assert_eq!(name.as_deref(), Some("my_tool_with_words"));
}
