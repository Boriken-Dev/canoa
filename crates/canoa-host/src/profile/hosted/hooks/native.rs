//! Which of the events ACP does not show an agent's own hooks give Canoa, and what each agent cannot give. What an agent's hooks do
//! is the agent's own word, from its description provider; this holds only what the host says when the agent has none.

use canoa::vendor::{HookSupportRequest, HooksRequest};
use coqui_aos::HookEvent;

/// How an event fires for an agent: `Ok` is what runs it, `Err` why nothing can.
pub(super) type Support = Result<String, String>;

/// How long the host may take to answer a hook the agent runs, in seconds: what a plugin's hook is given to wait.
pub(super) const ANSWER_SECONDS: u64 = 130;

/// Whether the agent whose registry id is `harness` can be given hooks that report to the host. An agent that answers a request for
/// no commands has them to give.
pub(super) fn gives_hooks(harness: Option<&str>) -> bool {
    harness
        .and_then(|aos| {
            crate::vendors::behave_aos(aos, |vendor| {
                vendor.hooks_delivery(HooksRequest::new(Vec::new(), ANSWER_SECONDS))
            })
        })
        .is_some_and(|answer| matches!(answer, Ok(Some(_))))
}

/// What the agent whose registry id is `harness` says of how `event` fires, or `otherwise` when it says nothing.
fn ask(
    harness: Option<&str>,
    event: &str,
    reason: Option<&str>,
    otherwise: &'static str,
) -> Option<Support> {
    let said = harness
        .and_then(|aos| {
            crate::vendors::behave_aos(aos, |vendor| {
                vendor.hook_support(HookSupportRequest::new(event, reason.map(str::to_owned)))
            })
        })
        .and_then(Result::ok)
        .flatten();
    Some(match said {
        Some(said) if said.fires => Ok(said.text),
        Some(said) => Err(said.text),
        None => Err(otherwise.to_owned()),
    })
}

/// How `event` fires for `harness` when it is one only an agent's own hooks show, for a hook that matches `select`;
/// `None` for an event Canoa sees over ACP itself.
pub(super) fn support(
    harness: Option<&str>,
    event: HookEvent,
    select: Option<&[String]>,
) -> Option<Support> {
    match event {
        HookEvent::Cd => ask(
            harness,
            "cd",
            None,
            "no hook of this agent's own tells Canoa of a change of working directory",
        ),
        HookEvent::PreCompact => ask(
            harness,
            "pre-compact",
            None,
            "no hook of this agent's own tells Canoa of a compaction",
        ),
        HookEvent::SessionStart => {
            let select = select?;
            if select
                .iter()
                .any(|reason| matches!(reason.as_str(), "new" | "resume"))
            {
                return None;
            }
            if select.iter().any(|reason| reason == "compact") {
                return ask(
                    harness,
                    "session-start",
                    Some("compact"),
                    "no hook of this agent's own tells Canoa of a compacted session",
                );
            }
            if select.iter().any(|reason| reason == "clear") {
                return ask(
                    harness,
                    "session-start",
                    Some("clear"),
                    "no harness Canoa reaches starts a session after a /clear over ACP",
                );
            }
            None
        }
        _ => None,
    }
}
