//! The `file_viewer` kind: a way to show a file of some type in the application's Files surface.
//!
//! Which viewers the application has beyond the ones it draws itself is not Canoa's to say: each is a provider of this kind, which
//! says what types of file it shows and how, and answers a file's contents with what to draw. The application never runs a
//! viewer's code: the host asks the provider, which hands back either an A2UI surface (the application draws it in its own
//! widgets and colours) or a self-contained page (a web view), and the application draws or opens only what came back.
//!
//! The provider never sees a path. The host reads the file and gives the provider its name, type, size and bytes, so a viewer
//! shows exactly the file the person opened and reaches nothing else of the computer through it.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `file_viewer` kind.
pub const KIND: &str = "file_viewer";

/// The most types, and the most extensions, one viewer may name.
pub const MAX_TYPES: usize = 64;

/// The longest answer to [`FileViewer::render`] the host takes, in bytes: a surface's messages or a page's document.
pub const MAX_RENDER_BYTES: usize = 4 * 1024 * 1024;

/// What a viewer answers with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Shows {
    /// A JSON array of A2UI messages ([`FileViewer::render`] answers it as text), which the application draws with its own
    /// widgets in the person's theme.
    Surface,
    /// One self-contained HTML document with its styles, scripts and pictures inline. The host puts a policy on it that allows no
    /// request, form or frame, and the application opens it in the person's browser, since it embeds no web view.
    Page,
}

/// What a viewer is, from [`FileViewer::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ViewerInfo {
    /// The id the viewer is found by: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people, as the Open with menu says it.
    pub title: String,
    /// A line saying what it shows.
    pub help: Option<String>,
    /// The MIME types it shows: exact (`application/json`) or by family (`image/*`).
    pub mime_types: Vec<String>,
    /// The extensions, without the dot, it shows whatever the type says.
    pub extensions: Vec<String>,
    /// Which of two viewers that can show a file is tried first: the higher. The application's own viewers are at -10 to 10.
    pub priority: i32,
    /// What it answers with.
    pub shows: Shows,
    /// Whether it opens a document at a section ([`ViewRequest::section`]).
    pub opens_sections: bool,
}

impl ViewerInfo {
    /// A viewer showing files of `mime_types` with `shows`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        shows: Shows,
        mime_types: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            mime_types,
            extensions: Vec::new(),
            priority: 0,
            shows,
            opens_sections: false,
        }
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.id.is_empty()
            && self.id.len() <= 32
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(alloc::format!(
                "the viewer id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the viewer {:?} has no title", self.id));
        }
        if self.mime_types.is_empty() && self.extensions.is_empty() {
            return Some(alloc::format!(
                "the viewer {:?} names no type and no extension",
                self.id
            ));
        }
        if self.mime_types.len() > MAX_TYPES || self.extensions.len() > MAX_TYPES {
            return Some(alloc::format!(
                "the viewer {:?} names more than {MAX_TYPES} types or extensions",
                self.id
            ));
        }
        if let Some(bad) = self.mime_types.iter().find(|each| !is_mime(each)) {
            return Some(alloc::format!(
                "the viewer {:?} names {bad:?}, which is not a MIME type (type/subtype or type/*, lower case)",
                self.id
            ));
        }
        if let Some(bad) = self.extensions.iter().find(|each| !is_extension(each)) {
            return Some(alloc::format!(
                "the viewer {:?} names the extension {bad:?}, which is not lower-case letters and digits without a dot",
                self.id
            ));
        }
        None
    }
}

fn is_mime(text: &str) -> bool {
    let Some((family, kind)) = text.split_once('/') else {
        return false;
    };
    let token = |part: &str| {
        !part.is_empty()
            && part.len() <= 127
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'.' | b'+' | b'_')
            })
    };
    token(family) && (kind == "*" || token(kind))
}

fn is_extension(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// A file to show: what the host read of it, and no path.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ViewRequest {
    /// The file's name, without its folder.
    pub name: String,
    /// The type the host judged the file to be, when it could.
    pub mime: Option<String>,
    /// The file's whole size in bytes.
    pub size: u64,
    /// Whether `data` stops short of the file.
    pub truncated: bool,
    /// What the host read of the file, in base64: its start, as many bytes as one answer of the host carries.
    pub data: String,
    /// The heading to open at, by its anchor or its title, for a viewer that [`ViewerInfo::opens_sections`].
    pub section: Option<String>,
}

