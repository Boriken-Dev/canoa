//! What the host can emit: one variant for each signal of the catalog, carrying exactly the attributes the catalog lists. There is no
//! way to emit text from a session, an agent or an error: a text attribute is a [`Text`], cut to 64 characters of a safe set, and
//! every other attribute is a number or one of a closed set of words.

use canoa::telemetry::{Attr, LogRecord, MetricKind, Point, Signals, Span};

use super::catalog;

/// The most characters a text attribute keeps.
pub(crate) const MAX_TEXT: usize = 64;

/// A text attribute: at most [`MAX_TEXT`] characters of `A-Z a-z 0-9 . _ : / -`, anything else replaced by `_`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Text(String);

impl Text {
    /// `raw` cut and cleaned; `unknown` when it holds nothing.
    pub(crate) fn new(raw: &str) -> Self {
        let cut: String = raw
            .chars()
            .take(MAX_TEXT)
            .map(|each| {
                if each.is_ascii_alphanumeric() || matches!(each, '.' | '_' | ':' | '/' | '-') {
                    each
                } else {
                    '_'
                }
            })
            .collect();
        Self(if cut.is_empty() {
            "unknown".to_owned()
        } else {
            cut
        })
    }

    /// `raw` as a text attribute, or `None` when it is empty.
    pub(crate) fn maybe(raw: &str) -> Option<Self> {
        (!raw.trim().is_empty()).then(|| Self::new(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! words {
    ($(#[$meta:meta])* $name:ident, $set:path { $($variant:ident => $word:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum $name { $($variant),+ }

        impl $name {
            /// Every value, in the catalog's order.
            #[allow(dead_code, reason = "the tests compare it with the catalog")]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The word the catalog lists for it.
            pub(crate) const fn word(self) -> &'static str {
                match self { $(Self::$variant => $word),+ }
            }

            /// The value the word names.
            #[allow(dead_code, reason = "only some of the sets are read back from a word")]
            pub(crate) fn parse(word: &str) -> Option<Self> {
                match word { $($word => Some(Self::$variant),)+ _ => None }
            }
        }

        #[cfg(test)]
        impl $name {
            pub(crate) fn listed() -> &'static [&'static str] { $set }
        }
    };
}

words!(
    /// How a turn ended.
    StopReason, catalog::STOP_REASONS {
        EndTurn => "end_turn", MaxTokens => "max_tokens", MaxTurnRequests => "max_turn_requests",
        Cancelled => "cancelled", Refusal => "refusal", Error => "error",
    }
);
words!(
    /// How a session ended.
    SessionEnd, catalog::SESSION_ENDS { Closed => "closed", Failed => "failed", HostStopped => "host_stopped" }
);
words!(
    /// What a tool does, as ACP names it.
    ToolKind, catalog::TOOL_KINDS {
        Read => "read", Edit => "edit", Delete => "delete", Move => "move", Search => "search", Execute => "execute",
        Think => "think", Fetch => "fetch", SwitchMode => "switch_mode", Other => "other",
    }
);
words!(
    /// How a tool call ended.
    ToolStatus, catalog::TOOL_STATUSES { Completed => "completed", Failed => "failed" }
);
words!(
    /// What was decided on a request for permission.
    Outcome, catalog::OUTCOMES { Allow => "allow", Reject => "reject", Cancelled => "cancelled" }
);
words!(
    /// Who decided.
    DecidedBy, catalog::DECIDED_BY {
        Hook => "hook", Policy => "policy", Person => "person", Approver => "approver", Timeout => "timeout",
    }
);
words!(
    /// A kind of token.
    TokenType, catalog::TOKEN_TYPES {
        Input => "input", Output => "output", CacheRead => "cache_read", CacheWrite => "cache_write",
    }
);
words!(
    /// An allowance of an agent.
    Bucket, catalog::BUCKETS { FiveHour => "five_hour", SevenDay => "seven_day", RecordedToday => "recorded_today" }
);
words!(
    /// Whether a notice was sent.
    NoticeResult, catalog::RESULTS { Sent => "sent", Failed => "failed" }
);
words!(
    /// Why the host restarted.
    RestartReason, catalog::RESTART_REASONS { Requested => "requested", Updated => "updated", Unclean => "unclean" }
);
words!(
    /// Why a session or an update failed.
    FailReason, catalog::FAIL_REASONS {
        Spawn => "spawn", AgentExited => "agent_exited", Auth => "auth", Network => "network", Timeout => "timeout",
        Refused => "refused", Other => "other",
    }
);
words!(
    /// What a spending limit covers.
    Scope, catalog::SCOPES { Account => "account", Profile => "profile", All => "all" }
);
words!(
    /// Why an exporter could not deliver.
    ExporterReason, catalog::EXPORTER_REASONS {
        Unavailable => "unavailable", Auth => "auth", Bad => "bad", Rejected => "rejected",
    }
);

/// Where a span sits: its trace and ids, and the session whose sampling decides whether it is exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ctx {
    pub(crate) trace: u128,
    pub(crate) span: u64,
    pub(crate) parent: Option<u64>,
    /// The session's random key; an exporter's `sample` is decided on it, so a session is exported whole or not at all.
    pub(crate) session: u64,
}

/// The tokens a turn reported; one the agent did not report is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnTokens {
    pub(crate) input: Option<u64>,
    pub(crate) output: Option<u64>,
    pub(crate) cache_read: Option<u64>,
    pub(crate) cache_write: Option<u64>,
}

/// One signal. `*_ns` and `at` are Unix nanoseconds.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    dead_code,
    reason = "the usage fraction, the notifications, the limits, the updates and the restart are emitted by the modules that reach them"
)]
pub(crate) enum Signal {
    Session {
        ctx: Ctx,
        harness: Text,
        end: SessionEnd,
        start_ns: i64,
        end_ns: i64,
    },
    Turn {
        ctx: Ctx,
        stop: StopReason,
        model: Option<Text>,
        tokens: TurnTokens,
        cost_micro_usd: Option<u64>,
        start_ns: i64,
        end_ns: i64,
    },
    ToolCall {
        ctx: Ctx,
        kind: ToolKind,
        name: Option<Text>,
        status: ToolStatus,
        start_ns: i64,
        end_ns: i64,
    },
    Permission {
        ctx: Ctx,
        outcome: Outcome,
        by: DecidedBy,
        kind: ToolKind,
        start_ns: i64,
        end_ns: i64,
    },
    Turns {
        harness: Text,
        stop: StopReason,
        at: i64,
    },
    TurnDuration {
        harness: Text,
        seconds: f64,
        at: i64,
    },
    Tokens {
        harness: Text,
        model: Text,
        token: TokenType,
        count: u64,
        at: i64,
    },
    Cost {
        harness: Text,
        model: Text,
        micro_usd: u64,
        at: i64,
    },
    ToolCalls {
        kind: ToolKind,
        status: ToolStatus,
        at: i64,
    },
    Permissions {
        outcome: Outcome,
        by: DecidedBy,
        at: i64,
    },
    SessionsActive {
        harness: Text,
        count: u64,
        at: i64,
    },
    UsageFraction {
        harness: Text,
        bucket: Bucket,
        permille: u32,
        at: i64,
    },
    Notifications {
        event: Text,
        notifier: Text,
        result: NoticeResult,
        at: i64,
    },
    Dropped {
        exporter: Text,
        count: u64,
        at: i64,
    },
    HostStarted {
        version: Text,
        at: i64,
    },
    HostStopped {
        at: i64,
    },
    HostRestarted {
        reason: RestartReason,
        at: i64,
    },
    SessionFailed {
        reason: FailReason,
        at: i64,
    },
    LimitWarn {
        scope: Scope,
        permille: u32,
        at: i64,
    },
    LimitStop {
        scope: Scope,
        permille: u32,
        at: i64,
    },
    UpdateFailed {
        reason: FailReason,
        at: i64,
    },
    ExporterFailed {
        reason: ExporterReason,
        at: i64,
    },
    HostTest {
        at: i64,
    },
}

