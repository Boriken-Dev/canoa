//! The steps of a sign-in after the first: the pending sign-in a browser holds between them, the factors it is asked for, and the
//! guards every try at one passes through.
//!
//! A first credential that the policy finds not enough opens a pending sign-in: the account it named, the steps passed and a
//! count of tries, kept in memory and named by a cookie of its own. Each try at a next step is let through the throttle under
//! the account's name and the person's own count of failures, so a code cannot be guessed by starting over or from many
//! addresses. A step must name the account the pending sign-in has; the session opens only when the policy says the steps
//! passed are enough.

use super::session::{Pending, PersonTry, Purpose, Tried};
use super::*;

/// The cookie that names a browser's pending sign-in or setup.
pub const PENDING: &str = "__Host-canoa-pending";

/// Why a pending state cannot be decided now.
pub enum Stale {
    /// The account is not there any more.
    Gone,
    /// The directory or the policy could not say.
    Unavailable,
}

/// The pending state of `purpose` that the request's cookie names.
pub fn pending_of(inner: &Inner, request: &Request, purpose: Purpose) -> Option<(String, Pending)> {
    let token = request.cookie(PENDING)?;
    let pending = inner.pendings.get(&token)?;
    (pending.purpose == purpose).then_some((token, pending))
}

/// The header that sets the pending cookie.
pub fn pending_cookie(inner: &Inner, token: &str, lifetime: Duration) -> String {
    let _ = inner;
    format!(
        "{PENDING}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
        lifetime.as_secs()
    )
}

/// The header that clears the pending cookie.
pub fn clear_pending() -> String {
    format!("{PENDING}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0")
}

/// What the policy offers for the steps a pending state has passed, and the account's groups as the directory has them now.
pub fn decide_pending(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    pending: &Pending,
    purpose: &str,
) -> Result<(policy::Decision, Vec<String>), Stale> {
    let groups = match inner.accounts.still(&pending.user, pending.uid) {
        Standing::Present(groups) => groups,
        Standing::Gone => return Err(Stale::Gone),
        _ => return Err(Stale::Unavailable),
    };
    let context = policy::context(
        purpose,
        client_address(inner, request, conn),
        Some(policy::person(&pending.user, pending.uid, &groups)),
        pending.done.clone(),
    );
    match inner.policy.decide(context) {
        Ok(mut decision) => {
            usable(inner, conn, &mut decision);
            Ok((decision, groups))
        }
        Err(why) => {
            tracing::warn!("the sign-in policy could not decide: {why}");
            Err(Stale::Unavailable)
        }
    }
}

/// A pending sign-in's next step, as the login page shows it.
pub struct Next {
    /// The cookie's token.
    pub token: String,
    /// The state now.
    pub pending: Pending,
    /// What the policy offers next.
    pub decision: policy::Decision,
    /// The account's groups.
    pub groups: Vec<String>,
}

/// Whether the page of a pending sign-in offers to set a code up.
pub fn setup_offer(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    pending: &Pending,
    decision: &policy::Decision,
) -> page::SetupOffer {
    if decision.offer(Method::Totp).is_none() {
        return page::SetupOffer::None;
    }
    match inner.accounts.factor_standing(&pending.user, pending.uid) {
        FactorStanding::NotEnrolled => {}
        FactorStanding::Reset => return page::SetupOffer::Administrator,
        _ => return page::SetupOffer::None,
    }
    match decide_pending(inner, request, conn, pending, purposes::ENROL_FIRST) {
        Ok((enrol, _)) if enrol.complete => page::SetupOffer::Link,
        _ => page::SetupOffer::Administrator,
    }
}

/// The failure a step ends in, as the page it was made on shows it: the status, the notice and the seconds to wait.
pub type Fail<'a> = &'a dyn Fn(u16, page::Notice, Option<u64>) -> Response;

