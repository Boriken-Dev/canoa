//! The list of what the host records, and what it never does: the only one. Each signal has a name, a group, a unit, a sentence and
//! its attributes (a name, a type and either the closed set of values or "text cut to 64 characters of `[A-Za-z0-9._:/-]`").
//!
//! The list is served by `telemetry/catalog` and drawn on the Settings page. Its digest is what the person acknowledges: a new
//! signal or attribute changes it, which stops export until the person has read the list again.

use canoa::telemetry::Signals;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub(crate) const STOP_REASONS: &[&str] = &[
    "end_turn",
    "max_tokens",
    "max_turn_requests",
    "cancelled",
    "refusal",
    "error",
];
pub(crate) const SESSION_ENDS: &[&str] = &["closed", "failed", "host_stopped"];
pub(crate) const TOOL_KINDS: &[&str] = &[
    "read",
    "edit",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "switch_mode",
    "other",
];
pub(crate) const TOOL_STATUSES: &[&str] = &["completed", "failed"];
pub(crate) const OUTCOMES: &[&str] = &["allow", "reject", "cancelled"];
pub(crate) const DECIDED_BY: &[&str] = &["hook", "policy", "person", "approver", "timeout"];
pub(crate) const TOKEN_TYPES: &[&str] = &["input", "output", "cache_read", "cache_write"];
pub(crate) const BUCKETS: &[&str] = &["five_hour", "seven_day", "recorded_today"];
pub(crate) const RESULTS: &[&str] = &["sent", "failed"];
pub(crate) const RESTART_REASONS: &[&str] = &["requested", "updated", "unclean"];
pub(crate) const FAIL_REASONS: &[&str] = &[
    "spawn",
    "agent_exited",
    "auth",
    "network",
    "timeout",
    "refused",
    "other",
];
pub(crate) const SCOPES: &[&str] = &["account", "profile", "all"];
pub(crate) const EXPORTER_REASONS: &[&str] = &["unavailable", "auth", "bad", "rejected"];

/// What a signal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Span,
    Counter,
    Histogram,
    Gauge,
    Event,
}

impl Kind {
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Span => "span",
            Self::Counter => "counter",
            Self::Histogram => "histogram",
            Self::Gauge => "gauge",
            Self::Event => "event",
        }
    }
}

/// What an attribute holds.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Ty {
    /// Text cut to 64 characters of `[A-Za-z0-9._:/-]`, anything else in it replaced by `_`.
    Text,
    /// A whole number.
    Int,
    /// One of a closed set of words.
    Choice(&'static [&'static str]),
}

/// One attribute of a signal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AttrDef {
    pub(crate) name: &'static str,
    pub(crate) ty: Ty,
    /// Left out when the agent did not report it.
    pub(crate) optional: bool,
}

const fn attr(name: &'static str, ty: Ty) -> AttrDef {
    AttrDef {
        name,
        ty,
        optional: false,
    }
}

const fn maybe(name: &'static str, ty: Ty) -> AttrDef {
    AttrDef {
        name,
        ty,
        optional: true,
    }
}

/// One signal the host can record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Def {
    pub(crate) name: &'static str,
    pub(crate) group: Signals,
    pub(crate) kind: Kind,
    /// The OpenTelemetry unit; empty for a span and an event.
    pub(crate) unit: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) attrs: &'static [AttrDef],
}

const HARNESS: AttrDef = attr("canoa.harness", Ty::Text);
const MODEL: AttrDef = attr("canoa.model", Ty::Text);
const STOP: AttrDef = attr("canoa.stop_reason", Ty::Choice(STOP_REASONS));
const TOOL_KIND: AttrDef = attr("canoa.tool.kind", Ty::Choice(TOOL_KINDS));
const TOOL_STATUS: AttrDef = attr("canoa.tool.status", Ty::Choice(TOOL_STATUSES));
const OUTCOME: AttrDef = attr("canoa.permission.outcome", Ty::Choice(OUTCOMES));
const BY: AttrDef = attr("canoa.permission.by", Ty::Choice(DECIDED_BY));

