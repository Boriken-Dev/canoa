//! The portal's splash and sign-in page: the brand's mark and colours, light and dark, one card at any width, and a section for each
//! way in the portal accepts. The page is server-rendered from the same list `GET /portal/methods` answers, and its one
//! script (`/portal/login.js`, under `script-src 'self'`) counts a `429`'s wait down and runs the passkey ceremonies a page asks
//! for; what only it can do stays hidden until it runs.

use super::brand::Brand;
use super::signin::Method;
use super::signin::{CertificateStatus, FactorStanding, PasskeyInfo, PinState};
use super::{CERTIFICATE, FACTOR, LOGIN, PASSKEY, PASSKEYS};

/// The page's script, served at [`JS_PATH`].
pub const JS: &str = include_str!("page/login.js");
/// Where the stylesheet is served.
pub const CSS_PATH: &str = "/portal/login.css";
/// Where the script is served.
pub const JS_PATH: &str = "/portal/login.js";
/// Where the favicon, the brand's icon, is served.
pub const ICON_PATH: &str = "/portal/icon.svg";

/// What the page says above the forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A password was refused, or the account is none the portal admits: the same words for both.
    Failed,
    /// A certificate was not accepted.
    Certificate,
    /// The browser sent no certificate.
    NoCertificate,
    /// The certificate is past its validity dates.
    CertificateExpired,
    /// The certificate is pinned to no account.
    CertificateUnpinned,
    /// Too many failures.
    Throttled,
    /// Every sign-in slot is taken.
    Busy,
    /// The accounts could not be asked.
    Unavailable,
    /// A one-time or recovery code was not accepted.
    CodeFailed,
    /// A passkey was not accepted, or the ceremony it answers had ended.
    Passkey,
    /// A sign-in that had passed a step ran out of time or tries.
    Ended,
}

impl Notice {
    /// What the page says for it.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Failed => "Sign-in failed.",
            Self::Certificate => "That certificate was not accepted here.",
            Self::NoCertificate => {
                "Your browser sent no certificate. Check that the card is inserted, then restart the browser: it remembers a choice of none until it restarts."
            }
            Self::CertificateExpired => "That certificate has expired.",
            Self::CertificateUnpinned => "That certificate is not linked to an account here.",
            Self::Throttled => "Too many failed attempts.",
            Self::Busy => "Busy. Try again in a moment.",
            Self::Unavailable => "Sign-in is not available.",
            Self::CodeFailed => "That code was not accepted.",
            Self::Passkey => "That passkey was not accepted.",
            Self::Ended => "That sign-in timed out. Start again.",
        }
    }
}

/// The Content-Security-Policy of the page: nothing inline, nothing from elsewhere.
pub const POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self'; script-src 'self'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// What opens a section only the page's script shows: without it the section stays hidden.
const GATED: &str = "<div hidden data-gated>";
const OR: &str = "<div class=\"or\" role=\"separator\"><span>or</span></div>";

/// `html` as a section the script reveals.
fn gated(html: &str) -> String {
    format!("{GATED}{html}</div>")
}

/// The sections of a page, each after the first set off by "or". A section only the script reveals brings its own separator,
/// so that a page without the script shows none for it.
fn join(sections: &[String]) -> String {
    let mut out = String::new();
    for (index, section) in sections.iter().enumerate() {
        match (index, section.strip_prefix(GATED)) {
            (0, _) => out.push_str(section),
            (_, Some(inner)) => {
                out.push_str(GATED);
                out.push_str(OR);
                out.push_str(inner);
            }
            (_, None) => {
                out.push_str(OR);
                out.push_str(section);
            }
        }
    }
    if !sections.is_empty() && sections.iter().all(|section| section.starts_with(GATED)) {
        out.push_str(
            "<noscript><p class=\"notice\" role=\"alert\">This needs a browser with scripts turned on.</p></noscript>",
        );
    }
    out
}

