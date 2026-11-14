//! Which of the events ACP does not show a harness's own hooks give Canoa, and what each harness cannot give.

use coqui_aos::HookEvent;

/// How an event fires for a harness: `Ok` is what runs it, `Err` why nothing can.
pub(super) type Support = Result<&'static str, &'static str>;

/// The harness whose hooks Canoa writes: Claude Code, which runs the hooks of a plugin the session is given.
pub(super) fn is_claude(harness: Option<&str>) -> bool {
    harness == Some("claude-code")
}

/// How `event` fires for `harness` when it is one only a harness's own hooks show, for a hook that matches `select`;
/// `None` for an event Canoa sees over ACP itself.
pub(super) fn support(
    harness: Option<&str>,
    event: HookEvent,
    select: Option<&[String]>,
) -> Option<Support> {
    let claude = is_claude(harness);
    let codex = harness == Some("codex");
    match event {
        HookEvent::Cd => Some(if claude {
            Ok(
                "run by Canoa when Claude Code reports a new working directory after a batch of tool calls (a plugin's hook: the event of its own is not given to plugins)",
            )
        } else if codex {
            Err("Codex has no event for a change of working directory")
        } else {
            Err("Canoa reaches a working directory change only through Claude Code's hooks")
        }),
        HookEvent::PreCompact => Some(if claude {
            Ok(
                "run by Canoa before Claude Code compacts, for the person's /compact (a plugin's PreCompact hook): block stops it, context is not applied",
            )
        } else if codex {
            Err(
                "the Codex adapter has no way to compact over ACP, and Codex's own hook for it has no trigger there",
            )
        } else {
            Err("Canoa reaches compaction only through Claude Code's hooks")
        }),
        HookEvent::SessionStart => {
            let select = select?;
            if select
                .iter()
                .any(|reason| matches!(reason.as_str(), "new" | "resume"))
            {
                return None;
            }
            if select.iter().any(|reason| reason == "compact") {
                return Some(if claude {
                    Ok(
                        "run by Canoa when Claude Code starts a session again after compacting (a plugin's SessionStart hook): context goes in the next prompt",
                    )
                } else if codex {
                    Err("the Codex adapter has no way to compact over ACP")
                } else {
                    Err("Canoa reaches a compacted session only through Claude Code's hooks")
                });
            }
            if select.iter().any(|reason| reason == "clear") {
                return Some(Err(if claude {
                    "over ACP the person's /clear ends Claude Code's session and the adapter's, so no session starts after it"
                } else {
                    "no harness Canoa reaches starts a session after a /clear over ACP"
                }));
            }
            None
        }
        _ => None,
    }
}
