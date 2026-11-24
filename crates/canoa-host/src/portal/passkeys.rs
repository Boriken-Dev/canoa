//! Passkeys on the portal's own pages: `/portal/passkey` makes the challenge a browser signs, and `/portal/passkeys` is where a
//! signed-in person sees their passkeys, removes or names one, and adds another.
//!
//! Signing in by passkey is one more step the sign-in page takes (`steps::take_step`); this module serves what the step needs
//! before it, and the page that keeps the passkeys. A passkey is added only after the person shows again, under the policy's
//! `enrol` purpose, what that rule asks for: a way into the account is added with a sign-in as strong as the strongest way in,
//! so a session a stolen cookie opened, or a password typed in an office, never plants one that signs in from anywhere.

use super::page::{CertNotice, PasskeyStage};
use super::session::{Ceremony, Pending, Purpose};
use super::steps::{self, Cx, Stale};
use super::*;

/// The cookie that names the passkey challenge a browser holds. Like the session's, it is `__Host-`: no sibling site can set it.
pub const COOKIE: &str = "__Host-canoa-passkey";

/// How long a challenge made now lives, from the provider's own expiry; at least a second and at most an hour.
fn lifetime(expires_unix: i64) -> Duration {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_secs()).ok())
        .unwrap_or(0);
    Duration::from_secs(
        u64::try_from(expires_unix.saturating_sub(now))
            .unwrap_or(0)
            .clamp(1, 3600),
    )
}

fn cookie(token: &str, life: Duration) -> String {
    format!(
        "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
        life.as_secs()
    )
}

/// The account and its groups as the directory has them now.
struct Who {
    user: String,
    uid: u32,
    groups: Vec<String>,
}

fn who_of(inner: &Inner, session: &Session) -> Result<Who, Response> {
    match inner.accounts.still(&session.user, session.uid) {
        Standing::Present(groups) => Ok(Who {
            user: session.user.clone(),
            uid: session.uid,
            groups,
        }),
        Standing::Gone => Err(Response::redirect(LOGIN)),
        _ => Err(Response::refusal(503)),
    }
}

/// The signed-in person, whose session is still good.
fn signed_in(inner: &Inner, request: &Request, conn: &Conn) -> Result<Session, Response> {
    let Some(session) = authed(inner, request) else {
        return Err(Response::redirect(LOGIN));
    };
    match resume(inner, request, conn, &session) {
        Ok(()) => Ok(session),
        // A session that ended is a browser to sign in again.
        Err(refusal) if refusal.status == 401 => Err(Response::redirect(LOGIN)),
        Err(refusal) => Err(refusal),
    }
}

/// The steps a signed-in person has shown again for the purpose `enrol`, if they have begun.
fn enrol_pending(inner: &Inner, request: &Request, session: &Session) -> Option<(String, Pending)> {
    steps::pending_of(inner, request, Purpose::Enrol)
        .filter(|(_, pending)| pending.session == Some(session.id) && pending.uid == session.uid)
}

/// The policy's answer for the purpose `enrol` with the steps in `done`.
fn enrol_decision(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    done: Vec<coqui_access::Factor>,
) -> Result<policy::Decision, Response> {
    let context = policy::context(
        purposes::ENROL,
        client_address(inner, request, conn),
        Some(policy::person(&who.user, who.uid, &who.groups)),
        done,
    );
    match inner.policy.decide(context) {
        Ok(mut decision) => {
            usable(inner, conn, &mut decision);
            Ok(decision)
        }
        Err(why) => {
            tracing::warn!("the sign-in policy could not decide: {why}");
            Err(Response::refusal(503))
        }
    }
}