/// What a try at a step is made against.
pub struct Cx<'a> {
    pub inner: &'a Inner,
    pub request: &'a Request,
    pub conn: &'a Conn,
    pub started: Instant,
    /// The account every step must name, when an earlier step or the session already did.
    pub account: Option<(&'a str, u32)>,
    /// Its groups, for the policy.
    pub groups: &'a [String],
    /// The pending state whose tries these count against, when there is one.
    pub pending: Option<&'a (String, Pending)>,
    /// How the page shows a failure.
    pub fail: Fail<'a>,
    /// What to answer when the pending state has ended.
    pub ended: &'a dyn Fn() -> Response,
}

/// The counts a try at a step is in: the address and account's, and, once an account is named, the person's own.
pub struct Guard {
    attempt: session::Attempt,
    person: Option<PersonTry>,
}

impl Guard {
    /// The step was right: its counts are taken back.
    pub fn succeeded(self, inner: &Inner) {
        inner.throttle.succeeded(self.attempt);
        if let Some(person) = self.person {
            inner.throttle.person_succeeded(person);
        }
    }
}

/// A step that was right.
pub struct Taken {
    pub method: Method,
    pub user: String,
    pub uid: u32,
    pub groups: Vec<String>,
    pub provider: String,
    pub guard: Guard,
}

/// The answer to a step the account did not accept, counted and slowed alike whatever it failed for.
pub fn refused(cx: &Cx<'_>, notice: page::Notice) -> Response {
    after_floor(cx.inner, cx.started, (cx.fail)(401, notice, None))
}

/// Lets a try through the guards, counting it as a failure until it proves otherwise, or answers it: the pending state has used
/// up its tries, the address and account have failed too often, or the person has.
pub fn guard_try(cx: &Cx<'_>, key: Option<&str>, certificate: bool) -> Result<Guard, Response> {
    let inner = cx.inner;
    let address = client_of(inner, cx.request, cx.conn);
    if let Some((token, _)) = cx.pending
        && inner.pendings.begin_try(token) == Tried::Ended
    {
        tracing::info!(
            "a pending sign-in from {} ended: its time or its tries are used",
            shown(client_address(inner, cx.request, cx.conn))
        );
        return Err((cx.ended)());
    }
    let begun = match (certificate, key) {
        (true, _) => inner.throttle.begin_certificate(address),
        (false, Some(key)) => inner.throttle.begin(address, key),
        (false, None) => None,
    };
    let Some(attempt) = begun else {
        tracing::info!("a sign-in from {address} was refused for its failures");
        let wait = inner.settings.failure_window.as_secs();
        return Err(after_floor(
            inner,
            cx.started,
            (cx.fail)(429, page::Notice::Throttled, Some(wait))
                .with("Retry-After", wait.to_string()),
        ));
    };
    // An administrator's unlock, written where the portal can see it, clears what was counted for the account.
    if let Some((name, uid)) = cx.account
        && let Some(stamp) = inner.accounts.unlock_stamp(name, uid)
    {
        let mut seen = inner
            .unlocked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if seen.get(&uid).is_none_or(|last| *last < stamp) {
            seen.insert(uid, stamp);
            inner.throttle.clear_person(uid);
            tracing::info!("an administrator cleared the failure counts of {name}");
        }
    }
    let person = match cx.account {
        Some((name, uid)) if certificate || key.is_some() => match inner.throttle.begin_person(uid)
        {
            Ok(person) => Some(person),
            Err(which) => {
                // Left counted: a refused try is a failed one for the address, which the person's count does not add to. The
                // first in a window is logged where an administrator looks, with the account, for what it may be: someone
                // guessing at its codes; the ones after it, which a guesser makes in numbers, are not.
                let span = if which == "in the day" {
                    inner.settings.factor_day
                } else {
                    inner.settings.failure_window
                };
                let first = {
                    let mut warned = inner.warned.lock().unwrap_or_else(PoisonError::into_inner);
                    let first = warned.get(&uid).is_none_or(|at| at.elapsed() >= span);
                    if first {
                        warned.insert(uid, Instant::now());
                    }
                    first
                };
                if first {
                    tracing::warn!(
                        "second-step tries for {name} from {address} are refused for the account's failures {which}: its codes may be guessed at"
                    );
                } else {
                    tracing::info!(
                        "a second-step try for {name} from {address} was refused for the account's failures {which}"
                    );
                }
                return Err(after_floor(
                    inner,
                    cx.started,
                    (cx.fail)(401, page::Notice::CodeFailed, None),
                ));
            }
        },
        _ => None,
    };
    if !attempt.wait.is_zero() {
        thread::sleep(attempt.wait);
    }
    Ok(Guard { attempt, person })
}

