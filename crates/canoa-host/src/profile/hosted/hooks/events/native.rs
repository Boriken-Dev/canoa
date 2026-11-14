//! The events a harness's own hooks show Claude Code's session: compaction and a changed working directory. The host
//! gives each session a plugin whose hooks run the forwarder (`canoa hook-forward`), reads what it reports between the
//! messages it relays, runs the overlays' hooks itself, and answers.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_aos::{Decision, HookEvent};
use coqui_transport::Channel;
use serde_json::{Map, Value, json};

use super::super::forward::{Forward, Forwarded, Kind, Verdict, write_plugin};
use super::super::native::is_claude;
use super::SessionHooks;
use crate::profile::hosted::hooks::report_item;
use crate::profile::hosted::{Hosted, Written};

/// A directory compared as the harness and the client may each have spelled it: separators and, on Windows, case.
fn plain(dir: &str) -> String {
    let mut plain = dir.replace('\\', "/");
    while plain.len() > 1 && plain.ends_with('/') {
        plain.pop();
    }
    if cfg!(windows) {
        plain.make_ascii_lowercase();
    }
    plain
}

/// The channel and plugin made for one session request.
#[derive(Debug)]
pub(super) struct Enrolled {
    pub(super) secret: String,
    pub(super) written: Written,
}

/// What came of giving a session a plugin of the harness's own hooks.
pub(super) enum Plugin {
    /// No overlay listens for what the harness's hooks show, or the harness is not Claude Code.
    Nothing,
    Added(Enrolled),
    /// One was needed and could not be made; `guarded` when an overlay's hook that needs it fails closed.
    Failed {
        why: String,
        guarded: bool,
    },
}

