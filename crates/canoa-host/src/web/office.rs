//! The Office add-in's routes: the task pane's pages, the page of the app they frame, and the add-in's icons, under `/office/`.
//!
//! Off unless `[host.web] office = true`. With it off, `/office/...` is an ordinary path and every answer has the one policy
//! of [`super::policy`]. With it on, only the routes named here are answered differently, each with a policy that relaxes
//! exactly what Office needs of it and nothing else:
//!
//! | Route | Relaxed |
//! | --- | --- |
//! | `/office/taskpane.html` | Office.js may load; Office on the web may frame it (no `X-Frame-Options`); it may frame `/office/app/` |
//! | `/office/dialog.html` | Office.js may load; no `Cross-Origin-Opener-Policy`, so the dialog keeps its link to the window that opened it |
//! | `/office/app/` | the site's `index.html`, under today's policy, but its `frame-ancestors` names this page and Office on the web |
//! | `/office/icon-N.png` | `Cross-Origin-Resource-Policy: cross-origin`, since Office draws it from its own origin |
//!
//! `taskpane.js`, `relation.js` and `dialog.js` are scripts of this origin and are answered with the usual headers. Every other
//! path under `/office/` is the site's, as before.

use alloc::string::String;
use alloc::vec::Vec;

/// Where Office.js is loaded from: the address in the `<script>` tag of Microsoft Learn's "Referencing the Office JavaScript API
/// library" (`/office/dev/add-ins/develop/referencing-the-javascript-api-for-office-library-from-its-cdn`), whose page also says
/// the add-in's network allow lists must hold `*.static.microsoft`. Only this one host is allowed here.
pub(crate) const JS_ORIGIN: &str = "https://officeapis.public.onecdn.static.microsoft";

/// The origins of Office on the web that may frame an add-in's pages. Microsoft publishes no list of the origins that frame a
/// task pane, so these are a starting list to check against each client's blocked-frame report, and `[host.web] office_frames`
/// adds to it:
///
/// - `outlook.office.com`, `outlook.office365.com`: Outlook on the web for a work or school mailbox (its own addresses);
/// - `outlook.live.com`: Outlook on the web for a personal mailbox (its own address);
/// - `*.office.com`, `*.cloud.microsoft`: the families Microsoft 365's web apps run on, the second being the unified domain
///   Microsoft Learn describes in "Introducing cloud.microsoft" (linked from the Office.js reference page above).
///
/// Word, Excel and PowerPoint on the web are served from hosts none of these names (SharePoint, `officeapps.live.com`); they
/// are not listed until measured, and are added with `office_frames`.
pub(crate) const FRAMERS: &[&str] = &[
    "https://outlook.office.com",
    "https://outlook.office365.com",
    "https://outlook.live.com",
    "https://*.office.com",
    "https://*.cloud.microsoft",
];

/// The pages of the add-in this host carries, by name under `/office/`.
pub(crate) struct Page {
    name: &'static str,
    body: &'static str,
    kind: Kind,
}

/// What a page is, which says what is relaxed for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// The task pane, framed by Office.
    Pane,
    /// The dialog Office opens, never framed.
    Dialog,
    /// A script of this origin.
    Script,
}

const PAGES: &[Page] = &[
    Page {
        name: "taskpane.html",
        body: include_str!("../../../../clients/office/taskpane/taskpane.html"),
        kind: Kind::Pane,
    },
    Page {
        name: "taskpane.js",
        body: include_str!("../../../../clients/office/taskpane/taskpane.js"),
        kind: Kind::Script,
    },
    Page {
        name: "relation.js",
        body: include_str!("../../../../clients/office/taskpane/relation.js"),
        kind: Kind::Script,
    },
    Page {
        name: "dialog.html",
        body: include_str!("../../../../clients/office/taskpane/dialog.html"),
        kind: Kind::Dialog,
    },
    Page {
        name: "dialog.js",
        body: include_str!("../../../../clients/office/taskpane/dialog.js"),
        kind: Kind::Script,
    },
];