/// `POST /portal/passkey`: a challenge for the passkey step the browser is at. With `for=enrol`, for a signed-in person showing a
/// passkey again before adding one; otherwise for the sign-in: the pending one's account, or any passkey when there is none.
/// The answer is the options for `navigator.credentials.get`, and the cookie names the challenge the answer is checked with.
pub fn challenge(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    if !from_own_page(request, conn) {
        return Response::refusal(403);
    }
    let (account, decision) = if form_field(&request.body, "for").as_deref() == Some("enrol") {
        let session = match signed_in(inner, request, conn) {
            Ok(session) => session,
            Err(_) => return Response::refusal(401),
        };
        let who = match who_of(inner, &session) {
            Ok(who) => who,
            Err(_) => return Response::refusal(401),
        };
        let done = enrol_pending(inner, request, &session)
            .map(|(_, pending)| pending.done)
            .unwrap_or_default();
        match enrol_decision(inner, request, conn, &who, done) {
            Ok(decision) => (Some((who.user, who.uid)), decision),
            Err(refusal) => return refusal,
        }
    } else {
        let pending =
            steps::pending_of(inner, request, Purpose::SignIn).and_then(|(_, pending)| {
                match steps::decide_pending(inner, request, conn, &pending, purposes::SIGN_IN) {
                    Ok((decision, _)) if !decision.complete && !decision.offers.is_empty() => {
                        Some(Ok((Some((pending.user, pending.uid)), decision)))
                    }
                    Err(Stale::Unavailable) => Some(Err(Response::refusal(503))),
                    _ => None,
                }
            });
        match pending {
            Some(Ok(found)) => found,
            Some(Err(refusal)) => return refusal,
            None => match offer_for(inner, request, conn) {
                Some(decision) => (None, decision),
                None => return Response::refusal(503),
            },
        }
    };
    let Some(offer) = decision.offer(Method::Passkey) else {
        tracing::info!(
            "a passkey from {} is not accepted (rule {})",
            shown(client_address(inner, request, conn)),
            decision.rule_name()
        );
        return Response::refusal(403);
    };
    let address = client_of(inner, request, conn);
    let made = match inner.accounts.passkey_challenge(
        account.as_ref().map(|(user, uid)| (user.as_str(), *uid)),
        address,
        &offer.providers,
    ) {
        Ok(made) => made,
        Err(PasskeyError::Busy) => {
            tracing::info!("a passkey challenge for {address} was refused: too many are waiting");
            return Response::refusal(429).with("Retry-After", "30".to_owned());
        }
        Err(PasskeyError::NoPasskey) => return Response::refusal(404),
        Err(PasskeyError::Unsupported) => return Response::refusal(403),
        Err(other) => {
            tracing::warn!("a passkey challenge could not be made: {other:?}");
            return Response::refusal(503);
        }
    };
    let Ok(options) = serde_json::from_str::<serde_json::Value>(&made.options) else {
        tracing::warn!("the passkey provider's options are not JSON");
        return Response::refusal(503);
    };
    let life = lifetime(made.expires_unix);
    let Some(token) = inner.ceremonies.open(
        Ceremony {
            challenge: made.challenge,
            account,
        },
        life,
        request.cookie(COOKIE).as_deref(),
    ) else {
        return Response::refusal(429).with("Retry-After", "30".to_owned());
    };
    Response::json(200, &serde_json::json!({ "options": options }))
        .with("Set-Cookie", cookie(&token, life))
}

/// What the person may do on the page now.
enum Gate {
    Unsupported,
    Nobody,
    Ask(Vec<Method>),
    Ready,
}

fn gate(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<&Pending>,
) -> Result<Gate, Response> {
    if matches!(
        inner.accounts.passkeys(&who.user, who.uid),
        Err(PasskeyError::Unsupported)
    ) {
        return Ok(Gate::Unsupported);
    }
    let done = enrol
        .map(|pending| pending.done.clone())
        .unwrap_or_default();
    let decision = enrol_decision(inner, request, conn, who, done)?;
    Ok(if decision.complete {
        Gate::Ready
    } else if decision.offers.is_empty() {
        Gate::Nobody
    } else {
        Gate::Ask(decision.methods())
    })
}

