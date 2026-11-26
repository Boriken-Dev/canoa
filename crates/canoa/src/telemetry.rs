//! The `telemetry_exporter` kind: somewhere the host's traces, metrics and events go, when the person has turned that on.
//!
//! The host decides what is recorded (a closed list it shows the person, with no free text from a session) and owns batching,
//! the queue, retries and the stop; an exporter only delivers a batch. A provider describes itself ([`ExporterInfo`]), takes a
//! [`Batch`] in [`TelemetryExporter::export`] and is asked to finish what it holds in [`TelemetryExporter::flush`] when the host
//! stops. Its settings are those of its instance, as for the other kinds that take settings.
//!
//! The models are plain data in the shape of OpenTelemetry's, the same in every language: a time is Unix nanoseconds, an id is
//! lower-case hexadecimal, an attribute holds one of text, an integer, a number or a flag. [`otlp`] writes a batch as OTLP's JSON
//! encoding, which the built-in `file` provider and an HTTP exporter share.
//!
//! A provider that cannot deliver says why with [`ExporterError`]: `Unavailable` and `Auth` make the host keep the batch and try
//! again later, `Bad` and `Rejected` make it drop the batch and count it.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue};

/// The name of the `telemetry_exporter` kind.
pub const KIND: &str = "telemetry_exporter";

/// The versions of the kind's description.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// Which of the three groups of signals an exporter takes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Signals {
    /// Traces: one span per session, turn, tool call and permission.
    Spans,
    /// Counters, gauges and histograms.
    Metrics,
    /// Events with a fixed name and fixed attributes.
    Logs,
}

impl Signals {
    /// Its name in `record` and in the files an exporter writes.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Spans => "spans",
            Self::Metrics => "metrics",
            Self::Logs => "logs",
        }
    }
}

/// What an exporter is, from [`TelemetryExporter::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ExporterInfo {
    /// Its name for people.
    pub title: String,
    /// A line saying where it sends what it takes.
    pub help: Option<String>,
    /// The groups of signals it takes; it is never handed the others.
    #[map(default)]
    pub signals: Vec<Signals>,
}

impl ExporterInfo {
    /// An exporter that takes every group.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            help: None,
            signals: alloc::vec![Signals::Spans, Signals::Metrics, Signals::Logs],
        }
    }
}

/// One attribute: its name and exactly one of the four values.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Attr {
    /// The attribute's name, such as `canoa.harness`.
    pub key: String,
    /// Its value, when it is text.
    pub text: Option<String>,
    /// Its value, when it is a whole number.
    pub int: Option<i64>,
    /// Its value, when it is any other number.
    pub number: Option<f64>,
    /// Its value, when it is a flag.
    pub flag: Option<bool>,
}

impl Attr {
    fn blank(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            text: None,
            int: None,
            number: None,
            flag: None,
        }
    }

    /// A text attribute.
    #[must_use]
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            text: Some(value.into()),
            ..Self::blank(key)
        }
    }

    /// A whole-number attribute.
    #[must_use]
    pub fn int(key: impl Into<String>, value: i64) -> Self {
        Self {
            int: Some(value),
            ..Self::blank(key)
        }
    }

    /// A number attribute.
    #[must_use]
    pub fn number(key: impl Into<String>, value: f64) -> Self {
        Self {
            number: Some(value),
            ..Self::blank(key)
        }
    }

    /// A flag attribute.
    #[must_use]
    pub fn flag(key: impl Into<String>, value: bool) -> Self {
        Self {
            flag: Some(value),
            ..Self::blank(key)
        }
    }
}

/// One finished span.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Span {
    /// The trace it belongs to: 32 lower-case hexadecimal digits, random per session and per run.
    pub trace_id: String,
    /// Its own id: 16 lower-case hexadecimal digits.
    pub span_id: String,
    /// The span it is part of, when it is part of one.
    pub parent_id: Option<String>,
    /// Its name, such as `canoa.turn`.
    pub name: String,
    /// When it began, in Unix nanoseconds.
    pub start_ns: i64,
    /// When it ended, in Unix nanoseconds.
    pub end_ns: i64,
    /// What it says.
    #[map(default)]
    pub attrs: Vec<Attr>,
}

