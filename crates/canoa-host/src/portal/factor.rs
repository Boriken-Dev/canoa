//! `/portal/factor`: whether a one-time code is set up, and setting one up.
//!
//! Two people come here. A signed-in person asks the policy, for the purpose `enrol`, which factors they must show again before
//! they may set a code up, and shows them as they would to sign in; a sign-in that has passed its first step and is asked for a
//! code the account does not have may set one up when the same policy allows it for the steps it has passed. Nobody else gets
//! this far: with no cookie there is no account, and with no `enrol` rule the page says an administrator does it.

use super::page::{CertNotice, FactorStage};
use super::session::{Pending, Purpose};
use super::steps::{self, Cx, Stale, Taken};
use super::*;

/// Who is asking.
enum Actor {
    /// A signed-in person.
    Session(Session),
    /// A sign-in that has passed a step, with the token of its pending state.
    Setup(String, Pending),
}

/// The account and its groups as the directory has them now.
struct Who {
    user: String,
    uid: u32,
    groups: Vec<String>,
}

/// What the person may do on the page now.
enum Gate {
    Unsupported,
    Blocked(&'static str),
    Nobody,
    Ask(Vec<Method>),
    Ready,
}

fn actor(inner: &Inner, request: &Request, conn: &Conn) -> Result<Actor, Response> {
    if let Some(session) = authed(inner, request) {
        return match resume(inner, request, conn, &session) {
            Ok(()) => Ok(Actor::Session(session)),
            // A session that ended is a browser to sign in again.
            Err(refusal) if refusal.status == 401 => Err(Response::redirect(LOGIN)),
            Err(refusal) => Err(refusal),
        };
    }
    if let Some((token, pending)) = steps::pending_of(inner, request, Purpose::SignIn) {
        return Ok(Actor::Setup(token, pending));
    }
    Err(Response::redirect(LOGIN))
}

fn who_of(inner: &Inner, actor: &Actor) -> Result<Who, Response> {
    let (user, uid) = match actor {
        Actor::Session(session) => (session.user.clone(), session.uid),
        Actor::Setup(_, pending) => (pending.user.clone(), pending.uid),
    };
    match inner.accounts.still(&user, uid) {
        Standing::Present(groups) => Ok(Who { user, uid, groups }),
        Standing::Gone => Err(Response::redirect(LOGIN)),
        _ => Err(Response::refusal(503)),
    }
}

/// The enrolment steps a signed-in person has passed, if they have begun.
fn enrol_pending(inner: &Inner, request: &Request, actor: &Actor) -> Option<(String, Pending)> {
    match actor {
        Actor::Session(session) => {
            steps::pending_of(inner, request, Purpose::Enrol).filter(|(_, pending)| {
                pending.session == Some(session.id) && pending.uid == session.uid
            })
        }
        Actor::Setup(..) => None,
    }
}

/// The policy's answer for the purpose `enrol` with the steps in `done`.
fn enrol_decision(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    who: &Who,
    purpose: &str,
    done: Vec<coqui_access::Factor>,
) -> Result<policy::Decision, Response> {
    let context = policy::context(
        purpose,
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

fn gate(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    actor: &Actor,
    who: &Who,
    enrol: Option<&Pending>,
) -> Result<(FactorStanding, Gate), Response> {
    let standing = inner.accounts.factor_standing(&who.user, who.uid);
    match standing {
        FactorStanding::Unsupported => return Ok((standing, Gate::Unsupported)),
        FactorStanding::Unknown => return Err(Response::refusal(503)),
        FactorStanding::Reset => {
            return Ok((
                standing,
                Gate::Blocked(
                    "An administrator reset your code: ask an administrator to set up a new one.",
                ),
            ));
        }
        FactorStanding::Disabled => {
            return Ok((
                standing,
                Gate::Blocked("An administrator disabled your code: ask an administrator."),
            ));
        }
        _ => {}
    }
    let done = match actor {
        Actor::Setup(_, pending) => {
            // A person with a code already set up signs in with it, or with a recovery code: a password alone never gets them a
            // new one.
            if standing == FactorStanding::Enrolled {
                return Ok((
                    standing,
                    Gate::Blocked(
                        "A code is set up already. Sign in with it or with a recovery code; to replace it, ask an administrator.",
                    ),
                ));
            }
            match steps::decide_pending(inner, request, conn, pending, purposes::SIGN_IN) {
                Ok((signin, _)) if signin.offer(Method::Totp).is_some() => {}
                Ok(_) => {
                    return Ok((
                        standing,
                        Gate::Blocked("Your sign-in does not ask for a code."),
                    ));
                }
                Err(Stale::Gone) => return Err(Response::redirect(LOGIN)),
                Err(Stale::Unavailable) => return Err(Response::refusal(503)),
            }
            pending.done.clone()
        }
        Actor::Session(_) => enrol
            .map(|pending| pending.done.clone())
            .unwrap_or_default(),
    };
    // A sign-in that is waiting for a code is asked as `enrol_first`, which only a rule that names it allows: whoever holds the
    // steps it has passed is the one who would set a code up, and the person it belongs to is not the one asked.
    let purpose = match actor {
        Actor::Setup(..) => purposes::ENROL_FIRST,
        Actor::Session(_) => purposes::ENROL,
    };
    let decision = enrol_decision(inner, request, conn, who, purpose, done)?;
    let stage = if decision.complete {
        Gate::Ready
    } else if matches!(actor, Actor::Session(_)) && !decision.offers.is_empty() {
        Gate::Ask(decision.methods())
    } else {
        Gate::Nobody
    };
    Ok((standing, stage))
}

fn stage_of<'a>(gate: &'a Gate) -> FactorStage<'a> {
    match gate {
        Gate::Unsupported => FactorStage::Unsupported,
        Gate::Blocked(why) => FactorStage::Blocked(why),
        Gate::Nobody => FactorStage::Nobody,
        Gate::Ask(methods) => FactorStage::Ask(methods),
        Gate::Ready => FactorStage::Ready,
    }
}

fn page(
    who: &Who,
    standing: FactorStanding,
    stage: FactorStage<'_>,
    notice: Option<CertNotice>,
    signing_in: bool,
    status: u16,
) -> Response {
    Response::text(
        status,
        "text/html; charset=utf-8",
        page::render_factor(&who.user, standing, stage, notice, signing_in),
    )
    .with("Content-Security-Policy", page::POLICY.to_owned())
}

/// The page as it stands, with `notice` above.
#[allow(
    clippy::too_many_arguments,
    reason = "the request's parts and the page's state, passed through from the handlers"
)]
fn stand(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    actor: &Actor,
    who: &Who,
    enrol: Option<&Pending>,
    notice: Option<CertNotice>,
    status: u16,
) -> Response {
    match gate(inner, request, conn, actor, who, enrol) {
        Ok((standing, gate)) => page(
            who,
            standing,
            stage_of(&gate),
            notice,
            matches!(actor, Actor::Setup(..)),
            status,
        ),
        Err(response) => response,
    }
}