impl<C: Channel + ?Sized> Hosted<'_, C> {
    /// Gives a session a plugin of Claude Code hooks that report to this host, when an overlay listens for what only
    /// they show, and adds it to the session request's plugins. When one is needed and cannot be made the answer says why,
    /// and whether an overlay's guard needs it, which the caller must not let fail open.
    pub(super) fn native_plugin(
        &self,
        params: &mut Map<String, Value>,
        session: &SessionHooks,
    ) -> Plugin {
        if !is_claude(Some(&self.harness_label())) {
            return Plugin::Nothing;
        }
        let (kinds, guarded) = self.connection.hooks.native(&session.allowed);
        if kinds.is_empty() {
            return Plugin::Nothing;
        }
        let failed = |why: String| {
            tracing::warn!(
                entry = self.connection.entry,
                "a harness's own hooks report nothing for a session: {why}"
            );
            Plugin::Failed { why, guarded }
        };
        let mut hold = self.hold();
        if hold.book.forward.is_none() {
            match Forward::open() {
                Ok(open) => hold.book.forward = Some(open),
                Err(error) => return failed(format!("the channel could not be opened ({error})")),
            }
        }
        let Some(forward) = hold.book.forward.as_ref() else {
            return failed("the channel is not open".to_owned());
        };
        let (secret, channel) = match forward.enroll(&session.id) {
            Ok(enrolled) => enrolled,
            Err(error) => return failed(format!("no channel for the session ({error})")),
        };
        let before: Vec<String> = self
            .connection
            .before
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let made = self.connection.private().and_then(|under| {
            write_plugin(
                under,
                &self.connection.launcher.to_string_lossy(),
                &before,
                &kinds,
                channel,
            )
        });
        let written = match made {
            Ok(Some(written)) => written,
            Ok(None) => {
                forward.revoke_secret(&secret);
                return failed(
                    "the host's program is at a path a hook command cannot quote".to_owned(),
                );
            }
            Err(error) => {
                forward.revoke_secret(&secret);
                return failed(format!("the plugin could not be written ({error})"));
            }
        };
        let path = written.root().to_string_lossy().into_owned();
        drop(hold);
        let meta = params.entry("_meta").or_insert_with(|| json!({}));
        if !meta.is_object() {
            *meta = json!({});
        }
        let options = &mut meta["claudeCode"];
        if !options.is_object() {
            *options = json!({});
        }
        let options = &mut options["options"];
        if !options.is_object() {
            *options = json!({});
        }
        let plugins = &mut options["plugins"];
        if !plugins.is_array() {
            *plugins = json!([]);
        }
        if let Some(plugins) = plugins.as_array_mut() {
            plugins.push(json!({"type": "local", "path": path}));
        }
        Plugin::Added(Enrolled { secret, written })
    }

    /// Ends the channel and the plugins of a session.
    pub(super) fn revoke_native(&self, session: &str) {
        let mut hold = self.hold();
        if let Some(forward) = &hold.book.forward {
            forward.revoke(session);
        }
        hold.book.plugins.remove(session);
    }

    /// Runs the hooks for what the harness's hooks have reported since the last call, and answers each report.
    pub(in crate::profile::hosted) fn drain_forwarded(&self) {
        let taken = self
            .hold()
            .book
            .forward
            .as_ref()
            .map(Forward::take)
            .unwrap_or_default();
        for report in taken {
            let verdict = self.forwarded(&report);
            report.answer(verdict);
        }
    }

    /// The hooks' session Canoa's id `id` names, wherever it is: open, or still being opened.
    fn session_by_id(&self, id: &str) -> Option<SessionHooks> {
        let hold = self.hold();
        hold.book
            .sessions
            .values()
            .chain(hold.book.opening.values().map(|opening| &opening.hooks))
            .find(|session| session.id == id)
            .cloned()
    }

    /// Where the session works now.
    fn move_to(&self, id: &str, cwd: &str) {
        let mut hold = self.hold();
        let book = &mut hold.book;
        for session in book.sessions.values_mut() {
            if session.id == id {
                cwd.clone_into(&mut session.cwd);
            }
        }
        for opening in book.opening.values_mut() {
            if opening.hooks.id == id {
                cwd.clone_into(&mut opening.hooks.cwd);
            }
        }
    }

    /// What one report amounts to.
    fn forwarded(&self, report: &Forwarded) -> Verdict {
        let Some(session) = self.session_by_id(&report.session) else {
            return Verdict::default();
        };
        let hooks = &self.connection.hooks;
        let previous = session.cwd.clone();
        let moved = report
            .cwd
            .as_deref()
            .filter(|cwd| !cwd.is_empty() && plain(cwd) != plain(&previous));
        let mut session = session;
        if let Some(cwd) = moved {
            self.move_to(&session.id, cwd);
            cwd.clone_into(&mut session.cwd);
        }
        match report.kind {
            Kind::Start => {
                let reason = match report.source.as_deref() {
                    Some("compact") => "compact",
                    Some("clear") => "clear",
                    // A session Canoa opened told the host so itself: this one only says where it is.
                    _ => return Verdict::default(),
                };
                if hooks.listens(HookEvent::SessionStart, &session.allowed) {
                    let fired = self.fire(
                        HookEvent::SessionStart,
                        &session,
                        &[("reason", json!(reason))],
                        Some(reason),
                        &["context"],
                    );
                    let mut hold = self.hold();
                    hold.book.remember(fired.reports.iter().map(report_item));
                    let context = hold.book.took(&session.id, fired.context);
                    if !context.is_empty() {
                        hold.book
                            .carry
                            .entry(session.id)
                            .or_default()
                            .extend(context);
                    }
                }
                Verdict::default()
            }
            Kind::PreCompact => {
                if !hooks.listens(HookEvent::PreCompact, &session.allowed) {
                    return Verdict::default();
                }
                let trigger = match report.source.as_deref() {
                    Some("auto") => "auto",
                    _ => "manual",
                };
                let fired = self.fire(
                    HookEvent::PreCompact,
                    &session,
                    &[("trigger", json!(trigger))],
                    None,
                    &["block"],
                );
                self.hold()
                    .book
                    .remember(fired.reports.iter().map(report_item));
                let block = fired.decision == Some(Decision::Block);
                if block {
                    tracing::info!(
                        entry = self.connection.entry,
                        "a compaction was blocked by an overlay's hook"
                    );
                }
                Verdict { block }
            }
            Kind::Batch => {
                if moved.is_some() && hooks.listens(HookEvent::Cd, &session.allowed) {
                    let fired = self.fire(
                        HookEvent::Cd,
                        &session,
                        &[("previous", json!(previous))],
                        None,
                        &[],
                    );
                    self.hold()
                        .book
                        .remember(fired.reports.iter().map(report_item));
                }
                Verdict::default()
            }
        }
    }
}