impl Span {
    /// A span of `name` from `start_ns` to `end_ns` in the trace `trace_id`.
    #[must_use]
    pub fn new(
        trace_id: impl Into<String>,
        span_id: impl Into<String>,
        parent_id: Option<String>,
        name: impl Into<String>,
        start_ns: i64,
        end_ns: i64,
        attrs: Vec<Attr>,
    ) -> Self {
        Self {
            trace_id: trace_id.into(),
            span_id: span_id.into(),
            parent_id,
            name: name.into(),
            start_ns,
            end_ns,
            attrs,
        }
    }
}

/// What a metric measures.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum MetricKind {
    /// A count: each point is what was added since the last, never negative.
    Counter,
    /// A level: each point is the value at its time.
    Gauge,
    /// A distribution: each point is one measurement.
    Histogram,
}

/// One reading of a metric.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Point {
    /// When it was taken, in Unix nanoseconds.
    pub time_ns: i64,
    /// Its value.
    pub value: f64,
    /// What it says.
    #[map(default)]
    pub attrs: Vec<Attr>,
}

impl Point {
    /// A reading of `value` at `time_ns`.
    #[must_use]
    pub fn new(time_ns: i64, value: f64, attrs: Vec<Attr>) -> Self {
        Self {
            time_ns,
            value,
            attrs,
        }
    }
}

/// A metric and its readings in one batch.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Metric {
    /// Its name, such as `canoa.turns`.
    pub name: String,
    /// Its unit, as OpenTelemetry writes one: `s`, `{token}`.
    pub unit: String,
    /// What it counts or measures.
    pub kind: MetricKind,
    /// Its readings, oldest first.
    #[map(default)]
    pub points: Vec<Point>,
}

impl Metric {
    /// A metric of `name` in `unit`, with its readings.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        unit: impl Into<String>,
        kind: MetricKind,
        points: Vec<Point>,
    ) -> Self {
        Self {
            name: name.into(),
            unit: unit.into(),
            kind,
            points,
        }
    }
}

/// One event.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct LogRecord {
    /// When it happened, in Unix nanoseconds.
    pub time_ns: i64,
    /// Its fixed name, such as `host.started`.
    pub name: String,
    /// What it says.
    #[map(default)]
    pub attrs: Vec<Attr>,
}

impl LogRecord {
    /// An event `name` at `time_ns`.
    #[must_use]
    pub fn new(time_ns: i64, name: impl Into<String>, attrs: Vec<Attr>) -> Self {
        Self {
            time_ns,
            name: name.into(),
            attrs,
        }
    }
}

/// What an exporter is handed at once: the groups it takes, from one host run.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Batch {
    /// What every signal in it came from: `service.name`, `service.version`, `service.instance.id`, `os.type`, `host.arch`.
    #[map(default)]
    pub resource: Vec<Attr>,
    /// The spans, when the exporter takes them.
    #[map(default)]
    pub spans: Vec<Span>,
    /// The metrics, when it takes them.
    #[map(default)]
    pub metrics: Vec<Metric>,
    /// The events, when it takes them.
    #[map(default)]
    pub logs: Vec<LogRecord>,
}

impl Batch {
    /// An empty batch about `resource`.
    #[must_use]
    pub fn new(resource: Vec<Attr>) -> Self {
        Self {
            resource,
            ..Self::default()
        }
    }

    /// Whether it holds no signal.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty() && self.metrics.is_empty() && self.logs.is_empty()
    }
}

/// Somewhere signals are delivered. One instance is one configuration of one provider (a folder, an address and its headers).
///
/// `describe` answers the same on every call with nothing run. `export` returns when the batch is delivered or fails with an
/// [`ExporterError`]; the host calls it from one thread at a time and keeps a batch the exporter could not take until it can.
/// `flush` is called once when the host stops, with the host's wait for it: an exporter that holds nothing answers at once.
#[coqui_acp_api::guatiao::kind(name = "telemetry_exporter", crate = ::coqui_acp_api::guatiao)]
pub trait TelemetryExporter: Send + Sync {
    /// What this exporter is.
    fn describe(&self) -> Result<ExporterInfo, ProviderError>;

    /// Delivers a batch.
    fn export(&self, batch: Batch) -> Result<(), ProviderError>;

    /// Finishes what the exporter holds, when the host stops.
    fn flush(&self) -> Result<(), ProviderError>;
}

/// Why a batch was not delivered. `why` is one line with no secret and never the destination's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExporterError {
    /// The destination cannot be reached now: the host keeps the batch and tries again.
    Unavailable(String),
    /// The destination refused the credentials: the host keeps the batch and tries again, so a person who fixes them is heard.
    Auth(String),
    /// The settings or the batch are wrong in a way another try will not change: the batch is dropped and counted.
    Bad(String),
    /// The destination refused the batch for good: it is dropped and counted.
    Rejected(String),
}