/// A form the script runs a passkey ceremony for: it asks `PASSKEY` for options with `options_body`, has the browser answer, and
/// posts the answer to `post_to` with `fields`. `kind` is `get` or `create`.
fn passkey_form(
    kind: &str,
    post_to: &str,
    fields: &[(&str, &str)],
    options_url: &str,
    options_body: &str,
    extra: &str,
    button: &str,
) -> String {
    let hidden: String = fields
        .iter()
        .map(|(name, value)| format!("<input type=\"hidden\" name=\"{name}\" value=\"{value}\">"))
        .collect();
    gated(&format!(
        "<form class=\"method\" method=\"post\" action=\"{post_to}\" data-passkey=\"{kind}\" data-options-url=\"{options_url}\" data-options-body=\"{options_body}\">{hidden}<input type=\"hidden\" name=\"answer\" value=\"\">{extra}<button type=\"submit\" class=\"secondary\">{button}</button><p class=\"hint\" data-passkey-status role=\"status\"></p></form><p class=\"hint\" hidden data-passkey-missing>This browser cannot use a passkey.</p>"
    ))
}

/// The passkey step of a sign-in, on the login page.
fn passkey_login_form() -> String {
    passkey_form(
        "get",
        LOGIN,
        &[("method", "passkey")],
        PASSKEY,
        "",
        "",
        "Sign in with a passkey",
    )
}

/// The passkey step a signed-in person shows again on the page at `post_to`, before the policy lets them set something up.
fn passkey_again_form(post_to: &str) -> String {
    passkey_form(
        "get",
        post_to,
        &[("action", "step"), ("method", "passkey")],
        PASSKEY,
        "for=enrol",
        "",
        "Use a passkey",
    )
}

fn password_form() -> String {
    format!(
        "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"password\"><label for=\"user\">Account</label><input id=\"user\" name=\"user\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" required><label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">Sign in</button></form>"
    )
}

fn certificate_form() -> String {
    format!(
        "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"certificate\"><button type=\"submit\" class=\"secondary\">Sign in with your certificate</button><p class=\"hint\">Uses the certificate your browser offered when it opened this page.</p></form>"
    )
}

/// The sign-in page offering `methods`, with `notice` above them. `retry` is the seconds a `429` asks to wait.
#[must_use]
pub fn render(
    brand: &Brand,
    methods: &[Method],
    notice: Option<Notice>,
    retry: Option<u64>,
) -> Vec<u8> {
    let name = brand.name_html();
    let mark = brand.mark();
    let note = notice.map_or_else(String::new, |notice| {
        let wait = if notice == Notice::Throttled {
            " <span id=\"wait\">Try again later.</span>"
        } else {
            ""
        };
        format!(
            "<p class=\"notice\" role=\"alert\">{}{wait}</p>",
            notice.text()
        )
    });
    let mut sections = Vec::new();
    for method in methods {
        sections.push(match method {
            Method::Password => password_form(),
            Method::Certificate => certificate_form(),
            Method::Passkey => passkey_login_form(),
            // A code is checked for an account an earlier step named, and so has no place on the first page.
            Method::Totp | Method::RecoveryCode => continue,
        });
    }
    let body = if sections.is_empty() {
        "<p class=\"notice\" role=\"alert\">No way to sign in is available here.</p>".to_owned()
    } else {
        join(&sections)
    };
    let retry = retry.map_or_else(String::new, |seconds| format!(" data-retry=\"{seconds}\""));
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign in to {name}</title><link rel=\"icon\" type=\"image/svg+xml\" href=\"{ICON_PATH}\"><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main{retry}><header>{mark}<h1>{name}</h1><p class=\"tag\">Sign in to continue</p></header>{note}{body}</main><footer>Boriken Dev</footer><script src=\"{JS_PATH}\"></script></body></html>"
    )
    .into_bytes()
}