fn stage_of(gate: &Gate) -> PasskeyStage<'_> {
    match gate {
        Gate::Unsupported => PasskeyStage::Unsupported,
        Gate::Nobody => PasskeyStage::Nobody,
        Gate::Ask(methods) => PasskeyStage::Ask(methods),
        Gate::Ready => PasskeyStage::Ready,
    }
}

fn page(
    inner: &Inner,
    who: &Who,
    gate: &Gate,
    notice: Option<CertNotice>,
    status: u16,
) -> Response {
    let held = inner.accounts.passkeys(&who.user, who.uid).ok();
    Response::text(
        status,
        "text/html; charset=utf-8",
        page::render_passkeys(
            &inner.brand,
            &who.user,
            held.as_deref(),
            stage_of(gate),
            notice,
            account::links(inner, &who.user, who.uid),
        ),
    )
    .with("Content-Security-Policy", page::POLICY.to_owned())
}

/// The page as it stands now, with `notice` above.
fn stand(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<&Pending>,
    notice: Option<CertNotice>,
    status: u16,
) -> Response {
    match gate(inner, request, conn, who, enrol) {
        Ok(gate) => page(inner, who, &gate, notice, status),
        Err(response) => response,
    }
}

/// `GET /portal/passkeys`.
pub fn show(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let session = match signed_in(inner, request, conn) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let who = match who_of(inner, &session) {
        Ok(who) => who,
        Err(response) => return response,
    };
    let enrol = enrol_pending(inner, request, &session);
    stand(
        inner,
        request,
        conn,
        &who,
        enrol.as_ref().map(|(_, pending)| pending),
        None,
        200,
    )
}

/// `POST /portal/passkeys`: `action` is `step` (show a factor again), `begin`, `confirm`, `remove`, `label` or `cancel`.
pub fn act(inner: &Inner, request: &Request, conn: &Conn, started: Instant) -> Response {
    if !from_own_page(request, conn) {
        return Response::refusal(403);
    }
    if !request
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"))
    {
        return Response::refusal(400);
    }
    let session = match signed_in(inner, request, conn) {
        Ok(session) => session,
        // A POST with no account behind it is turned away, not sent to sign in.
        Err(response) if response.status == 303 => return Response::refusal(401),
        Err(response) => return response,
    };
    let who = match who_of(inner, &session) {
        Ok(who) => who,
        Err(response) if response.status == 303 => return Response::refusal(401),
        Err(response) => return response,
    };
    let enrol = enrol_pending(inner, request, &session);
    match form_field(&request.body, "action").as_deref() {
        Some("step") => step(inner, request, conn, started, &session, &who, enrol),
        Some("begin") => begin(inner, request, conn, &who, enrol),
        Some("confirm") => confirm(inner, request, conn, &who, enrol),
        Some("remove") => remove(inner, request, conn, &who, enrol),
        Some("label") => label(inner, request, conn, &who, enrol),
        Some("cancel") => {
            if let Some((token, _)) = &enrol {
                inner.pendings.close(token);
            }
            Response::redirect(PASSKEYS).with("Set-Cookie", steps::clear_pending())
        }
        _ => Response::refusal(400),
    }
}