/// The sizes the manifests ask icons in; each is the site's one icon, which the browser scales.
const ICONS: &[&str] = &["icon-16.png", "icon-32.png", "icon-64.png", "icon-80.png"];

/// Where the site keeps the icon that [`ICONS`] are.
pub(crate) const ICON_FILE: &[&str] = &["icons", "Icon-192.png"];

/// Which route a path under `/office/` is, when it is one of the add-in's.
pub(crate) enum Route {
    /// A page or script the host carries.
    Page(&'static Page),
    /// The site's `index.html`, as the chat-only app's page, which the task pane frames.
    App,
    /// The site's icon.
    Icon,
}

/// Whether `source` is one origin a `frame-ancestors` may name here: `https://`, a host of letters, digits, `-` and `.` that may
/// start with `*.` (and then holds a dot more, so `https://*.com` is not one), and an optional port. Nothing that could end the
/// directive or widen it.
pub(crate) fn frame_source(source: &str) -> bool {
    let Some(rest) = source.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = rest.split_once(':').unwrap_or((rest, "0"));
    let (wildcard, name) = host
        .strip_prefix("*.")
        .map_or((false, host), |name| (true, name));
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && (!wildcard || name.contains('.'))
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

/// The three policies of the routes that are relaxed, made once when the server binds.
pub(crate) struct Office {
    pane: String,
    dialog: String,
    app: String,
}

impl Office {
    /// The policies for the site's own `connect` sources and the origins `extra` adds to [`FRAMERS`].
    pub(crate) fn new(connect: &[&str], extra: &[String]) -> Self {
        let framers: String = FRAMERS
            .iter()
            .copied()
            .chain(extra.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            // Office.js needs to load, and the pane to frame the app: nothing else is added to a policy with no default.
            pane: format!(
                "default-src 'none'; script-src 'self' {JS_ORIGIN}; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; font-src 'self' data:; connect-src 'self' {JS_ORIGIN}; frame-src 'self'; \
                 object-src 'none'; base-uri 'self'; form-action 'none'; frame-ancestors {framers}"
            ),
            dialog: format!(
                "default-src 'none'; script-src 'self' {JS_ORIGIN}; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; connect-src 'self' {JS_ORIGIN}; object-src 'none'; base-uri 'self'; \
                 form-action 'none'; frame-ancestors 'none'"
            ),
            app: super::policy_framed(connect, &format!("'self' {framers}")),
        }
    }

    /// The route a request for `segments` is, or `None` for a path that is the site's as ever.
    pub(crate) fn route(segments: &[String]) -> Option<Route> {
        let [first, second] = segments else {
            return None;
        };
        if first != "office" {
            return None;
        }
        if second == "app" {
            return Some(Route::App);
        }
        if ICONS.contains(&second.as_str()) {
            return Some(Route::Icon);
        }
        PAGES
            .iter()
            .find(|page| page.name == second)
            .map(Route::Page)
    }

    /// The headers a route's answer carries in place of the usual ones. A header with an empty value is not sent.
    pub(crate) fn headers(&self, route: &Route) -> Vec<(&'static str, String)> {
        match route {
            Route::App => vec![
                ("Content-Security-Policy", self.app.clone()),
                ("X-Frame-Options", String::new()),
            ],
            Route::Icon => vec![("Cross-Origin-Resource-Policy", "cross-origin".to_owned())],
            Route::Page(page) => match page.kind {
                Kind::Pane => vec![
                    ("Content-Security-Policy", self.pane.clone()),
                    ("X-Frame-Options", String::new()),
                ],
                Kind::Dialog => vec![
                    ("Content-Security-Policy", self.dialog.clone()),
                    ("Cross-Origin-Opener-Policy", String::new()),
                ],
                Kind::Script => Vec::new(),
            },
        }
    }
}

impl Page {
    /// The page as a body, with its type.
    pub(crate) fn body(&self) -> Vec<u8> {
        self.body.as_bytes().to_vec()
    }

    /// Its file name, for the media type.
    pub(crate) const fn name(&self) -> &'static str {
        self.name
    }
}