/// What the certificate page says above its sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertNotice {
    /// Something was done.
    Done(&'static str),
    /// Something was refused.
    Problem(&'static str),
}

/// `text` as HTML text. A certificate's subject is whatever its maker wrote.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

fn post_button(action: &str, label: &str, class: &str) -> String {
    format!(
        "<form method=\"post\" action=\"{CERTIFICATE}\"><input type=\"hidden\" name=\"action\" value=\"{action}\"><button type=\"submit\" class=\"{class}\">{label}</button></form>"
    )
}

/// The page where `user` sees the certificate their browser offered and the one pinned to their account. `needs_password` says
/// the certificate was not the one a password signed the session in with, so pinning it asks for the password again.
#[must_use]
pub fn render_certificate(
    brand: &Brand,
    user: &str,
    status: Option<&CertificateStatus>,
    needs_password: bool,
    notice: Option<CertNotice>,
    links: Links,
) -> Vec<u8> {
    let name = brand.name_html();
    let mark = brand.mark();
    let note = match notice {
        Some(CertNotice::Done(text)) => {
            format!("<p class=\"notice ok\" role=\"status\">{text}</p>")
        }
        Some(CertNotice::Problem(text)) => format!("<p class=\"notice\" role=\"alert\">{text}</p>"),
        None => String::new(),
    };
    let mut body = String::new();
    match status {
        None => body.push_str("<p class=\"hint\">This portal does not keep certificates.</p>"),
        Some(status) => {
            body.push_str("<h2>This browser's certificate</h2>");
            match &status.presented {
                None => body.push_str(
                    "<p>Your browser did not offer a certificate to this site, or this connection carries none.</p>",
                ),
                Some((presented, state)) => {
                    body.push_str(&format!(
                        "<dl><dt>Subject</dt><dd>{}</dd><dt>SHA-256</dt><dd class=\"mono\">{}</dd></dl>",
                        escape(&presented.subject),
                        escape(&presented.fingerprint)
                    ));
                    body.push_str(&match state {
                        _ if presented.expired => "<p>This certificate has expired.</p>".to_owned(),
                        PinState::Yours => "<p>This certificate signs you in.</p>".to_owned(),
                        PinState::Another => {
                            "<p>This certificate is already used by another account.</p>".to_owned()
                        }
                        _ if status.disabled => {
                            "<p>The certificate pinned to your account is disabled: ask an administrator.</p>"
                                .to_owned()
                        }
                        _ => enrol_form(status.pinned, needs_password),
                    });
                }
            }
            body.push_str("<h2>Pinned to your account</h2>");
            if status.disabled {
                body.push_str("<p>A certificate is pinned to your account, and an administrator disabled it.</p>");
            } else if status.pinned {
                body.push_str("<p>A certificate is pinned to your account.</p>");
                body.push_str(&post_button("remove", "Remove it", "secondary"));
            } else {
                body.push_str("<p>No certificate is pinned.</p>");
            }
        }
    }
    let more = nav(Here::Certificate, links, &name);
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Certificate, {name}</title><link rel=\"icon\" type=\"image/svg+xml\" href=\"{ICON_PATH}\"><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main><header>{mark}<h1>{name}</h1><p class=\"tag\">Signed in as {}</p></header>{note}{body}{more}</main><footer>Boriken Dev</footer></body></html>",
        escape(user)
    )
    .into_bytes()
}

/// The form that pins the certificate on offer, with the password again when the session did not confirm this certificate.
fn enrol_form(replaces: bool, needs_password: bool) -> String {
    let what = if replaces {
        "It replaces the certificate pinned to your account now."
    } else {
        "After this, the certificate alone signs you in."
    };
    let password = if needs_password {
        "<label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required>"
    } else {
        ""
    };
    format!(
        "<p>{what}</p><form method=\"post\" action=\"{CERTIFICATE}\"><input type=\"hidden\" name=\"action\" value=\"enrol\">{password}<button type=\"submit\">Use this certificate to sign in</button></form>"
    )
}

/// Whether the page of a pending sign-in offers to set a one-time code up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupOffer {
    /// Nothing about setting one up.
    None,
    /// The person may set one up now.
    Link,
    /// No code is set up and the policy leaves setting one up to an administrator.
    Administrator,
}