/// Every signal, in the order the page lists them.
pub(crate) const CATALOG: &[Def] = &[
    Def {
        name: "canoa.session",
        group: Signals::Spans,
        kind: Kind::Span,
        unit: "",
        doc: "A session, from its start to its end.",
        attrs: &[
            HARNESS,
            attr("canoa.session.ref", Ty::Text),
            attr("canoa.session.end", Ty::Choice(SESSION_ENDS)),
        ],
    },
    Def {
        name: "canoa.turn",
        group: Signals::Spans,
        kind: Kind::Span,
        unit: "",
        doc: "One turn of a session: how it ended, the model, and the tokens and cost the agent reported.",
        attrs: &[
            STOP,
            maybe("canoa.model", Ty::Text),
            maybe("canoa.tokens.input", Ty::Int),
            maybe("canoa.tokens.output", Ty::Int),
            maybe("canoa.tokens.cache_read", Ty::Int),
            maybe("canoa.tokens.cache_write", Ty::Int),
            maybe("canoa.cost.micro_usd", Ty::Int),
        ],
    },
    Def {
        name: "canoa.tool_call",
        group: Signals::Spans,
        kind: Kind::Span,
        unit: "",
        doc: "A tool call: its kind, the tool's name (never its input or output) and how it ended.",
        attrs: &[TOOL_KIND, maybe("canoa.tool.name", Ty::Text), TOOL_STATUS],
    },
    Def {
        name: "canoa.permission",
        group: Signals::Spans,
        kind: Kind::Span,
        unit: "",
        doc: "A request for permission: the kind of tool it was for, the outcome and who decided.",
        attrs: &[OUTCOME, BY, TOOL_KIND],
    },
    Def {
        name: "canoa.turns",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{turn}",
        doc: "Turns, by agent and how they ended.",
        attrs: &[HARNESS, STOP],
    },
    Def {
        name: "canoa.turn.duration",
        group: Signals::Metrics,
        kind: Kind::Histogram,
        unit: "s",
        doc: "How long turns took, by agent.",
        attrs: &[HARNESS],
    },
    Def {
        name: "canoa.tokens",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{token}",
        doc: "Tokens the agents reported, by agent, model and kind of token.",
        attrs: &[
            HARNESS,
            MODEL,
            attr("canoa.token.type", Ty::Choice(TOKEN_TYPES)),
        ],
    },
    Def {
        name: "canoa.cost",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{micro_usd}",
        doc: "What the agents reported spending, in millionths of a US dollar, by agent and model.",
        attrs: &[HARNESS, MODEL],
    },
    Def {
        name: "canoa.tool_calls",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{call}",
        doc: "Tool calls, by kind and how they ended.",
        attrs: &[TOOL_KIND, TOOL_STATUS],
    },
    Def {
        name: "canoa.permissions",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{request}",
        doc: "Requests for permission, by outcome and who decided.",
        attrs: &[OUTCOME, BY],
    },
    Def {
        name: "canoa.sessions.active",
        group: Signals::Metrics,
        kind: Kind::Gauge,
        unit: "{session}",
        doc: "Sessions open now, by agent.",
        attrs: &[HARNESS],
    },
    Def {
        name: "canoa.usage.fraction",
        group: Signals::Metrics,
        kind: Kind::Gauge,
        unit: "{permille}",
        doc: "How much of an allowance an agent has used, in thousandths, by agent and allowance.",
        attrs: &[HARNESS, attr("canoa.bucket", Ty::Choice(BUCKETS))],
    },
    Def {
        name: "canoa.notifications",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{notice}",
        doc: "Notices the host sent, by event, notifier and result.",
        attrs: &[
            attr("canoa.event", Ty::Text),
            attr("canoa.notifier", Ty::Text),
            attr("canoa.result", Ty::Choice(RESULTS)),
        ],
    },
    Def {
        name: "canoa.telemetry.dropped",
        group: Signals::Metrics,
        kind: Kind::Counter,
        unit: "{signal}",
        doc: "Signals an exporter's queue let go of because it was full, by exporter.",
        attrs: &[attr("canoa.exporter", Ty::Text)],
    },
    Def {
        name: "host.started",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "The host started, and its version.",
        attrs: &[attr("canoa.version", Ty::Text)],
    },
    Def {
        name: "host.stopped",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "The host stopped.",
        attrs: &[],
    },
    Def {
        name: "host.restarted",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "The host restarted, and why.",
        attrs: &[attr("canoa.reason", Ty::Choice(RESTART_REASONS))],
    },
    Def {
        name: "session.failed",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "A session could not start or ended badly, and the kind of reason.",
        attrs: &[attr("canoa.reason", Ty::Choice(FAIL_REASONS))],
    },
    Def {
        name: "limit.warn",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "A spending limit's warning line was reached.",
        attrs: &[
            attr("canoa.scope", Ty::Choice(SCOPES)),
            attr("canoa.permille", Ty::Int),
        ],
    },
    Def {
        name: "limit.stop",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "A spending limit stopped new turns.",
        attrs: &[
            attr("canoa.scope", Ty::Choice(SCOPES)),
            attr("canoa.permille", Ty::Int),
        ],
    },
    Def {
        name: "update.failed",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "An update could not be made, and the kind of reason.",
        attrs: &[attr("canoa.reason", Ty::Choice(FAIL_REASONS))],
    },
    Def {
        name: "exporter.failed",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "An exporter could not deliver a batch, and the kind of reason.",
        attrs: &[attr("canoa.reason", Ty::Choice(EXPORTER_REASONS))],
    },
    Def {
        name: "host.test",
        group: Signals::Logs,
        kind: Kind::Event,
        unit: "",
        doc: "The one record the Test button of an exporter sends.",
        attrs: &[],
    },
];