/// What a signal becomes in a batch.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Record {
    Span(Span),
    Metric {
        name: &'static str,
        unit: &'static str,
        kind: MetricKind,
        point: Point,
    },
    Log(LogRecord),
}

fn text(key: &str, value: &Text) -> Attr {
    Attr::text(key, value.as_str())
}

fn word(key: &str, value: &'static str) -> Attr {
    Attr::text(key, value)
}

fn count(key: &str, value: u64) -> Attr {
    Attr::int(key, i64::try_from(value).unwrap_or(i64::MAX))
}

fn hex_trace(trace: u128) -> String {
    format!("{trace:032x}")
}

fn hex_span(span: u64) -> String {
    format!("{span:016x}")
}

impl Signal {
    /// Its name in the catalog.
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Session { .. } => "canoa.session",
            Self::Turn { .. } => "canoa.turn",
            Self::ToolCall { .. } => "canoa.tool_call",
            Self::Permission { .. } => "canoa.permission",
            Self::Turns { .. } => "canoa.turns",
            Self::TurnDuration { .. } => "canoa.turn.duration",
            Self::Tokens { .. } => "canoa.tokens",
            Self::Cost { .. } => "canoa.cost",
            Self::ToolCalls { .. } => "canoa.tool_calls",
            Self::Permissions { .. } => "canoa.permissions",
            Self::SessionsActive { .. } => "canoa.sessions.active",
            Self::UsageFraction { .. } => "canoa.usage.fraction",
            Self::Notifications { .. } => "canoa.notifications",
            Self::Dropped { .. } => "canoa.telemetry.dropped",
            Self::HostStarted { .. } => "host.started",
            Self::HostStopped { .. } => "host.stopped",
            Self::HostRestarted { .. } => "host.restarted",
            Self::SessionFailed { .. } => "session.failed",
            Self::LimitWarn { .. } => "limit.warn",
            Self::LimitStop { .. } => "limit.stop",
            Self::UpdateFailed { .. } => "update.failed",
            Self::ExporterFailed { .. } => "exporter.failed",
            Self::HostTest { .. } => "host.test",
        }
    }

    /// Which group of the person's `record` it belongs to.
    pub(crate) fn group(&self) -> Signals {
        catalog::find(self.name()).map_or(Signals::Logs, |def| def.group)
    }

    /// The session key a span's sampling is decided on; none for a metric or an event.
    pub(crate) const fn sample_key(&self) -> Option<u64> {
        match self {
            Self::Session { ctx, .. }
            | Self::Turn { ctx, .. }
            | Self::ToolCall { ctx, .. }
            | Self::Permission { ctx, .. } => Some(ctx.session),
            _ => None,
        }
    }

    /// Its attributes, in the catalog's order, an absent optional one left out.
    pub(crate) fn attrs(&self) -> Vec<Attr> {
        match self {
            Self::Session {
                ctx, harness, end, ..
            } => vec![
                text("canoa.harness", harness),
                Attr::text("canoa.session.ref", hex_span(ctx.session)),
                word("canoa.session.end", end.word()),
            ],
            Self::Turn {
                stop,
                model,
                tokens,
                cost_micro_usd,
                ..
            } => {
                let mut list = vec![word("canoa.stop_reason", stop.word())];
                if let Some(model) = model {
                    list.push(text("canoa.model", model));
                }
                for (key, value) in [
                    ("canoa.tokens.input", tokens.input),
                    ("canoa.tokens.output", tokens.output),
                    ("canoa.tokens.cache_read", tokens.cache_read),
                    ("canoa.tokens.cache_write", tokens.cache_write),
                    ("canoa.cost.micro_usd", *cost_micro_usd),
                ] {
                    if let Some(value) = value {
                        list.push(count(key, value));
                    }
                }
                list
            }
            Self::ToolCall {
                kind, name, status, ..
            } => {
                let mut list = vec![word("canoa.tool.kind", kind.word())];
                if let Some(name) = name {
                    list.push(text("canoa.tool.name", name));
                }
                list.push(word("canoa.tool.status", status.word()));
                list
            }
            Self::Permission {
                outcome, by, kind, ..
            } => vec![
                word("canoa.permission.outcome", outcome.word()),
                word("canoa.permission.by", by.word()),
                word("canoa.tool.kind", kind.word()),
            ],
            Self::Turns { harness, stop, .. } => vec![
                text("canoa.harness", harness),
                word("canoa.stop_reason", stop.word()),
            ],
            Self::TurnDuration { harness, .. } | Self::SessionsActive { harness, .. } => {
                vec![text("canoa.harness", harness)]
            }
            Self::Tokens {
                harness,
                model,
                token,
                ..
            } => vec![
                text("canoa.harness", harness),
                text("canoa.model", model),
                word("canoa.token.type", token.word()),
            ],
            Self::Cost { harness, model, .. } => {
                vec![text("canoa.harness", harness), text("canoa.model", model)]
            }
            Self::ToolCalls { kind, status, .. } => vec![
                word("canoa.tool.kind", kind.word()),
                word("canoa.tool.status", status.word()),
            ],
            Self::Permissions { outcome, by, .. } => vec![
                word("canoa.permission.outcome", outcome.word()),
                word("canoa.permission.by", by.word()),
            ],
            Self::UsageFraction {
                harness, bucket, ..
            } => vec![
                text("canoa.harness", harness),
                word("canoa.bucket", bucket.word()),
            ],
            Self::Notifications {
                event,
                notifier,
                result,
                ..
            } => vec![
                text("canoa.event", event),
                text("canoa.notifier", notifier),
                word("canoa.result", result.word()),
            ],
            Self::Dropped { exporter, .. } => vec![text("canoa.exporter", exporter)],
            Self::HostStarted { version, .. } => vec![text("canoa.version", version)],
            Self::HostStopped { .. } | Self::HostTest { .. } => Vec::new(),
            Self::HostRestarted { reason, .. } => vec![word("canoa.reason", reason.word())],
            Self::SessionFailed { reason, .. } | Self::UpdateFailed { reason, .. } => {
                vec![word("canoa.reason", reason.word())]
            }
            Self::LimitWarn {
                scope, permille, ..
            }
            | Self::LimitStop {
                scope, permille, ..
            } => vec![
                word("canoa.scope", scope.word()),
                count("canoa.permille", u64::from(*permille)),
            ],
            Self::ExporterFailed { reason, .. } => vec![word("canoa.reason", reason.word())],
        }
    }

    /// The record it becomes.
    pub(crate) fn into_record(self) -> Record {
        let attrs = self.attrs();
        let name = self.name();
        let unit = catalog::find(name).map_or("", |def| def.unit);
        let metric = |kind, at, value: f64| Record::Metric {
            name,
            unit,
            kind,
            point: Point::new(at, value, attrs.clone()),
        };
        // Counts are small enough for a float to hold exactly.
        #[allow(
            clippy::cast_precision_loss,
            reason = "a count of signals, tokens or micro-dollars"
        )]
        let number = |count: u64| count as f64;
        match self {
            Self::Session {
                ctx,
                start_ns,
                end_ns,
                ..
            }
            | Self::Turn {
                ctx,
                start_ns,
                end_ns,
                ..
            }
            | Self::ToolCall {
                ctx,
                start_ns,
                end_ns,
                ..
            }
            | Self::Permission {
                ctx,
                start_ns,
                end_ns,
                ..
            } => Record::Span(Span::new(
                hex_trace(ctx.trace),
                hex_span(ctx.span),
                ctx.parent.map(hex_span),
                name,
                start_ns,
                end_ns,
                attrs,
            )),
            Self::Turns { at, .. } => metric(MetricKind::Counter, at, 1.0),
            Self::TurnDuration { seconds, at, .. } => metric(MetricKind::Histogram, at, seconds),
            Self::Tokens { count, at, .. } => metric(MetricKind::Counter, at, number(count)),
            Self::Cost { micro_usd, at, .. } => metric(MetricKind::Counter, at, number(micro_usd)),
            Self::ToolCalls { at, .. }
            | Self::Permissions { at, .. }
            | Self::Notifications { at, .. } => metric(MetricKind::Counter, at, 1.0),
            Self::SessionsActive { count, at, .. } => metric(MetricKind::Gauge, at, number(count)),
            Self::UsageFraction { permille, at, .. } => {
                metric(MetricKind::Gauge, at, f64::from(permille))
            }
            Self::Dropped { count, at, .. } => metric(MetricKind::Counter, at, number(count)),
            Self::HostStarted { at, .. }
            | Self::HostStopped { at }
            | Self::HostRestarted { at, .. }
            | Self::SessionFailed { at, .. }
            | Self::LimitWarn { at, .. }
            | Self::LimitStop { at, .. }
            | Self::UpdateFailed { at, .. }
            | Self::ExporterFailed { at, .. }
            | Self::HostTest { at } => Record::Log(LogRecord::new(at, name, attrs)),
        }
    }
}