fn code_form(method: Method) -> String {
    let (id, label, hint, autocomplete, mode) = match method {
        Method::RecoveryCode => (
            "recovery",
            "Recovery code",
            "One of the codes you were shown when you set up your code. Each works once.",
            "off",
            "text",
        ),
        _ => (
            "code",
            "Code from your authenticator app",
            "",
            "one-time-code",
            "numeric",
        ),
    };
    let hint = if hint.is_empty() {
        String::new()
    } else {
        format!("<p class=\"hint\">{hint}</p>")
    };
    format!(
        "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"{}\"><label for=\"{id}\">{label}</label><input id=\"{id}\" name=\"code\" autocomplete=\"{autocomplete}\" inputmode=\"{mode}\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"32\" required><button type=\"submit\">Verify</button>{hint}</form>",
        method.id()
    )
}

fn step_certificate_form() -> String {
    format!(
        "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"certificate\"><p class=\"hint\">This step needs your certificate. Your browser must present it to this site.</p><button type=\"submit\" class=\"secondary\">Use your certificate</button></form>"
    )
}

/// The page of a sign-in that has passed a step, offering `methods` for the next, with `notice` above them. `retry` is the seconds
/// a `429` asks to wait.
#[must_use]
pub fn render_step(
    brand: &Brand,
    user: &str,
    methods: &[Method],
    notice: Option<Notice>,
    retry: Option<u64>,
    setup: SetupOffer,
) -> Vec<u8> {
    let name = brand.name_html();
    let mark = brand.mark();
    let note = notice.map_or_else(String::new, |notice| {
        let wait = if notice == Notice::Throttled {
            " <span id=\"wait\">Try again later.</span>"
        } else {
            ""
        };
        format!(
            "<p class=\"notice\" role=\"alert\">{}{wait}</p>",
            notice.text()
        )
    });
    let mut sections = Vec::new();
    for method in methods {
        sections.push(match method {
            Method::Totp => code_form(Method::Totp),
            Method::RecoveryCode => format!(
                "<details class=\"recovery\"><summary>Use a recovery code</summary>{}</details>",
                code_form(Method::RecoveryCode)
            ),
            Method::Password => password_form(),
            Method::Certificate => step_certificate_form(),
            Method::Passkey => passkey_login_form(),
        });
    }
    let mut body = join(&sections);
    match setup {
        SetupOffer::Link => body.push_str(&format!(
            "<p class=\"hint\">No code is set up for your account. <a href=\"{FACTOR}\">Set up a code</a>.</p>"
        )),
        SetupOffer::Administrator => body.push_str(
            "<p class=\"hint\">No code is set up for your account: ask an administrator.</p>",
        ),
        SetupOffer::None => {}
    }
    let cancel = format!(
        "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"cancel\"><button type=\"submit\" class=\"secondary\">Start over</button></form>"
    );
    let retry = retry.map_or_else(String::new, |seconds| format!(" data-retry=\"{seconds}\""));
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign in to {name}</title><link rel=\"icon\" type=\"image/svg+xml\" href=\"{ICON_PATH}\"><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main{retry}><header>{mark}<h1>{name}</h1><p class=\"tag\">One more step for {}</p></header>{note}{body}{cancel}</main><footer>Boriken Dev</footer><script src=\"{JS_PATH}\"></script></body></html>",
        escape(user)
    )
    .into_bytes()
}