impl ExporterError {
    /// The error a provider answers with.
    #[must_use]
    pub fn into_provider(self) -> ProviderError {
        match self {
            Self::Unavailable(why) => ProviderError::new(Status::GUATIAO_ERR_GONE, &why),
            Self::Auth(why) => {
                ProviderError::new(Status::GUATIAO_ERR_GONE, &alloc::format!("auth: {why}"))
            }
            Self::Bad(why) | Self::Rejected(why) => {
                ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why)
            }
        }
    }
}

/// Whether the host keeps a batch that failed with `error` and tries it again: everything but a refusal for good.
#[must_use]
pub fn is_retried(error: &ProviderError) -> bool {
    error.status != Status::GUATIAO_ERR_BAD_VALUE
}

/// Batches as OTLP's JSON encoding, one document per group.
pub mod otlp {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use serde_json::{Value, json};

    use super::{Attr, Batch, Metric, MetricKind};

    /// The bounds, in seconds, of the buckets a histogram of durations is written with.
    pub const DURATION_BOUNDS: [f64; 10] =
        [0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0];

    /// A batch as the bodies of `/v1/traces`, `/v1/metrics` and `/v1/logs`; a group with nothing in it is `None`.
    #[derive(Debug, Clone, PartialEq)]
    #[non_exhaustive]
    pub struct Documents {
        /// The body for traces.
        pub traces: Option<Value>,
        /// The body for metrics.
        pub metrics: Option<Value>,
        /// The body for logs.
        pub logs: Option<Value>,
    }

    fn attr(each: &Attr) -> Value {
        let value = if let Some(text) = &each.text {
            json!({"stringValue": text})
        } else if let Some(int) = each.int {
            json!({"intValue": int.to_string()})
        } else if let Some(number) = each.number {
            json!({"doubleValue": number})
        } else if let Some(flag) = each.flag {
            json!({"boolValue": flag})
        } else {
            json!({})
        };
        json!({"key": each.key, "value": value})
    }

    fn attrs(list: &[Attr]) -> Vec<Value> {
        list.iter().map(attr).collect()
    }

    fn nanos(time: i64) -> String {
        time.max(0).to_string()
    }

    fn metric(each: &Metric) -> Value {
        let points = |each_point: &super::Point| {
            json!({
                "timeUnixNano": nanos(each_point.time_ns),
                "asDouble": each_point.value,
                "attributes": attrs(&each_point.attrs),
            })
        };
        let body = match each.kind {
            MetricKind::Counter => json!({"sum": {
                "aggregationTemporality": 1,
                "isMonotonic": true,
                "dataPoints": each.points.iter().map(points).collect::<Vec<_>>(),
            }}),
            MetricKind::Gauge => json!({"gauge": {
                "dataPoints": each.points.iter().map(points).collect::<Vec<_>>(),
            }}),
            MetricKind::Histogram => json!({"histogram": {
                "aggregationTemporality": 1,
                "dataPoints": each.points.iter().map(|point| {
                    let mut counts = alloc::vec![0_u64; DURATION_BOUNDS.len() + 1];
                    let slot = DURATION_BOUNDS
                        .iter()
                        .position(|bound| point.value <= *bound)
                        .unwrap_or(DURATION_BOUNDS.len());
                    counts[slot] += 1;
                    json!({
                        "timeUnixNano": nanos(point.time_ns),
                        "count": "1",
                        "sum": point.value,
                        "bucketCounts": counts.iter().map(ToString::to_string).collect::<Vec<_>>(),
                        "explicitBounds": DURATION_BOUNDS,
                        "attributes": attrs(&point.attrs),
                    })
                }).collect::<Vec<_>>(),
            }}),
        };
        let mut object = json!({"name": each.name, "unit": each.unit});
        if let (Some(target), Value::Object(extra)) = (object.as_object_mut(), body) {
            target.extend(extra);
        }
        object
    }