/// The method a form word names.
pub fn method_of(word: Option<&str>) -> Option<Method> {
    match word {
        None | Some("password") => Some(Method::Password),
        Some(other) => Method::from_id(other),
    }
}

/// Takes one step: `method`, which the policy offered, with what the form carries. `Err` is the answer to give instead.
pub fn take_step(
    cx: &Cx<'_>,
    decision: &policy::Decision,
    method: Method,
) -> Result<Taken, Response> {
    let inner = cx.inner;
    let (request, conn) = (cx.request, cx.conn);
    let seen = client_address(inner, request, conn);
    let address = client_of(inner, request, conn);
    let Some(offer) = decision.offer(method) else {
        tracing::info!(
            "a {} sign-in from {} is not accepted (rule {})",
            method.id(),
            shown(seen),
            decision.rule_name()
        );
        return Err(Response::refusal(403));
    };
    // A certificate try whose request carries no certificate is no guess: the browser chose none, and a person told to try again
    // would be locked out by their own clicks. It is not counted and no provider is asked.
    if method == Method::Certificate && !inner.accounts.carries_certificate(&presented(request)) {
        tracing::info!(
            "a certificate sign-in from {} carried no certificate",
            shown(seen)
        );
        return Err((cx.fail)(401, page::Notice::NoCertificate, None));
    }
    let providers = offer.providers.clone();
    let (user, secret, key) = match method {
        Method::Certificate => (String::new(), String::new(), None),
        Method::Password => {
            let (Some(user), Some(password)) = (
                form_field(&request.body, "user"),
                form_field(&request.body, "password"),
            ) else {
                return Err(Response::refusal(400));
            };
            let key: String = user.trim().to_ascii_lowercase().chars().take(64).collect();
            (user, password, Some(key))
        }
        _ => {
            let (Some((name, _)), Some(code)) = (cx.account, form_field(&request.body, "code"))
            else {
                return Err(Response::refusal(if cx.account.is_some() {
                    400
                } else {
                    403
                }));
            };
            let key: String = name.to_ascii_lowercase().chars().take(64).collect();
            (name.to_owned(), code.trim().to_owned(), Some(key))
        }
    };
    let refused_notice = match method {
        Method::Certificate => page::Notice::Certificate,
        Method::Password => page::Notice::Failed,
        _ => page::Notice::CodeFailed,
    };
    let guard = guard_try(cx, key.as_deref(), method == Method::Certificate)?;
    // Only a check that reaches the operating system's accounts waits for a free place: a code is read from a file.
    let waits_in_line = !method.needs_account();
    if waits_in_line
        && inner.signing_in.fetch_add(1, Ordering::SeqCst) >= inner.settings.max_sign_ins
    {
        inner.signing_in.fetch_sub(1, Ordering::SeqCst);
        // Nothing was asked of the accounts, so nothing counts against the address.
        guard.succeeded(inner);
        return Err((cx.fail)(503, page::Notice::Busy, None));
    }
    let result = match method {
        Method::Certificate => {
            inner
                .accounts
                .sign_in_certificate(&presented(request), address, &providers)
        }
        Method::Password => inner.accounts.sign_in(user.trim(), &secret, &providers),
        _ => match cx.account {
            Some((name, uid)) => {
                match inner
                    .accounts
                    .check_code(name, uid, method, &secret, &providers)
                {
                    CodeCheck::Valid { provider } => SignIn::Admitted {
                        user: name.to_owned(),
                        uid,
                        groups: cx.groups.to_vec(),
                        provider,
                    },
                    CodeCheck::Refused => SignIn::Refused,
                    CodeCheck::Failed(why) => SignIn::Failed(why),
                }
            }
            None => SignIn::Refused,
        },
    };
    if waits_in_line {
        inner.signing_in.fetch_sub(1, Ordering::SeqCst);
    }
    match result {
        SignIn::Admitted {
            user,
            uid,
            groups,
            provider,
        } if !crate::portal_link::system_uid(uid) && uid >= inner.settings.min_uid => {
            if cx
                .account
                .is_some_and(|(name, expected)| name != user || expected != uid)
            {
                tracing::info!(
                    "a {} step from {address} names another account than the sign-in has",
                    method.id()
                );
                return Err(refused(cx, refused_notice));
            }
            Ok(Taken {
                method,
                user,
                uid,
                groups,
                provider,
                guard,
            })
        }
        // Counted like any failed try; the log says which kind it was, and never the certificate.
        SignIn::NoCertificate => Err(certificate_failure(
            cx,
            address,
            "absent",
            page::Notice::NoCertificate,
        )),
        SignIn::Expired => Err(certificate_failure(
            cx,
            address,
            "expired",
            page::Notice::CertificateExpired,
        )),
        SignIn::Unpinned => Err(certificate_failure(
            cx,
            address,
            "pinned to no account",
            page::Notice::CertificateUnpinned,
        )),
        // An account the accounts admit that the portal does not (root, or below the lowest user id) is refused as any other.
        SignIn::Admitted { .. } | SignIn::Refused => {
            tracing::info!("a {} sign-in from {address} failed", method.id());
            Err(refused(cx, refused_notice))
        }
        SignIn::Failed(why) => {
            tracing::warn!("the accounts could not be asked: {why}");
            Err(after_floor(
                inner,
                cx.started,
                (cx.fail)(503, page::Notice::Unavailable, None),
            ))
        }
    }
}