impl ViewRequest {
    /// A request for the file `name` of `size` bytes, whose bytes read are `data` in base64.
    #[must_use]
    pub fn new(name: impl Into<String>, size: u64, data: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mime: None,
            size,
            truncated: false,
            data: data.into(),
            section: None,
        }
    }
}

/// A way to show a file.
///
/// `describe` answers the same on every call with nothing run. `render` is asked once for each time a person opens a file with the
/// viewer.
#[coqui_acp_api::guatiao::kind(name = "file_viewer", crate = ::coqui_acp_api::guatiao)]
pub trait FileViewer: Send + Sync {
    /// What this viewer is.
    fn describe(&self) -> Result<ViewerInfo, ProviderError>;

    /// The file drawn: for [`Shows::Surface`] a JSON array of A2UI messages that creates a surface with a `root` component, for
    /// [`Shows::Page`] an HTML document. At most [`MAX_RENDER_BYTES`] bytes.
    fn render(&self, request: ViewRequest) -> Result<String, ProviderError>;
}

/// The policy the host puts on a page before it gives it out: the page draws from itself (inline styles and scripts, pictures and
/// fonts as `data:` or `blob:` addresses) and can make no request, submit no form, open no frame and set no base address.
pub const PAGE_POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'";

/// `html` as the document the application opens: the policy first, so that nothing the page says comes before it.
#[must_use]
pub fn sealed_page(html: &str) -> String {
    alloc::format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{PAGE_POLICY}\">{html}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer() -> ViewerInfo {
        ViewerInfo::new(
            "stamp",
            "Stamp",
            Shows::Surface,
            alloc::vec!["application/x-stamp".into()],
        )
    }

    #[test]
    fn a_viewer_with_a_word_for_an_id_a_title_and_a_type_is_usable() {
        assert_eq!(viewer().problem(), None);
        let mut by_extension = viewer();
        by_extension.mime_types.clear();
        by_extension.extensions.push("stamp".into());
        assert_eq!(by_extension.problem(), None);
        let mut family = viewer();
        family.mime_types = alloc::vec!["image/*".into(), "text/x-diff".into()];
        assert_eq!(family.problem(), None);
    }

    #[test]
    fn an_id_that_is_no_word_a_blank_title_or_no_type_is_refused() {
        for id in ["", "Stamp", "a b", &"a".repeat(33)] {
            let mut info = viewer();
            info.id = id.into();
            assert!(info.problem().is_some(), "{id:?}");
        }
        let mut blank = viewer();
        blank.title = " ".into();
        assert!(blank.problem().is_some_and(|why| why.contains("title")));
        let mut none = viewer();
        none.mime_types.clear();
        assert!(none.problem().is_some_and(|why| why.contains("no type")));
    }

    #[test]
    fn a_type_that_is_not_a_mime_type_or_an_extension_with_a_dot_is_refused() {
        for mime in ["json", "Text/Plain", "text/", "/plain", "*/*", "text/a b"] {
            let mut info = viewer();
            info.mime_types = alloc::vec![mime.into()];
            assert!(
                info.problem().is_some_and(|why| why.contains("MIME")),
                "{mime}"
            );
        }
        for extension in [".stamp", "Stamp", "", "a/b"] {
            let mut info = viewer();
            info.extensions = alloc::vec![extension.into()];
            assert!(
                info.problem().is_some_and(|why| why.contains("extension")),
                "{extension}"
            );
        }
        let mut many = viewer();
        many.extensions = (0..=MAX_TYPES).map(|n| alloc::format!("e{n}")).collect();
        assert!(many.problem().is_some_and(|why| why.contains("more than")));
    }

    #[test]
    fn a_page_is_given_out_under_a_policy_that_comes_first() {
        let page = sealed_page("<p>hi</p>");
        assert!(page.starts_with(
            "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\""
        ));
        assert!(page.ends_with("<p>hi</p>"));
        assert!(PAGE_POLICY.starts_with("default-src 'none'"));
        assert!(
            !PAGE_POLICY.contains("connect-src"),
            "no request is allowed"
        );
    }
}