    /// The documents of `batch`.
    #[must_use]
    pub fn render(batch: &Batch) -> Documents {
        let resource = json!({"attributes": attrs(&batch.resource)});
        let scope = json!({"name": "canoa"});
        Documents {
            traces: (!batch.spans.is_empty()).then(|| {
                json!({"resourceSpans": [{
                    "resource": resource,
                    "scopeSpans": [{"scope": scope, "spans": batch.spans.iter().map(|span| {
                        let mut object = json!({
                            "traceId": span.trace_id,
                            "spanId": span.span_id,
                            "name": span.name,
                            "kind": 1,
                            "startTimeUnixNano": nanos(span.start_ns),
                            "endTimeUnixNano": nanos(span.end_ns),
                            "attributes": attrs(&span.attrs),
                        });
                        if let (Some(parent), Some(target)) = (&span.parent_id, object.as_object_mut()) {
                            target.insert("parentSpanId".to_owned(), json!(parent));
                        }
                        object
                    }).collect::<Vec<_>>()}],
                }]})
            }),
            metrics: (!batch.metrics.is_empty()).then(|| {
                json!({"resourceMetrics": [{
                    "resource": resource,
                    "scopeMetrics": [{"scope": scope, "metrics": batch.metrics.iter().map(metric).collect::<Vec<_>>()}],
                }]})
            }),
            logs: (!batch.logs.is_empty()).then(|| {
                json!({"resourceLogs": [{
                    "resource": resource,
                    "scopeLogs": [{"scope": scope, "logRecords": batch.logs.iter().map(|record| json!({
                        "timeUnixNano": nanos(record.time_ns),
                        "severityNumber": 9,
                        "severityText": "INFO",
                        "body": {"stringValue": record.name},
                        "eventName": record.name,
                        "attributes": attrs(&record.attrs),
                    })).collect::<Vec<_>>()}],
                }]})
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Batch {
        let mut batch = Batch::new(alloc::vec![Attr::text("service.name", "canoa")]);
        batch.spans.push(Span {
            trace_id: "0".repeat(32),
            span_id: "1".repeat(16),
            parent_id: Some("2".repeat(16)),
            name: "canoa.turn".into(),
            start_ns: 5,
            end_ns: 9,
            attrs: alloc::vec![Attr::int("canoa.tokens.input", 12)],
        });
        batch.metrics.push(Metric {
            name: "canoa.turn.duration".into(),
            unit: "s".into(),
            kind: MetricKind::Histogram,
            points: alloc::vec![Point {
                time_ns: 9,
                value: 0.7,
                attrs: alloc::vec![Attr::text("canoa.harness", "claude")],
            }],
        });
        batch.metrics.push(Metric {
            name: "canoa.turns".into(),
            unit: "{turn}".into(),
            kind: MetricKind::Counter,
            points: alloc::vec![Point {
                time_ns: 9,
                value: 1.0,
                attrs: Vec::new(),
            }],
        });
        batch.logs.push(LogRecord {
            time_ns: 3,
            name: "host.started".into(),
            attrs: alloc::vec![Attr::flag("x", true)],
        });
        batch
    }

    #[test]
    fn a_batch_is_written_as_otlp_json() {
        let documents = otlp::render(&sample());
        let traces = documents.traces.expect("spans");
        let span = &traces["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["name"], "canoa.turn");
        assert_eq!(span["startTimeUnixNano"], "5");
        assert_eq!(span["parentSpanId"], "2".repeat(16));
        assert_eq!(span["attributes"][0]["value"]["intValue"], "12");
        let metrics = documents.metrics.expect("metrics");
        let list = &metrics["resourceMetrics"][0]["scopeMetrics"][0]["metrics"];
        let histogram = &list[0]["histogram"]["dataPoints"][0];
        assert_eq!(histogram["count"], "1");
        assert_eq!(histogram["bucketCounts"][1], "1", "0.7 s is in (0.5, 1]");
        assert_eq!(list[1]["sum"]["isMonotonic"], true);
        let logs = documents.logs.expect("logs");
        assert_eq!(
            logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["eventName"],
            "host.started"
        );
    }

    #[test]
    fn a_group_with_nothing_in_it_has_no_document() {
        let documents = otlp::render(&Batch::new(Vec::new()));
        assert!(
            documents.traces.is_none() && documents.metrics.is_none() && documents.logs.is_none()
        );
    }

    #[test]
    fn only_a_refusal_for_good_is_not_retried() {
        for (error, retried) in [
            (ExporterError::Unavailable("down".into()), true),
            (ExporterError::Auth("401".into()), true),
            (ExporterError::Bad("settings".into()), false),
            (ExporterError::Rejected("400".into()), false),
        ] {
            assert_eq!(is_retried(&error.into_provider()), retried);
        }
    }
}
