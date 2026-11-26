//! The words of a notice: fixed phrases per event in English and Spanish, with the session's title and the agent's name where
//! the event is about a session. Nothing an agent sent but a title the session carries enters a notice: never a command, a path or
//! a tool input.

use canoa::notifier::SessionRef;

/// The longest title or name a notice carries, in characters.
const LONGEST: usize = 80;

/// What an occurrence is about, beyond its event.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    /// The session, for an event about one.
    pub(crate) session: Option<SessionRef>,
    /// A figure the phrase carries (a percent, a version).
    pub(crate) figure: Option<String>,
}

/// `text` as one line of at most [`LONGEST`] characters, or `None` when nothing is left.
pub(crate) fn short_line(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    let mut chars = line.chars();
    let cut: String = chars.by_ref().take(LONGEST).collect();
    Some(if chars.next().is_some() {
        format!("{}…", cut.trim_end())
    } else {
        cut
    })
}

fn spanish(language: &str) -> bool {
    language
        .split(['-', '_'])
        .next()
        .is_some_and(|primary| primary.eq_ignore_ascii_case("es"))
}

/// The title and the body of a notice for `event`, in `language` (English for any this host has no words for).
pub(crate) fn notice(event: &str, language: &str, facts: &Facts) -> (String, String) {
    let es = spanish(language);
    let agent = facts
        .session
        .as_ref()
        .and_then(|session| session.agent.as_deref())
        .and_then(short_line)
        .unwrap_or_else(|| if es { "El agente" } else { "The agent" }.to_owned());
    let figure = facts
        .figure
        .as_deref()
        .and_then(short_line)
        .unwrap_or_default();
    let body = match (event, es) {
        ("permission", false) => format!("{agent} asks leave to use a tool"),
        ("permission", true) => format!("{agent} pide permiso para usar una herramienta"),
        ("question", false) => format!("{agent} has a question for you"),
        ("question", true) => format!("{agent} tiene una pregunta para ti"),
        ("finished", false) => format!("{agent} finished its turn"),
        ("finished", true) => format!("{agent} terminó su turno"),
        ("failed", false) => format!("{agent} could not finish its turn"),
        ("failed", true) => format!("{agent} no pudo terminar su turno"),
        ("usage_warn", false) => format!("A usage limit is near {figure}")
            .trim_end()
            .to_owned(),
        ("usage_warn", true) => format!("Un límite de uso está cerca {figure}")
            .trim_end()
            .to_owned(),
        ("usage_stop", false) => "A usage limit was reached".to_owned(),
        ("usage_stop", true) => "Se alcanzó un límite de uso".to_owned(),
        ("update_failed", false) => "An update failed".to_owned(),
        ("update_failed", true) => "Una actualización falló".to_owned(),
        ("host_restarted", false) => "The host restarted".to_owned(),
        ("host_restarted", true) => "El servidor se reinició".to_owned(),
        ("host_updated", false) => format!("The host was updated {figure}")
            .trim_end()
            .to_owned(),
        ("host_updated", true) => format!("El servidor se actualizó {figure}")
            .trim_end()
            .to_owned(),
        (_, false) => "Something happened".to_owned(),
        (_, true) => "Algo ocurrió".to_owned(),
    };
    let title = facts.session.as_ref().map_or_else(
        || "Canoa".to_owned(),
        |session| {
            session
                .title
                .as_deref()
                .and_then(short_line)
                .unwrap_or_else(|| {
                    if es {
                        "Una conversación"
                    } else {
                        "A conversation"
                    }
                    .to_owned()
                })
        },
    );
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(title: Option<&str>) -> Facts {
        let mut reference = SessionRef::new("s1");
        reference.title = title.map(str::to_owned);
        reference.agent = Some("Claude".to_owned());
        Facts {
            session: Some(reference),
            figure: None,
        }
    }

    #[test]
    fn a_notice_names_the_session_and_the_agent_and_a_fixed_phrase() {
        let (title, body) = notice("permission", "en", &session(Some("Fix the build")));
        assert_eq!(title, "Fix the build");
        assert_eq!(body, "Claude asks leave to use a tool");
        let (title, body) = notice("finished", "es-PR", &session(None));
        assert_eq!(title, "Una conversación");
        assert_eq!(body, "Claude terminó su turno");
    }

    #[test]
    fn a_title_is_one_short_line_and_a_host_event_names_no_session() {
        let long = format!("a\n\n  b {}", "x".repeat(200));
        let (title, _) = notice("failed", "en", &session(Some(&long)));
        assert!(
            !title.contains('\n') && title.chars().count() <= LONGEST + 1,
            "{title}"
        );
        let facts = Facts {
            session: None,
            figure: Some("1.2".into()),
        };
        let (title, body) = notice("host_updated", "en", &facts);
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Canoa", "The host was updated 1.2")
        );
    }
}