/// `GET /portal/factor`.
pub fn show(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let actor = match actor(inner, request, conn) {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let who = match who_of(inner, &actor) {
        Ok(who) => who,
        Err(response) => return response,
    };
    let enrol = enrol_pending(inner, request, &actor);
    stand(
        inner,
        request,
        conn,
        &actor,
        &who,
        enrol.as_ref().map(|(_, pending)| pending),
        None,
        200,
    )
}

/// `POST /portal/factor`: `action` is `step` (show a factor again), `begin`, `confirm` or `cancel`.
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
    let actor = match actor(inner, request, conn) {
        Ok(actor) => actor,
        // A POST with no account behind it is turned away, not sent to sign in.
        Err(response) if response.status == 303 => return Response::refusal(401),
        Err(response) => return response,
    };
    let who = match who_of(inner, &actor) {
        Ok(who) => who,
        Err(response) if response.status == 303 => return Response::refusal(401),
        Err(response) => return response,
    };
    // A portal that keeps no codes has nothing to set up, whoever asks.
    if inner.accounts.factor_standing(&who.user, who.uid) == FactorStanding::Unsupported {
        return page(
            &who,
            FactorStanding::Unsupported,
            FactorStage::Unsupported,
            None,
            matches!(actor, Actor::Setup(..)),
            403,
        );
    }
    let enrol = enrol_pending(inner, request, &actor);
    match form_field(&request.body, "action").as_deref() {
        Some("step") => step(inner, request, conn, started, &actor, &who, enrol),
        Some("begin") => begin(inner, request, conn, &actor, &who, enrol),
        Some("confirm") => confirm(inner, request, conn, started, &actor, &who, enrol),
        Some("cancel") => match &actor {
            Actor::Session(_) => {
                if let Some((token, _)) = &enrol {
                    inner.pendings.close(token);
                }
                Response::redirect(FACTOR).with("Set-Cookie", steps::clear_pending())
            }
            Actor::Setup(..) => Response::redirect(LOGIN),
        },
        _ => Response::refusal(400),
    }
}