/// Where a person is on the page that sets a one-time code up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactorStage<'a> {
    /// This portal keeps no one-time codes.
    Unsupported,
    /// Nothing can be done here, for the reason.
    Blocked(&'static str),
    /// The policy lets no one set a code up for themselves: an administrator does.
    Nobody,
    /// The policy asks these methods again, as fresh proof, before a code may be set up.
    Ask(&'a [Method]),
    /// A code may be set up now.
    Ready,
    /// A setup began: its link, shown once with the recovery codes, and the field for the first code.
    Setup {
        /// The `otpauth://` link.
        uri: &'a str,
        /// The recovery codes.
        codes: &'a [String],
    },
}

/// The QR code of `text` as an inline `<svg>`, dark on light whatever the page's colours; none when `text` does not fit one.
fn qr(text: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let drawn = code
        .render::<svg::Color<'_>>()
        .min_dimensions(224, 224)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    let start = drawn.find("<svg")?;
    Some(drawn[start..].replacen(
        "<svg ",
        "<svg class=\"qr\" role=\"img\" aria-label=\"QR code for your authenticator app\" ",
        1,
    ))
}

/// The forms that show a factor again, as a signed-in person would to sign in, posted to `post_to` as `action=step`.
fn ask_forms(post_to: &str, user: &str, methods: &[Method]) -> String {
    let mut sections = Vec::new();
    for method in methods {
        sections.push(match method {
            Method::Password => format!(
                "<form class=\"method\" method=\"post\" action=\"{post_to}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"password\"><input type=\"hidden\" name=\"user\" value=\"{}\"><label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">Continue</button></form>",
                escape(user)
            ),
            Method::Certificate => format!(
                "<form class=\"method\" method=\"post\" action=\"{post_to}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"certificate\"><p class=\"hint\">Your browser must present your certificate to this site.</p><button type=\"submit\" class=\"secondary\">Use your certificate</button></form>"
            ),
            Method::Passkey => passkey_again_form(post_to),
            Method::Totp | Method::RecoveryCode => {
                let (id, label, mode, autocomplete) = if *method == Method::Totp {
                    ("code", "Current code from your authenticator app", "numeric", "one-time-code")
                } else {
                    ("recovery", "Recovery code", "text", "off")
                };
                format!(
                    "<form class=\"method\" method=\"post\" action=\"{post_to}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"{}\"><label for=\"{id}\">{label}</label><input id=\"{id}\" name=\"code\" autocomplete=\"{autocomplete}\" inputmode=\"{mode}\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"32\" required><button type=\"submit\">Continue</button></form>",
                    method.id()
                )
            }
        });
    }
    join(&sections)
}

fn factor_post(action: &str, label: &str, class: &str) -> String {
    format!(
        "<form method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"{action}\"><button type=\"submit\" class=\"{class}\">{label}</button></form>"
    )
}

/// The pages of the portal's own a signed-in person can move between: which of them this portal has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Links {
    /// The one-time code page.
    pub code: bool,
    /// The Passkeys page.
    pub passkeys: bool,
    /// The certificate page.
    pub certificate: bool,
}

/// The page a [`nav`] list is on, which it leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Here {
    Code,
    Passkeys,
    Certificate,
}

/// The links at the foot of a signed-in page: the other pages the portal has, and the app.
fn nav(here: Here, links: Links, name: &str) -> String {
    let mut out = String::from("<nav class=\"pages\">");
    for (page, on, path, text) in [
        (Here::Code, links.code, FACTOR, "Your one-time code"),
        (Here::Passkeys, links.passkeys, PASSKEYS, "Your passkeys"),
        (
            Here::Certificate,
            links.certificate,
            CERTIFICATE,
            "Your certificate",
        ),
    ] {
        if on && page != here {
            out.push_str(&format!(
                "<p class=\"hint\"><a href=\"{path}\">{text}</a></p>"
            ));
        }
    }
    out.push_str(&format!(
        "<p class=\"hint\"><a href=\"/\">Continue to {name}</a></p></nav>"
    ));
    out
}