/// A factor shown again, as a signed-in person would to sign in.
fn step(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    started: Instant,
    session: &Session,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let Some(method) = steps::method_of(form_field(&request.body, "method").as_deref()) else {
        return Response::refusal(400);
    };
    let done = enrol
        .as_ref()
        .map(|(_, pending)| pending.done.clone())
        .unwrap_or_default();
    let decision = match enrol_decision(inner, request, conn, who, done) {
        Ok(decision) => decision,
        Err(response) => return response,
    };
    let ask = decision.methods();
    let again = Gate::Ask(ask);
    let fail = |status: u16, notice: page::Notice, _retry: Option<u64>| {
        page(
            inner,
            who,
            &again,
            Some(CertNotice::Problem(notice.text())),
            status,
        )
    };
    let ended = || {
        page(
            inner,
            who,
            &again,
            Some(CertNotice::Problem(page::Notice::Ended.text())),
            401,
        )
        .with("Set-Cookie", steps::clear_pending())
    };
    let cx = Cx {
        inner,
        request,
        conn,
        started,
        account: Some((who.user.as_str(), who.uid)),
        groups: &who.groups,
        pending: enrol.as_ref(),
        fail: &fail,
        ended: &ended,
    };
    let taken = match steps::take_step(&cx, &decision, method) {
        Ok(taken) => taken,
        Err(response) => return response,
    };
    let (token, mut state) = match enrol {
        Some(found) => found,
        None => (
            String::new(),
            Pending {
                user: who.user.clone(),
                uid: who.uid,
                purpose: Purpose::Enrol,
                session: Some(session.id),
                done: Vec::new(),
                tries: 0,
                first: None,
                password_at: None,
                fingerprint: None,
                passkey: None,
            },
        ),
    };
    steps::record_step(inner, request, conn, &mut state, &taken);
    let token = if token.is_empty() {
        match inner.pendings.open_for(state.clone(), inner.settings.enrol) {
            Some(token) => token,
            None => {
                taken.guard.succeeded(inner);
                return fail(503, page::Notice::Busy, None);
            }
        }
    } else {
        inner.pendings.record(&token, &state);
        token
    };
    taken.guard.succeeded(inner);
    stand(inner, request, conn, who, Some(&state), None, 200).with(
        "Set-Cookie",
        steps::pending_cookie(inner, &token, inner.settings.enrol),
    )
}

/// What the person is told for a refused change, when the gate says no.
fn gated(inner: &Inner, who: &Who, gate: &Gate) -> Response {
    page(inner, who, gate, None, 403)
}

/// Begins adding a passkey: the options the browser creates it from. Only once the steps the policy asks for are shown.
fn begin(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let gate = match gate(inner, request, conn, who, found) {
        Ok(gate) => gate,
        Err(response) => return response,
    };
    if !matches!(gate, Gate::Ready) {
        tracing::info!(
            "adding a passkey from {} was not allowed",
            shown(client_address(inner, request, conn))
        );
        return Response::refusal(403);
    }
    match inner.accounts.begin_passkey(&who.user, who.uid) {
        Ok(options) => match serde_json::from_str::<serde_json::Value>(&options) {
            Ok(options) => {
                tracing::info!(
                    "{} began adding a passkey from {}",
                    who.user,
                    shown(client_address(inner, request, conn))
                );
                Response::json(200, &serde_json::json!({ "options": options }))
            }
            Err(_) => {
                tracing::warn!("the passkey provider's creation options are not JSON");
                Response::refusal(503)
            }
        },
        Err(PasskeyError::Refused(why)) => {
            tracing::info!("adding a passkey for {} was refused: {why}", who.user);
            Response::refusal(403)
        }
        Err(other) => {
            tracing::warn!("a passkey could not be begun: {other:?}");
            Response::refusal(503)
        }
    }
}

/// Confirms the addition with the browser's answer.
fn confirm(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let gate = match gate(inner, request, conn, who, found) {
        Ok(gate) => gate,
        Err(response) => return response,
    };
    if !matches!(gate, Gate::Ready) {
        return gated(inner, who, &gate);
    }
    let (Some(answer), label) = (
        form_field(&request.body, "answer"),
        form_field(&request.body, "label").unwrap_or_default(),
    ) else {
        return Response::refusal(400);
    };
    let from = shown(client_address(inner, request, conn));
    let problem = |text, status| page(inner, who, &gate, Some(CertNotice::Problem(text)), status);
    match inner
        .accounts
        .confirm_passkey(&who.user, who.uid, &answer, &label)
    {
        Ok(id) => {
            tracing::warn!("{} added passkey {id} from {from}", who.user);
            // One registration is one fresh sign-in: the next asks again.
            if let Some((token, _)) = &enrol {
                inner.pendings.close(token);
            }
            let again = gate_after(inner, request, conn, who);
            page(
                inner,
                who,
                &again,
                Some(CertNotice::Done("This passkey now signs you in.")),
                200,
            )
            .with("Set-Cookie", steps::clear_pending())
        }
        Err(PasskeyError::NothingWaiting) => problem(
            "No passkey was being added, or it timed out. Add it again.",
            400,
        ),
        Err(PasskeyError::Refused(why)) => {
            tracing::info!("a passkey for {} from {from} was refused: {why}", who.user);
            problem("That passkey was not accepted.", 403)
        }
        Err(other) => {
            tracing::warn!("a passkey could not be added: {other:?}");
            problem("The passkey could not be saved.", 503)
        }
    }
}