/// A factor shown again, as a signed-in person would to sign in.
fn step(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    started: Instant,
    actor: &Actor,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let Actor::Session(session) = actor else {
        return Response::refusal(403);
    };
    let Some(method) = steps::method_of(form_field(&request.body, "method").as_deref()) else {
        return Response::refusal(400);
    };
    let done = enrol
        .as_ref()
        .map(|(_, pending)| pending.done.clone())
        .unwrap_or_default();
    let decision = match enrol_decision(inner, request, conn, who, purposes::ENROL, done) {
        Ok(decision) => decision,
        Err(response) => return response,
    };
    let ask = decision.methods();
    let standing = inner.accounts.factor_standing(&who.user, who.uid);
    let fail = |status: u16, notice: page::Notice, _retry: Option<u64>| {
        page(
            who,
            standing,
            FactorStage::Ask(&ask),
            Some(CertNotice::Problem(notice.text())),
            false,
            status,
        )
    };
    let ended = || {
        page(
            who,
            standing,
            FactorStage::Ask(&ask),
            Some(CertNotice::Problem(page::Notice::Ended.text())),
            false,
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
    stand(inner, request, conn, actor, who, Some(&state), None, 200).with(
        "Set-Cookie",
        steps::pending_cookie(inner, &token, inner.settings.enrol),
    )
}

/// Begins the setup: the link and the recovery codes, said once.
fn begin(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    actor: &Actor,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let (standing, gate) = match gate(inner, request, conn, actor, who, found) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let signing_in = matches!(actor, Actor::Setup(..));
    if !matches!(gate, Gate::Ready) {
        tracing::info!(
            "a one-time code setup from {} was not allowed",
            shown(client_address(inner, request, conn))
        );
        return page(who, standing, stage_of(&gate), None, signing_in, 403);
    }
    match inner.accounts.begin_factor(&who.user, who.uid) {
        Ok(enrolment) => {
            let from = shown(client_address(inner, request, conn));
            if signing_in {
                // A code set up by whoever holds a password, before any session, is the one thing here an owner may not have asked
                // for: said at the level they read.
                tracing::warn!(
                    "{} began setting up a one-time code during a sign-in from {from}, the first one the account has",
                    who.user
                );
            } else {
                tracing::info!("{} began setting up a one-time code from {from}", who.user);
            }
            page(
                who,
                standing,
                FactorStage::Setup {
                    uri: &enrolment.uri,
                    codes: &enrolment.codes,
                },
                None,
                signing_in,
                200,
            )
        }
        Err(why) => {
            tracing::warn!("a one-time code could not be set up: {}", reason(&why));
            page(
                who,
                standing,
                stage_of(&gate),
                Some(CertNotice::Problem("The code could not be set up.")),
                signing_in,
                503,
            )
        }
    }
}

/// What a factor error says in a log; a store's own reason, never a code.
fn reason(error: &FactorError) -> String {
    match error {
        FactorError::Failed(why) => why.clone(),
        other => format!("{other:?}"),
    }
}

/// Confirms the setup with the first code from the app. In a sign-in, the code is also the step the sign-in was waiting for.
fn confirm(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    started: Instant,
    actor: &Actor,
    who: &Who,
    enrol: Option<(String, Pending)>,
) -> Response {
    let found = enrol.as_ref().map(|(_, pending)| pending);
    let (standing, gate) = match gate(inner, request, conn, actor, who, found) {
        Ok(found) => found,
        Err(response) => return response,
    };
    let signing_in = matches!(actor, Actor::Setup(..));
    if !matches!(gate, Gate::Ready) {
        return page(who, standing, stage_of(&gate), None, signing_in, 403);
    }
    let Some(code) = form_field(&request.body, "code") else {
        return Response::refusal(400);
    };
    let pending = match actor {
        Actor::Setup(token, state) => Some((token.clone(), state.clone())),
        Actor::Session(_) => enrol,
    };
    // A try is a try wherever it is made: the same counts as a code typed to sign in, and the pending state's tries.
    let fail = |status: u16, notice: page::Notice, _retry: Option<u64>| {
        page(
            who,
            standing,
            FactorStage::Ready,
            Some(CertNotice::Problem(notice.text())),
            signing_in,
            status,
        )
    };
    let ended = || {
        page(
            who,
            standing,
            FactorStage::Blocked("Start again."),
            Some(CertNotice::Problem(page::Notice::Ended.text())),
            signing_in,
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
        pending: pending.as_ref(),
        fail: &fail,
        ended: &ended,
    };
    let key: String = who.user.to_ascii_lowercase().chars().take(64).collect();
    let guard = match steps::guard_try(&cx, Some(&key), false) {
        Ok(guard) => guard,
        Err(response) => return response,
    };
    match inner
        .accounts
        .confirm_factor(&who.user, who.uid, code.trim())
    {
        Ok(provider) => {
            let from = shown(client_address(inner, request, conn));
            if signing_in {
                tracing::warn!(
                    "{} set up a one-time code during a sign-in from {from}",
                    who.user
                );
            } else {
                tracing::info!("{} set up a one-time code from {from}", who.user);
            }
            match actor {
                Actor::Session(_) => {
                    guard.succeeded(inner);
                    if let Some((token, _)) = &pending {
                        inner.pendings.close(token);
                    }
                    page(
                        who,
                        FactorStanding::Enrolled,
                        FactorStage::Blocked("Next time you sign in, use the code from the app."),
                        Some(CertNotice::Done("Your code is set up.")),
                        false,
                        200,
                    )
                    .with("Set-Cookie", steps::clear_pending())
                }
                Actor::Setup(_, state) => {
                    let taken = Taken {
                        method: Method::Totp,
                        user: who.user.clone(),
                        uid: who.uid,
                        groups: who.groups.clone(),
                        provider,
                        guard,
                    };
                    match steps::advance(&cx, taken, Some((actor_token(actor), state.clone()))) {
                        steps::Advance::Respond(response) => response,
                        steps::Advance::Next(_) => Response::redirect(LOGIN),
                    }
                }
            }
        }
        Err(FactorError::Wrong) => {
            tracing::info!(
                "a code confirming a setup from {} was not accepted",
                shown(client_address(inner, request, conn))
            );
            after_floor(inner, started, fail(401, page::Notice::CodeFailed, None))
        }
        Err(FactorError::NothingWaiting) => page(
            who,
            standing,
            FactorStage::Ready,
            Some(CertNotice::Problem(
                "No setup is waiting, or it timed out. Set up a code again.",
            )),
            signing_in,
            400,
        ),
        Err(why) => {
            tracing::warn!("a one-time code could not be confirmed: {}", reason(&why));
            page(
                who,
                standing,
                FactorStage::Ready,
                Some(CertNotice::Problem(page::Notice::Unavailable.text())),
                signing_in,
                503,
            )
        }
    }
}

fn actor_token(actor: &Actor) -> String {
    match actor {
        Actor::Setup(token, _) => token.clone(),
        Actor::Session(_) => String::new(),
    }
}
