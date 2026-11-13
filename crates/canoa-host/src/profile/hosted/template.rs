//! Templating (`spec/v0/harness.md#templating`): `${NAME}` in an overlay's context files and in the body of its bundled
//! skills, replaced when the text is handed to the agent. Installed files are never changed: the texts are replaced on
//! their way out.
//!
//! `AOS_OVERLAY_ROOT` is the private copy of the overlay's verified files, the one its hooks run from, made when a text
//! names it. The overlay has no state directory and Canoa gives its scopes no label, so those two names keep no value.
//! `AOS_PROJECT_ROOT` is the folder a session is opened in, so a context that names it is replaced again for each
//! session; a skill's body is written once for the connection and leaves it as written.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use coqui_aos::{self as aos, Document, Settings, TemplateFacts};

use super::ReadOverlay;
use super::hooks::Hooks;
use crate::profile::AosLayer;

/// What a text needs to be replaced again for a session: the file's bytes and the overlay's facts.
#[derive(Clone, Debug)]
pub(super) struct Again {
    bytes: Vec<u8>,
    manifest: Document,
    facts: TemplateFacts,
    /// Whether the bytes are an `OVERLAY.md` body, whose front matter is already dropped.
    body: bool,
}

impl Again {
    /// The text with `project` as `AOS_PROJECT_ROOT`.
    pub(super) fn render(&self, project: Option<&str>) -> Result<String, String> {
        let mut facts = self.facts.clone();
        facts.project = project.map(str::to_owned);
        let values = aos::template_values(&self.manifest, &facts).map_err(words)?;
        if self.body {
            let text = core::str::from_utf8(&self.bytes)
                .map_err(|_| "its text is not UTF-8".to_owned())?;
            return Ok(aos::template(text, &values));
        }
        aos::context_text(&self.bytes, &values, &Settings::default()).map_err(words)
    }
}

/// A text replaced with no project, and what replacing it for a session needs when it names the project.
pub(super) struct Rendered {
    pub(super) text: String,
    pub(super) again: Option<Again>,
}

/// An overlay's texts, from what a connection holds of it.
pub(super) struct Templating<'a> {
    pub(super) overlays: &'a [ReadOverlay],
    pub(super) hooks: &'a Hooks,
    pub(super) layers: &'a [AosLayer],
    /// The person's option values by overlay, from canoa.cfg.
    pub(super) options: &'a BTreeMap<String, Document>,
}

/// An error in words that quote no value.
fn words(error: aos::Error) -> String {
    if error.class == aos::Class::InvalidOption {
        format!(
            "the option {} is not of the type its overlay declares",
            error.subject
        )
    } else {
        format!(
            "its text cannot be read ({}: {})",
            error.class, error.subject
        )
    }
}

impl Templating<'_> {
    /// The manifest and facts of `identity`; its root is made only when `needs_root`.
    fn facts(&self, identity: &str, needs_root: bool) -> Result<(Document, TemplateFacts), String> {
        let layer = self
            .layers
            .iter()
            .find(|layer| layer.identity == identity)
            .ok_or("its overlay is not active")?;
        let mut facts = TemplateFacts::new()
            .name(aos::normalize_name(identity))
            .options(self.options.get(identity).cloned().unwrap_or_default());
        if needs_root {
            let read = self
                .overlays
                .iter()
                .find(|read| read.identity == identity)
                .ok_or("its overlay is not read")?;
            let root = self
                .hooks
                .copy_of(identity, read)
                .ok_or("a private copy of its overlay cannot be made")?;
            facts = facts.root(root.to_string_lossy().into_owned());
        }
        Ok((layer.manifest.clone(), facts))
    }

    /// The context file `path` of `identity`, replaced, with no project.
    pub(super) fn context(&self, identity: &str, path: &str) -> Result<Rendered, String> {
        let bytes = self
            .overlays
            .iter()
            .find(|read| read.identity == identity)
            .and_then(|read| read.file(path))
            .ok_or_else(|| format!("the file {path} cannot be read as UTF-8 text"))?;
        let names = |name: &str| contains(bytes, name);
        let (manifest, facts) = self.facts(identity, names("AOS_OVERLAY_ROOT"))?;
        let again = Again {
            bytes: bytes.to_vec(),
            manifest,
            facts,
            body: false,
        };
        let text = again.render(None)?;
        Ok(Rendered {
            text,
            again: names("AOS_PROJECT_ROOT").then_some(again),
        })
    }

    /// The body of `OVERLAY.md` of `identity`, replaced, with no project.
    pub(super) fn body(&self, identity: &str, body: &str) -> Result<Rendered, String> {
        let names = |name: &str| body.contains(name);
        let (manifest, facts) = self.facts(identity, names("AOS_OVERLAY_ROOT"))?;
        let again = Again {
            bytes: body.as_bytes().to_vec(),
            manifest,
            facts,
            body: true,
        };
        let text = again.render(None)?;
        Ok(Rendered {
            text,
            again: names("AOS_PROJECT_ROOT").then_some(again),
        })
    }

    /// A bundled skill's `SKILL.md` of `identity`, its body replaced and its front matter as written.
    pub(super) fn skill(&self, identity: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let (manifest, facts) = self.facts(identity, contains(bytes, "AOS_OVERLAY_ROOT"))?;
        let values = aos::template_values(&manifest, &facts).map_err(words)?;
        aos::skill_text(bytes, &values, &Settings::default())
            .map(String::into_bytes)
            .map_err(words)
    }
}

fn contains(bytes: &[u8], name: &str) -> bool {
    bytes
        .windows(name.len())
        .any(|window| window == name.as_bytes())
}