/// The gate once the steps shown are used up: the page asks again for the next.
fn gate_after(inner: &Inner, request: &Request, conn: &Conn, who: &Who) -> Gate {
    gate(inner, request, conn, who, None).unwrap_or(Gate::Nobody)
}

/// Takes one passkey away. A person's own session may: a stolen cookie can take a way in out, and never put one in.
fn remove(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let Some(id) = form_field(&request.body, "id") else {
        return Response::refusal(400);
    };
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let notice = match inner.accounts.remove_passkey(&who.user, who.uid, &id) {
        Ok(true) => {
            tracing::warn!(
                "{} removed passkey {id} from {}",
                who.user,
                shown(client_address(inner, request, conn))
            );
            // What the passkey let in does not outlive it.
            inner.sessions.close_passkey(&who.user, Some(&id));
            CertNotice::Done("The passkey is removed.")
        }
        Ok(false) => CertNotice::Problem("That passkey is not on your account."),
        Err(other) => {
            tracing::warn!("a passkey could not be removed: {other:?}");
            CertNotice::Problem("The passkey could not be removed.")
        }
    };
    stand(inner, request, conn, who, found, Some(notice), 200)
}

/// Names a passkey.
fn label(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let (Some(id), Some(label)) = (
        form_field(&request.body, "id"),
        form_field(&request.body, "label"),
    ) else {
        return Response::refusal(400);
    };
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let notice = match inner
        .accounts
        .label_passkey(&who.user, who.uid, &id, &label)
    {
        Ok(()) => CertNotice::Done("The passkey is renamed."),
        Err(PasskeyError::NoPasskey) => CertNotice::Problem("That passkey is not on your account."),
        Err(other) => {
            tracing::warn!("a passkey could not be renamed: {other:?}");
            CertNotice::Problem("The passkey could not be renamed.")
        }
    };
    stand(inner, request, conn, who, found, Some(notice), 200)
}

/// What the account's Passkeys page offers now.
pub(super) struct PasskeyPage {
    /// How many passkeys the account holds; `None` when they could not be read.
    pub count: Option<usize>,
    /// Why adding one is not possible now.
    pub adding: Option<&'static str>,
}

/// What `GET /portal/account` says of the Passkeys page: how many passkeys the account holds (`None` when they could not be
/// read) and, when adding one is not possible now, why. `None` for a portal that keeps no passkeys. It is the decision the page
/// makes, from the same gate.
pub(super) fn availability(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    session: &Session,
) -> Result<Option<PasskeyPage>, Response> {
    let who = who_of(inner, session)?;
    let enrol = enrol_pending(inner, request, session);
    let stage = gate(
        inner,
        request,
        conn,
        &who,
        enrol.as_ref().map(|(_, pending)| pending),
    )?;
    let count = inner
        .accounts
        .passkeys(&who.user, who.uid)
        .ok()
        .map(|held| held.len());
    Ok(match stage {
        Gate::Unsupported => None,
        Gate::Ready | Gate::Ask(_) => Some(PasskeyPage {
            count,
            adding: None,
        }),
        Gate::Nobody => Some(PasskeyPage {
            count,
            adding: Some("Adding a passkey is not allowed from here: ask an administrator."),
        }),
    })
}