/// What the host never records, in the words the page shows.
pub(crate) const NEVER: &[&str] = &[
    "The words of the person or of the agent.",
    "File names, paths and contents.",
    "Commands, and the input and output of tools.",
    "The working folder.",
    "Session titles.",
    "Account names and labels, e-mail addresses and plan names.",
    "Keys, tokens, passwords and any other secret.",
    "The host's name, the user name and addresses.",
    "Environment variables.",
    "Error messages.",
];

/// What every signal says of where it came from.
pub(crate) const RESOURCE: &[&str] = &[
    "service.name",
    "service.version",
    "service.instance.id",
    "os.type",
    "host.arch",
];

fn ty_text(ty: Ty) -> String {
    match ty {
        Ty::Text => "text".to_owned(),
        Ty::Int => "int".to_owned(),
        Ty::Choice(values) => format!("one of {}", values.join(",")),
    }
}

/// The catalog as one text, which the digest is taken of: any change to a signal, an attribute, a value or a sentence of what is never
/// recorded changes it.
fn canonical() -> String {
    let mut text = String::new();
    for def in CATALOG {
        text.push_str(&format!(
            "{}|{}|{}|{}|{}\n",
            def.name,
            def.group.word(),
            def.kind.word(),
            def.unit,
            def.doc
        ));
        for each in def.attrs {
            text.push_str(&format!(
                "  {}|{}|{}\n",
                each.name,
                ty_text(each.ty),
                each.optional
            ));
        }
    }
    for line in NEVER {
        text.push_str(&format!("never|{line}\n"));
    }
    for name in RESOURCE {
        text.push_str(&format!("resource|{name}\n"));
    }
    text
}

/// The first 16 hexadecimal digits of the SHA-256 of `text`.
fn digest_of(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    hash.iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The digest of the catalog: the first 16 hexadecimal digits of its SHA-256.
pub(crate) fn digest() -> String {
    digest_of(&canonical())
}

/// What the digest would be were `change` made to the catalog's text, for the tests that show a change moves it.
#[cfg(test)]
pub(crate) fn digest_after(change: impl FnOnce(&mut String)) -> String {
    let mut text = canonical();
    change(&mut text);
    digest_of(&text)
}

/// The definition of the signal `name`.
pub(crate) fn find(name: &str) -> Option<&'static Def> {
    CATALOG.iter().find(|def| def.name == name)
}

/// `telemetry/catalog`'s answer: the digest, every signal and what is never recorded.
pub(crate) fn json() -> Value {
    let signals: Vec<Value> = CATALOG
        .iter()
        .map(|def| {
            let attrs: Vec<Value> = def
                .attrs
                .iter()
                .map(|each| match each.ty {
                    Ty::Text => json!({"name": each.name, "type": "text", "optional": each.optional}),
                    Ty::Int => json!({"name": each.name, "type": "int", "optional": each.optional}),
                    Ty::Choice(values) => json!({
                        "name": each.name, "type": "choice", "values": values, "optional": each.optional,
                    }),
                })
                .collect();
            json!({
                "name": def.name,
                "group": def.group.word(),
                "kind": def.kind.word(),
                "unit": def.unit,
                "description": def.doc,
                "attributes": attrs,
            })
        })
        .collect();
    json!({
        "digest": digest(),
        "signals": signals,
        "never": NEVER,
        "resource": RESOURCE,
        "text_rule": "Text from an agent is cut to 64 characters of A-Z, a-z, 0-9 and . _ : / -, anything else in it replaced by _.",
    })
}