/// The page of `user`'s one-time code. `signing_in` says it is part of a sign-in, which the person returns to afterwards.
#[must_use]
pub fn render_factor(
    brand: &Brand,
    user: &str,
    standing: FactorStanding,
    stage: FactorStage<'_>,
    notice: Option<CertNotice>,
    signing_in: bool,
    links: Links,
) -> Vec<u8> {
    let name = brand.name_html();
    let mark = brand.mark();
    let note = match notice {
        Some(CertNotice::Done(text)) => {
            format!("<p class=\"notice ok\" role=\"status\">{text}</p>")
        }
        Some(CertNotice::Problem(text)) => format!("<p class=\"notice\" role=\"alert\">{text}</p>"),
        None => String::new(),
    };
    let mut body = String::new();
    match standing {
        FactorStanding::Enrolled => body.push_str("<p>A code is set up for your account.</p>"),
        FactorStanding::Disabled => body.push_str(
            "<p>A code is set up for your account and an administrator disabled it: ask an administrator.</p>",
        ),
        FactorStanding::NotEnrolled => body.push_str("<p>No code is set up for your account.</p>"),
        _ => {}
    }
    match stage {
        FactorStage::Unsupported => {
            body.push_str("<p class=\"hint\">This portal does not keep one-time codes.</p>");
        }
        FactorStage::Blocked(why) => body.push_str(&format!("<p class=\"hint\">{why}</p>")),
        FactorStage::Nobody => body.push_str(
            "<p class=\"hint\">Setting up a code needs an administrator: ask an administrator.</p>",
        ),
        FactorStage::Ask(methods) => {
            body.push_str("<p>To set up a code, show who you are again.</p>");
            body.push_str(&ask_forms(FACTOR, user, methods));
        }
        FactorStage::Ready => {
            body.push_str("<p>You can set up a code now. It replaces any code you have.</p>");
            body.push_str(&factor_post("begin", "Set up a code", ""));
        }
        FactorStage::Setup { uri, codes } => {
            body.push_str(
                "<p>Scan this with your authenticator app, or enter the link by hand.</p>",
            );
            if let Some(svg) = qr(uri) {
                body.push_str(&format!("<div class=\"qrbox\">{svg}</div>"));
            }
            body.push_str(&format!(
                "<p class=\"mono link\">{}</p><h2>Recovery codes</h2><p>Keep these somewhere safe. Each works once, and they are not shown again.</p><ul class=\"codes mono\">{}</ul>",
                escape(uri),
                codes
                    .iter()
                    .map(|code| format!("<li>{}</li>", escape(code)))
                    .collect::<String>()
            ));
            body.push_str(&format!(
                "<form class=\"method\" method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"confirm\"><label for=\"code\">Code from the app</label><input id=\"code\" name=\"code\" autocomplete=\"one-time-code\" inputmode=\"numeric\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"32\" required><button type=\"submit\">Confirm</button></form>"
            ));
        }
    }
    let back = if signing_in {
        format!(
            "<form class=\"method\" method=\"post\" action=\"{LOGIN}\"><input type=\"hidden\" name=\"method\" value=\"cancel\"><button type=\"submit\" class=\"secondary\">Start over</button></form><p class=\"hint\"><a href=\"{LOGIN}\">Back to the sign-in</a></p>"
        )
    } else {
        nav(Here::Code, links, &name)
    };
    // The script only runs a passkey ceremony here: a setup link and its drawing need none.
    let script = if body.contains("data-passkey") {
        format!("<script src=\"{JS_PATH}\"></script>")
    } else {
        String::new()
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign-in code, {name}</title><link rel=\"icon\" type=\"image/svg+xml\" href=\"{ICON_PATH}\"><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main><header>{mark}<h1>{name}</h1><p class=\"tag\">{} {}</p></header>{note}{body}{back}</main><footer>Boriken Dev</footer>{script}</body></html>",
        if signing_in { "Signing in as" } else { "Signed in as" },
        escape(user)
    )
    .into_bytes()
}

/// Where a person is on the Passkeys page when it comes to adding one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasskeyStage<'a> {
    /// This portal keeps no passkeys.
    Unsupported,
    /// The policy asks for no sign-in that lets a person add one: nobody can.
    Nobody,
    /// The policy asks these methods again, as fresh proof, before a passkey may be added.
    Ask(&'a [Method]),
    /// A passkey may be added now.
    Ready,
}

