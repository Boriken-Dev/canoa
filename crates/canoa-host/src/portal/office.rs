//! The Office add-in through the portal: the task pane's pages, a sign-in that ends in the Office dialog, and the cookie the
//! task pane's frame carries.
//!
//! The portal's session cookie is `SameSite=Strict`, so a browser does not send it from inside Outlook's page, which is another
//! site: the pane would never be signed in. The dialog Office opens is a window of its own, not the pane's frame, and shares no
//! cookies with it, so a sign-in made there cannot set one for the pane either. What crosses from the dialog to the pane is a
//! string, which names a sign-in that waits in memory (a [`session::Handoff`]); the pane's frame claims it once, from its own
//! page, and the answer sets the frame's cookie, `SameSite=None; Partitioned` so a browser that blocks third-party cookies still
//! keeps one for this top-level site alone.

use super::*;

/// How long the marker that a sign-in is for the Office dialog lasts: the time to type a password and a code.
const MARKER_SECONDS: u64 = 10 * 60;

/// The cookie that sets the pane's session.
fn pane_cookie(token: &str, max_age: u64) -> String {
    format!(
        "{PANE_COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=None; Partitioned; Max-Age={max_age}"
    )
}

/// The header that clears the marker.
pub fn clear_marker() -> String {
    format!("{OFFICE_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
}

/// Whether the browser says its request comes from this site's own page. A browser that says nothing is believed: nothing here
/// changes a session without a string only the dialog was given.
fn own_navigation(request: &Request) -> bool {
    request
        .header("sec-fetch-site")
        .is_none_or(|site| site == "same-origin")
}

/// The answer to a request for one of the add-in's routes, or `None` for any other path, which the portal answers as ever.
pub fn answer(inner: &Inner, request: &Request, conn: &Conn, path: &str) -> Option<Response> {
    let office = inner.office.as_ref()?;
    match (request.method.as_str(), path) {
        ("GET" | "HEAD", OFFICE_SIGN_IN) => return Some(start(request)),
        ("GET" | "HEAD", OFFICE_SESSION) => return Some(state(inner, request, conn)),
        ("POST", OFFICE_SESSION) => return Some(claim(inner, request, conn)),
        ("GET" | "HEAD", _) => {}
        _ => return None,
    }
    let segments = web::segments(path)?;
    let route = web::Office::route(&segments)?;
    let headers = office.headers(&route);
    Some(match &route {
        web::Route::Page(page) => {
            let mut all = vec![
                ("Content-Type", web::content_type(page.name()).to_owned()),
                ("Cache-Control", page.cache().to_owned()),
            ];
            all.extend(headers);
            Response {
                status: 200,
                headers: all,
                payload: Payload::Bytes(page.body()),
            }
        }
        // The icon is drawn by Office from its own page, before anyone has signed in.
        web::Route::Icon => site_file(
            inner,
            request,
            web::ICON_FILE
                .iter()
                .map(|part| (*part).to_owned())
                .collect(),
            true,
            headers,
        ),
        // The app is the signed-in site's own page. A frame that is refused shows nothing of it, so the pane asks first.
        web::Route::App => match authed(inner, request) {
            Some(_) => site_file(
                inner,
                request,
                vec!["index.html".to_owned()],
                false,
                headers,
            ),
            None => Response::refusal(401),
        },
    })
}

/// `GET /office/signin`: what the Office dialog goes to first. It marks the browser as signing in for the dialog and sends it to
/// the sign-in page; the sign-in then ends at the dialog's page and not at the site. Only this site's own page may ask, so
/// another site cannot make a person's next ordinary sign-in end in a dialog.
fn start(request: &Request) -> Response {
    if !own_navigation(request) {
        return Response::refusal(403);
    }
    Response::redirect(LOGIN).with(
        "Set-Cookie",
        format!(
            "{OFFICE_COOKIE}=1; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={MARKER_SECONDS}"
        ),
    )
}

/// `GET /office/session`: whether the pane's frame is signed in, and as whom.
fn state(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let Some(session) = authed(inner, request) else {
        return Response::json(401, &serde_json::json!({"signed_in": false}));
    };
    match resume(inner, request, conn, &session) {
        Ok(()) => Response::json(
            200,
            &serde_json::json!({"signed_in": true, "user": session.user}),
        ),
        Err(refusal) if refusal.status == 401 => {
            Response::json(401, &serde_json::json!({"signed_in": false}))
        }
        Err(refusal) => refusal,
    }
}

/// `POST /office/session` `{"handoff": STRING}`: claims the sign-in the dialog made and opens a session for this frame, whose
/// cookie the answer sets. The string is good for one claim and for `office_handoff_seconds`; the session is the one a sign-in
/// at the site would have opened, and is asked about again, as any session is, with the address this request comes from.
fn claim(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    if !from_own_page(request, conn) {
        return Response::refusal(403);
    }
    let Some(string) = serde_json::from_slice::<serde_json::Value>(&request.body)
        .ok()
        .and_then(|body| {
            body.get("handoff")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    else {
        return Response::refusal(400);
    };
    let Some(handoff) = inner.handoffs.take(&string) else {
        return Response::json(
            401,
            &serde_json::json!({"error": "that sign-in is not waiting: sign in again"}),
        );
    };
    let user = handoff.user.clone();
    let Some((token, session)) = inner.sessions.open_after(
        &handoff.user,
        handoff.uid,
        handoff.method,
        handoff.password_at,
        handoff.fingerprint,
        handoff.done,
        handoff.code_made,
    ) else {
        return Response::refusal(503);
    };
    if let Err(refusal) = resume(inner, request, conn, &session) {
        inner.sessions.close(&token);
        return refusal;
    }
    tracing::info!(
        "{user}'s sign-in made in the Office dialog was claimed by a task pane at {}",
        shown(client_address(inner, request, conn))
    );
    Response::json(200, &serde_json::json!({"signed_in": true, "user": user})).with(
        "Set-Cookie",
        pane_cookie(&token, inner.settings.session.as_secs()),
    )
}