/// The answer to a certificate try that failed in a way the person can act on; the log says which, never the certificate.
fn certificate_failure(cx: &Cx<'_>, address: IpAddr, why: &str, notice: page::Notice) -> Response {
    tracing::info!("a certificate sign-in from {address} failed: the certificate is {why}");
    after_floor(cx.inner, cx.started, (cx.fail)(401, notice, None))
}

/// `pending` with the step `taken` added: the steps passed, the first method, when a password was typed, and the certificate the
/// request carried, which is what a pin made from the session this opens may be.
pub fn record_step(
    inner: &Inner,
    request: &Request,
    conn: &Conn,
    pending: &mut Pending,
    taken: &Taken,
) {
    let factor = policy::factor(taken.method, &taken.provider, taken.uid);
    pending.done.retain(|done| done.method != taken.method.id());
    pending.done.push(factor);
    pending.first.get_or_insert(taken.method);
    let carried = || {
        if conn.from_proxy() {
            inner
                .accounts
                .certificate_status(&taken.user, taken.uid, &presented(request))
                .and_then(|status| status.presented)
                .map(|(cert, _)| cert.fingerprint)
        } else {
            None
        }
    };
    match taken.method {
        Method::Password => {
            pending.password_at = Some(Instant::now());
            pending.fingerprint = carried();
        }
        Method::Certificate if pending.password_at.is_none() => pending.fingerprint = carried(),
        _ => {}
    }
}

/// What a step led to.
pub enum Advance {
    /// An answer to give.
    Respond(Response),
    /// The sign-in goes on: the steps it still needs.
    Next(Next),
}