/// The day a time in seconds since the epoch falls on, in UTC, as `2026-11-23`.
fn day(unix: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let days = unix.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn passkey_item(passkey: &PasskeyInfo) -> String {
    let name = passkey
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .map_or_else(|| "Unnamed passkey".to_owned(), escape);
    let mut meta = Vec::new();
    if let Some(made) = passkey.made_unix {
        meta.push(format!("added {}", day(made)));
    }
    meta.push(passkey.used_unix.map_or_else(
        || "never used".to_owned(),
        |used| format!("used {}", day(used)),
    ));
    meta.push(
        match passkey.synced {
            Some(true) => "synced between your devices",
            Some(false) => "kept on one device",
            None => "",
        }
        .to_owned(),
    );
    if passkey.disabled {
        meta.push("disabled: ask an administrator".to_owned());
    }
    let meta = meta
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let id = escape(&passkey.id);
    format!(
        "<li><div class=\"name\">{name}</div><div class=\"meta\">{meta}</div><form method=\"post\" action=\"{PASSKEYS}\"><input type=\"hidden\" name=\"action\" value=\"label\"><input type=\"hidden\" name=\"id\" value=\"{id}\"><input name=\"label\" aria-label=\"Name\" maxlength=\"64\" autocomplete=\"off\" required><button type=\"submit\" class=\"secondary\">Rename</button></form><form method=\"post\" action=\"{PASSKEYS}\"><input type=\"hidden\" name=\"action\" value=\"remove\"><input type=\"hidden\" name=\"id\" value=\"{id}\"><button type=\"submit\" class=\"secondary\">Remove</button></form></li>"
    )
}

/// The page of `user`'s passkeys: each one, and what it takes to add another. `passkeys` is `None` when they could not be read.
#[must_use]
pub fn render_passkeys(
    brand: &Brand,
    user: &str,
    passkeys: Option<&[PasskeyInfo]>,
    stage: PasskeyStage<'_>,
    notice: Option<CertNotice>,
    links: Links,
) -> Vec<u8> {
    let name = brand.name_html();
    let mark = brand.mark();
    let note = match notice {
        Some(CertNotice::Done(text)) => {
            format!("<p class=\"notice ok\" role=\"status\">{text}</p>")
        }
        Some(CertNotice::Problem(text)) => format!("<p class=\"notice\" role=\"alert\">{text}</p>"),
        None => String::new(),
    };
    let mut body = String::from("<h2>Your passkeys</h2>");
    match passkeys {
        None => body.push_str("<p class=\"hint\">Your passkeys could not be read.</p>"),
        Some([]) => body.push_str("<p>No passkey is set up for your account.</p>"),
        Some(held) => {
            body.push_str("<ul class=\"passkeys\">");
            for passkey in held {
                body.push_str(&passkey_item(passkey));
            }
            body.push_str("</ul>");
        }
    }
    body.push_str("<h2>Add a passkey</h2>");
    match stage {
        PasskeyStage::Unsupported => {
            body.push_str("<p class=\"hint\">This portal does not keep passkeys.</p>");
        }
        PasskeyStage::Nobody => body.push_str(
            "<p class=\"hint\">Adding a passkey needs a sign-in this portal does not offer here: ask an administrator.</p>",
        ),
        PasskeyStage::Ask(methods) => {
            body.push_str("<p>To add a passkey, show who you are again.</p>");
            body.push_str(&ask_forms(PASSKEYS, user, methods));
        }
        PasskeyStage::Ready => {
            body.push_str(&passkey_form(
                "create",
                PASSKEYS,
                &[("action", "confirm")],
                PASSKEYS,
                "action=begin",
                "<label for=\"label\">Name</label><input id=\"label\" name=\"label\" maxlength=\"64\" autocomplete=\"off\" placeholder=\"This device\">",
                "Add a passkey",
            ));
            body.push_str(&format!(
                "<form method=\"post\" action=\"{PASSKEYS}\"><input type=\"hidden\" name=\"action\" value=\"cancel\"><button type=\"submit\" class=\"secondary\">Cancel</button></form>"
            ));
        }
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Passkeys, {name}</title><link rel=\"icon\" type=\"image/svg+xml\" href=\"{ICON_PATH}\"><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main><header>{mark}<h1>{name}</h1><p class=\"tag\">Signed in as {}</p></header>{note}{body}{more}</main><footer>Boriken Dev</footer><script src=\"{JS_PATH}\"></script></body></html>",
        escape(user),
        more = nav(Here::Passkeys, links, &name)
    )
    .into_bytes()
}