/// One of every signal, every optional attribute present, for the tests that compare the variants with the catalog.
#[cfg(test)]
pub(crate) fn samples() -> Vec<Signal> {
    let ctx = Ctx {
        trace: 7,
        span: 8,
        parent: Some(9),
        session: 10,
    };
    let harness = Text::new("claude");
    let model = Text::new("model-1");
    vec![
        Signal::Session {
            ctx,
            harness: harness.clone(),
            end: SessionEnd::Closed,
            start_ns: 1,
            end_ns: 2,
        },
        Signal::Turn {
            ctx,
            stop: StopReason::EndTurn,
            model: Some(model.clone()),
            tokens: TurnTokens {
                input: Some(1),
                output: Some(2),
                cache_read: Some(3),
                cache_write: Some(4),
            },
            cost_micro_usd: Some(5),
            start_ns: 1,
            end_ns: 2,
        },
        Signal::ToolCall {
            ctx,
            kind: ToolKind::Execute,
            name: Some(Text::new("Bash")),
            status: ToolStatus::Completed,
            start_ns: 1,
            end_ns: 2,
        },
        Signal::Permission {
            ctx,
            outcome: Outcome::Allow,
            by: DecidedBy::Person,
            kind: ToolKind::Edit,
            start_ns: 1,
            end_ns: 2,
        },
        Signal::Turns {
            harness: harness.clone(),
            stop: StopReason::EndTurn,
            at: 1,
        },
        Signal::TurnDuration {
            harness: harness.clone(),
            seconds: 1.5,
            at: 1,
        },
        Signal::Tokens {
            harness: harness.clone(),
            model: model.clone(),
            token: TokenType::Input,
            count: 3,
            at: 1,
        },
        Signal::Cost {
            harness: harness.clone(),
            model,
            micro_usd: 3,
            at: 1,
        },
        Signal::ToolCalls {
            kind: ToolKind::Read,
            status: ToolStatus::Failed,
            at: 1,
        },
        Signal::Permissions {
            outcome: Outcome::Reject,
            by: DecidedBy::Policy,
            at: 1,
        },
        Signal::SessionsActive {
            harness: harness.clone(),
            count: 2,
            at: 1,
        },
        Signal::UsageFraction {
            harness,
            bucket: Bucket::FiveHour,
            permille: 500,
            at: 1,
        },
        Signal::Notifications {
            event: Text::new("turn_finished"),
            notifier: Text::new("mail"),
            result: NoticeResult::Sent,
            at: 1,
        },
        Signal::Dropped {
            exporter: Text::new("local"),
            count: 4,
            at: 1,
        },
        Signal::HostStarted {
            version: Text::new("0.1.0"),
            at: 1,
        },
        Signal::HostStopped { at: 1 },
        Signal::HostRestarted {
            reason: RestartReason::Updated,
            at: 1,
        },
        Signal::SessionFailed {
            reason: FailReason::Spawn,
            at: 1,
        },
        Signal::LimitWarn {
            scope: Scope::Account,
            permille: 800,
            at: 1,
        },
        Signal::LimitStop {
            scope: Scope::All,
            permille: 1000,
            at: 1,
        },
        Signal::UpdateFailed {
            reason: FailReason::Network,
            at: 1,
        },
        Signal::ExporterFailed {
            reason: ExporterReason::Unavailable,
            at: 1,
        },
        Signal::HostTest { at: 1 },
    ]
}