/// Decides again with the step `taken` added to `pending`: the session opens when the steps are enough, the sign-in waits for
/// more when the policy offers more, and a sign-in the policy offers nothing more is refused as a wrong password is.
pub fn advance(cx: &Cx<'_>, taken: Taken, pending: Option<(String, Pending)>) -> Advance {
    let inner = cx.inner;
    let (request, conn) = (cx.request, cx.conn);
    let seen = client_address(inner, request, conn);
    let address = client_of(inner, request, conn);
    let (token, mut state) = match pending {
        Some((token, state)) => (Some(token), state),
        None => (
            None,
            Pending {
                user: taken.user.clone(),
                uid: taken.uid,
                purpose: Purpose::SignIn,
                session: None,
                done: Vec::new(),
                tries: 0,
                first: None,
                password_at: None,
                fingerprint: None,
            },
        ),
    };
    record_step(inner, request, conn, &mut state, &taken);
    let refused_notice = match taken.method {
        Method::Certificate => page::Notice::Certificate,
        Method::Password => page::Notice::Failed,
        _ => page::Notice::CodeFailed,
    };
    let decided = inner.policy.decide(policy::context(
        purposes::SIGN_IN,
        seen,
        Some(policy::person(&taken.user, taken.uid, &taken.groups)),
        state.done.clone(),
    ));
    // The steps are not enough and nothing more is asked, or the policy could not say: the sign-in ends, answered, counted and
    // slowed as a wrong password is, so nothing tells the two apart.
    let end = |token: &Option<String>| {
        if let Some(token) = token {
            inner.pendings.close(token);
        }
    };
    let mut decision = match decided {
        Ok(decision) => decision,
        Err(why) => {
            tracing::warn!("the sign-in policy could not decide: {why}");
            end(&token);
            return Advance::Respond(refused(cx, refused_notice));
        }
    };
    if !decision.complete {
        usable(inner, conn, &mut decision);
        if decision.offers.is_empty() {
            tracing::info!(
                "a {} sign-in from {} is not enough (rule {})",
                taken.method.id(),
                shown(seen),
                decision.rule_name()
            );
            end(&token);
            return Advance::Respond(refused(cx, refused_notice));
        }
        let token = match token {
            Some(token) => {
                inner.pendings.record(&token, &state);
                token
            }
            None => match inner.pendings.open(state.clone()) {
                Some(token) => token,
                None => {
                    taken.guard.succeeded(inner);
                    return Advance::Respond((cx.fail)(503, page::Notice::Busy, None));
                }
            },
        };
        taken.guard.succeeded(inner);
        return Advance::Next(Next {
            token,
            pending: state,
            decision,
            groups: taken.groups,
        });
    }
    // Enough: the session opens with the steps it was made of.
    end(&token);
    let method = if state.password_at.is_some() {
        Some(Method::Password)
    } else {
        state.first
    };
    // A session made with a code keeps which code it was, so that a code taken away, or set up again, does not carry it.
    let coded = state
        .done
        .iter()
        .any(|done| done.method == Method::Totp.id() || done.method == Method::RecoveryCode.id());
    let code_made = if coded {
        match inner.accounts.factor_record(&taken.user, taken.uid) {
            (FactorStanding::Unknown, _) => {
                // The time to remember cannot be read: the sign-in is not completed on a guess.
                taken.guard.succeeded(inner);
                return Advance::Respond((cx.fail)(503, page::Notice::Unavailable, None));
            }
            (_, made) => made,
        }
    } else {
        None
    };
    let Some((session_token, _)) = inner.sessions.open_after(
        &taken.user,
        taken.uid,
        method,
        state.password_at,
        state.fingerprint.clone(),
        state.done.clone(),
        code_made,
    ) else {
        taken.guard.succeeded(inner);
        return Advance::Respond((cx.fail)(503, page::Notice::Busy, None));
    };
    taken.guard.succeeded(inner);
    let ways: Vec<&str> = state.done.iter().map(|done| done.method.as_str()).collect();
    tracing::info!(
        "{} signed in from {address} by {}",
        taken.user,
        ways.join(" and ")
    );
    // A person who signed in by password with a certificate their account does not use yet is offered to pin it.
    let offered = state.password_at.is_some()
        && conn.from_proxy()
        && inner
            .accounts
            .certificate_status(&taken.user, taken.uid, &presented(request))
            .and_then(|status| status.presented)
            .is_some_and(|(cert, pin)| pin == PinState::Unpinned && !cert.expired);
    Advance::Respond(
        Response::redirect(if offered { CERTIFICATE } else { "/" })
            .with(
                "Set-Cookie",
                format!(
                    "{COOKIE}={session_token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}",
                    inner.settings.session.as_secs()
                ),
            )
            .with("Set-Cookie", clear_pending()),
    )
}

