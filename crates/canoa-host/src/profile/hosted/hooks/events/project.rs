//! A project's context files for a session: the files of the directory it opens in when it opens, and, where the harness tells
//! the host that the session moved, the files of each deeper directory it enters.

use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use coqui_transport::Channel;

use super::SessionHooks;
use crate::profile::hosted::project_context::{self, Fed};
use crate::profile::hosted::{Hosted, Item};

/// What a session's opening gives its project's context.
#[derive(Debug, Default)]
pub(super) struct Opened {
    /// The files to hand over, each as its path and text.
    pub(super) files: Vec<(String, String)>,
    /// What the report says of them.
    pub(super) items: Vec<Item>,
}

impl<C: Channel + ?Sized> Hosted<'_, C> {
    /// The files the harness leaves to the host: what it says it loads itself is not among them.
    fn native_context(&self) -> Vec<&'static str> {
        crate::vendors::by_aos(&self.harness_label())
            .map(|vendor| vendor.context_files)
            .unwrap_or_default()
    }

    /// Reads the context files of the session's directory `cwd` and makes it the root of the session's walk. `system` is whether
    /// they go with the system prompt, rather than with the first prompt.
    pub(super) fn open_project(
        &self,
        session: &mut SessionHooks,
        cwd: &str,
        system: bool,
    ) -> Opened {
        if cwd.is_empty() {
            return Opened::default();
        }
        let held = self.hold().book.used(&session.id);
        let how = if system {
            "ACP systemPrompt, appended"
        } else {
            "with the session's first prompt"
        };
        let fed = project_context::feed(
            self.connection.hooks.project_names(),
            &self.native_context(),
            Path::new(cwd),
            Path::new(cwd),
            &[],
            held,
            how,
        );
        cwd.clone_into(&mut session.root);
        session.fed.clone_from(&fed.levels);
        session.follows = fed.found;
        let Fed { files, items, .. } = fed;
        let files = self.hold().book.took(&session.id, files);
        Opened { files, items }
    }

    /// The session moved: the files of the directories between where it opened and where it is now, which it has not been
    /// given, wait for its next prompt.
    pub(super) fn follow_project(&self, session: &SessionHooks) {
        if !session.follows || session.root.is_empty() {
            return;
        }
        let held = self.hold().book.used(&session.id);
        let fed = project_context::feed(
            self.connection.hooks.project_names(),
            &self.native_context(),
            Path::new(&session.root),
            Path::new(&session.cwd),
            &session.fed,
            held,
            "with the next prompt",
        );
        let mut hold = self.hold();
        let Fed {
            files,
            items,
            levels,
            ..
        } = fed;
        for each in hold.book.sessions.values_mut() {
            if each.id == session.id {
                each.fed.extend(levels.iter().cloned());
            }
        }
        for opening in hold.book.opening.values_mut() {
            if opening.hooks.id == session.id {
                opening.hooks.fed.extend(levels.iter().cloned());
            }
        }
        let files = hold.book.took(&session.id, files);
        hold.book.remember(items);
        if !files.is_empty() {
            hold.book
                .project
                .entry(session.id.clone())
                .or_default()
                .extend(files);
        }
    }
}
