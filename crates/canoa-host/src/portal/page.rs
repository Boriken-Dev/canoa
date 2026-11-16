//! The portal's splash and sign-in page: Canoa's mark and colours, light and dark, one card at any width, and a section for each
//! way in the portal accepts. The page is server-rendered from the same list `GET /portal/methods` answers, and its one
//! script (`/portal/login.js`, under `script-src 'self'`) only counts a `429`'s wait down.

use super::signin::Method;
use super::signin::{CertificateStatus, FactorStanding, PinState};
use super::{CERTIFICATE, FACTOR, LOGIN};

/// The page's stylesheet, served at [`CSS_PATH`].
pub const CSS: &str = include_str!("page/login.css");
/// The page's script, served at [`JS_PATH`].
pub const JS: &str = include_str!("page/login.js");
/// Where the stylesheet is served.
pub const CSS_PATH: &str = "/portal/login.css";
/// Where the script is served.
pub const JS_PATH: &str = "/portal/login.js";

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
            Self::Ended => "That sign-in timed out. Start again.",
        }
    }
}

/// The Content-Security-Policy of the page: nothing inline, nothing from elsewhere.
pub const POLICY: &str = "default-src 'none'; style-src 'self'; script-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

// The paths of `app/assets/icons/canoa.svg`, at the size the page draws them.
const MARK: &str = r##"<svg class="mark" viewBox="0 0 64 64" width="72" height="72" role="img" aria-label="Canoa"><defs><mask id="carved" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64"><path fill="#fff" d="M 2 23 C 5 27 9 30 15 30 L 49 30 C 55 30 59 27 60.5 17 L 63.5 17 C 64.5 31 59 41 49 41.5 C 38 42.5 26 42.5 15 41.5 C 6 40.5 1 32 0 24 Z"/><path fill="#000" d="M 34.1 14.8 L 19.2 45.5 L 26.1 48.8 L 40.9 18.2 Z"/><path fill="none" stroke="#000" stroke-width="1.3" stroke-linecap="round" d="M 47.1 35.9 C 47.3 36.2 47.1 36.9 46.5 37.1 C 45.9 37.4 44.8 36.9 44.6 35.9 C 44.4 34.9 45.2 33.5 46.5 33.3 C 47.8 33.1 49.5 34.2 49.8 35.9 C 50 37.6 48.6 39.6 46.5 39.8 C 44.4 40.1 42.1 38.3 41.9 35.9"/></mask></defs><g fill="#3DAFA1"><rect width="64" height="64" mask="url(#carved)"/><path d="M 43.3 0.2 L 21.8 44.5 C 20 48.4 16.6 46.8 14.8 50.6 C 12.1 56.1 14.7 61.3 15 63 C 16.5 62.2 22.2 61 24.8 55.5 C 26.7 51.6 23.4 50 25.3 46.2 L 46.7 1.8 Z"/></g></svg>"##;

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
pub fn render(methods: &[Method], notice: Option<Notice>, retry: Option<u64>) -> Vec<u8> {
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
            // A code is checked for an account an earlier step named, and so has no place on the first page.
            Method::Totp | Method::RecoveryCode => continue,
        });
    }
    let body = if sections.is_empty() {
        "<p class=\"notice\" role=\"alert\">No way to sign in is available here.</p>".to_owned()
    } else {
        sections.join("<div class=\"or\" role=\"separator\"><span>or</span></div>")
    };
    let retry = retry.map_or_else(String::new, |seconds| format!(" data-retry=\"{seconds}\""));
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign in to Canoa</title><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main{retry}><header>{MARK}<h1>Canoa</h1><p class=\"tag\">Sign in to continue</p></header>{note}{body}</main><footer>Boriken Dev</footer><script src=\"{JS_PATH}\"></script></body></html>"
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
fn escape(text: &str) -> String {
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
    user: &str,
    status: Option<&CertificateStatus>,
    needs_password: bool,
    notice: Option<CertNotice>,
) -> Vec<u8> {
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
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Certificate, Canoa</title><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main><header>{MARK}<h1>Canoa</h1><p class=\"tag\">Signed in as {}</p></header>{note}{body}<p class=\"hint\"><a href=\"/\">Continue to Canoa</a></p></main><footer>Boriken Dev</footer></body></html>",
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
    user: &str,
    methods: &[Method],
    notice: Option<Notice>,
    retry: Option<u64>,
    setup: SetupOffer,
) -> Vec<u8> {
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
        });
    }
    let mut body = sections.join("<div class=\"or\" role=\"separator\"><span>or</span></div>");
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
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign in to Canoa</title><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main{retry}><header>{MARK}<h1>Canoa</h1><p class=\"tag\">One more step for {}</p></header>{note}{body}{cancel}</main><footer>Boriken Dev</footer><script src=\"{JS_PATH}\"></script></body></html>",
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

fn factor_post(action: &str, label: &str, class: &str) -> String {
    format!(
        "<form method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"{action}\"><button type=\"submit\" class=\"{class}\">{label}</button></form>"
    )
}

/// The page of `user`'s one-time code. `signing_in` says it is part of a sign-in, which the person returns to afterwards.
#[must_use]
pub fn render_factor(
    user: &str,
    standing: FactorStanding,
    stage: FactorStage<'_>,
    notice: Option<CertNotice>,
    signing_in: bool,
) -> Vec<u8> {
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
            let mut sections = Vec::new();
            for method in methods {
                sections.push(match method {
                    Method::Password => format!(
                        "<form class=\"method\" method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"password\"><input type=\"hidden\" name=\"user\" value=\"{}\"><label for=\"password\">Password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required><button type=\"submit\">Continue</button></form>",
                        escape(user)
                    ),
                    Method::Certificate => format!(
                        "<form class=\"method\" method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"certificate\"><p class=\"hint\">Your browser must present your certificate to this site.</p><button type=\"submit\" class=\"secondary\">Use your certificate</button></form>"
                    ),
                    Method::Totp | Method::RecoveryCode => {
                        let (id, label, mode, autocomplete) = if *method == Method::Totp {
                            ("code", "Current code from your authenticator app", "numeric", "one-time-code")
                        } else {
                            ("recovery", "Recovery code", "text", "off")
                        };
                        format!(
                            "<form class=\"method\" method=\"post\" action=\"{FACTOR}\"><input type=\"hidden\" name=\"action\" value=\"step\"><input type=\"hidden\" name=\"method\" value=\"{}\"><label for=\"{id}\">{label}</label><input id=\"{id}\" name=\"code\" autocomplete=\"{autocomplete}\" inputmode=\"{mode}\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"32\" required><button type=\"submit\">Continue</button></form>",
                            method.id()
                        )
                    }
                });
            }
            body.push_str(
                &sections.join("<div class=\"or\" role=\"separator\"><span>or</span></div>"),
            );
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
        "<p class=\"hint\"><a href=\"/\">Continue to Canoa</a></p>".to_owned()
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Sign-in code, Canoa</title><link rel=\"stylesheet\" href=\"{CSS_PATH}\"></head><body><main><header>{MARK}<h1>Canoa</h1><p class=\"tag\">{} {}</p></header>{note}{body}{back}</main><footer>Boriken Dev</footer></body></html>",
        if signing_in { "Signing in as" } else { "Signed in as" },
        escape(user)
    )
    .into_bytes()
}