/// `POST /portal/login`: a step of a sign-in, the first or a next one.
pub fn sign_in(inner: &Inner, request: &Request, conn: &Conn, started: Instant) -> Response {
    if !from_own_page(request, conn) {
        return Response::refusal(403);
    }
    if !request
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"))
    {
        return Response::refusal(400);
    }
    let word = form_field(&request.body, "method");
    if word.as_deref() == Some("cancel") {
        if let Some(token) = request.cookie(PENDING) {
            inner.pendings.close(&token);
        }
        return Response::redirect(LOGIN).with("Set-Cookie", clear_pending());
    }
    let Some(mut method) = method_of(word.as_deref()) else {
        return Response::refusal(400);
    };
    let pending = pending_of(inner, request, Purpose::SignIn);
    let fail =
        |status, notice, retry| login_response(inner, request, conn, status, Some(notice), retry);
    // Decided on this request's own, before the throttle counts it and before any provider is asked: a method the policy does
    // not offer from here costs the directory nothing and counts against no one.
    let (decision, groups) = match &pending {
        None => match offer_for(inner, request, conn) {
            Some(decision) => (decision, Vec::new()),
            None => return fail(503, page::Notice::Unavailable, None),
        },
        Some((token, state)) => {
            match decide_pending(inner, request, conn, state, purposes::SIGN_IN) {
                Ok((decision, groups)) if !decision.complete && !decision.offers.is_empty() => {
                    (decision, groups)
                }
                Ok(_) | Err(Stale::Gone) => {
                    inner.pendings.close(token);
                    return fail(401, page::Notice::Ended, None)
                        .with("Set-Cookie", clear_pending());
                }
                Err(Stale::Unavailable) => return fail(503, page::Notice::Unavailable, None),
            }
        }
    };
    let mut progress = (pending, decision, groups);
    let mut cookie: Option<String> = None;
    let mut immediate = false;
    loop {
        let owned = progress.0.clone();
        let (pending, decision, groups) = &progress;
        let ended = || fail(401, page::Notice::Ended, None).with("Set-Cookie", clear_pending());
        let cx = Cx {
            inner,
            request,
            conn,
            started,
            account: pending
                .as_ref()
                .map(|(_, state)| (state.user.as_str(), state.uid)),
            groups,
            pending: pending.as_ref(),
            fail: &fail,
            ended: &ended,
        };
        let taken = match take_step(&cx, decision, method) {
            Ok(taken) => taken,
            Err(response) => {
                return match &cookie {
                    Some(cookie) => response.with("Set-Cookie", cookie.clone()),
                    None => response,
                };
            }
        };
        match advance(&cx, taken, owned) {
            Advance::Respond(response) => return response,
            Advance::Next(next) => {
                let set = pending_cookie(inner, &next.token, inner.pendings.lifetime());
                // The browser that sent a password with a certificate that is pinned to this account is not asked to click for it. A
                // certificate that is not this account's is no guess to count against the person, so it is not tried: they are
                // asked, and choose.
                if !immediate
                    && method != Method::Certificate
                    && conn.from_proxy()
                    && next.decision.offer(Method::Certificate).is_some()
                    && inner.accounts.carries_certificate(&presented(request))
                    && inner
                        .accounts
                        .certificate_status(
                            &next.pending.user,
                            next.pending.uid,
                            &presented(request),
                        )
                        .and_then(|status| status.presented)
                        .is_some_and(|(cert, pin)| pin == PinState::Yours && !cert.expired)
                {
                    immediate = true;
                    method = Method::Certificate;
                    cookie = Some(set);
                    progress = (Some((next.token, next.pending)), next.decision, next.groups);
                    continue;
                }
                return Response::redirect(LOGIN).with("Set-Cookie", set);
            }
        }
    }
}
