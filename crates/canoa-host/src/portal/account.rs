//! `GET /portal/account`: which of the portal's own pages the signed-in person may open now, and what stands in each.
//!
//! The answer is built from the decisions the pages themselves make (the sign-in policy, for the purpose `enrol`, and the
//! account's standing), so it cannot disagree with what a page then does. A page the portal does not have is left out. A page
//! that offers nothing to do now carries `allowed: false` and the `reason`.

use super::signin::FactorStanding;
use super::*;

/// `GET /portal/account`: `401` with no session, or one that ended.
pub fn show(inner: &Inner, request: &Request, conn: &Conn) -> Response {
    let Some(session) = authed(inner, request) else {
        return Response::refusal(401);
    };
    if let Err(refusal) = resume(inner, request, conn, &session) {
        return refusal;
    }
    let mut pages = serde_json::Map::new();
    pages.insert("user".to_owned(), session.user.clone().into());
    match factor::availability(inner, request, conn, &session) {
        Ok(Some((standing, reason))) => {
            pages.insert(
                "code".to_owned(),
                page_of(
                    FACTOR,
                    reason,
                    serde_json::json!({"set_up": matches!(standing, FactorStanding::Enrolled | FactorStanding::Disabled)}),
                ),
            );
        }
        Ok(None) => {}
        Err(refusal) => return refusal,
    }
    match passkeys::availability(inner, request, conn, &session) {
        Ok(Some(held)) => {
            // A person who holds passkeys can always open the page to name or remove them; adding is what the policy decides.
            let reason = if held.count.is_some_and(|count| count > 0) {
                None
            } else {
                held.adding
            };
            pages.insert(
                "passkeys".to_owned(),
                page_of(PASSKEYS, reason, serde_json::json!({"count": held.count})),
            );
        }
        Ok(None) => {}
        Err(refusal) => return refusal,
    }
    let headers = if conn.from_proxy() {
        presented(request)
    } else {
        Vec::new()
    };
    if let Some(status) = inner
        .accounts
        .certificate_status(&session.user, session.uid, &headers)
    {
        let reason = if status.pinned || status.presented.is_some() {
            None
        } else {
            Some(
                "This connection carries no certificate: open the portal where the browser offers one.",
            )
        };
        pages.insert(
            "certificate".to_owned(),
            page_of(
                CERTIFICATE,
                reason,
                serde_json::json!({"pinned": status.pinned, "disabled": status.disabled}),
            ),
        );
    }
    pages.insert(
        "sign_out".to_owned(),
        serde_json::json!({"path": LOGOUT, "allowed": true}),
    );
    Response::json(200, &serde_json::Value::Object(pages))
        .with("Cache-Control", "no-store".to_owned())
}

/// One page's entry: its path, whether it is open now, why not, and what stands in it.
fn page_of(path: &str, reason: Option<&str>, state: serde_json::Value) -> serde_json::Value {
    let mut entry = state;
    entry["path"] = path.into();
    entry["allowed"] = reason.is_none().into();
    if let Some(reason) = reason {
        entry["reason"] = reason.into();
    }
    entry
}

/// Which of the portal's own pages this portal has for the account, for the links at the foot of each.
pub(super) fn links(inner: &Inner, user: &str, uid: u32) -> page::Links {
    page::Links {
        code: inner.accounts.factor_standing(user, uid) != FactorStanding::Unsupported,
        passkeys: !matches!(
            inner.accounts.passkeys(user, uid),
            Err(signin::PasskeyError::Unsupported)
        ),
        certificate: inner.accounts.certificate_status(user, uid, &[]).is_some(),
    }
}
