//! Signed-in browsers and the count of failed sign-ins.

use alloc::collections::BTreeMap;
use core::net::{IpAddr, Ipv6Addr};
use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_access::Factor;
use data_encoding::BASE64URL_NOPAD;

use super::signin::Method;
use sha2::{Digest, Sha256};

/// How many random bytes a session's token holds.
const TOKEN_BYTES: usize = 32;
/// The most addresses and names whose failures are kept; the oldest go first.
const MAX_TRACKED: usize = 10_000;

/// A signed-in user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// The account's name as the system spells it; a valid user name, so it makes a unit and a path.
    pub user: String,
    /// The account's user id: whose process must be at the other end of the sockets this session is given.
    pub uid: u32,
    /// What names this session to its relays, so that ending it ends them.
    pub id: u64,
    /// When a password signed this session in; `None` for a session any other method opened.
    pub password_at: Option<Instant>,
    /// The method that signed it in; `None` for a session opened with no method named.
    pub method: Option<Method>,
    /// The fingerprint of the certificate the sign-in request carried, whatever the method: what a pin from this session may
    /// be.
    pub fingerprint: Option<String>,
    /// The steps that signed it in, which the sign-in policy is asked about again each time it is used.
    pub done: Vec<Factor>,
    /// When the one-time code that signed it in was set up, if one did: the session ends when the account's code is not that one.
    pub code_made: Option<i64>,
    /// The id of the passkey that signed it in, if one did: the session ends when the account no longer holds that passkey.
    pub passkey: Option<String>,
}

struct Kept {
    user: String,
    uid: u32,
    id: u64,
    until: Instant,
    password_at: Option<Instant>,
    method: Option<Method>,
    fingerprint: Option<String>,
    done: Vec<Factor>,
    code_made: Option<i64>,
    passkey: Option<String>,
}

/// The sessions of signed-in browsers, by the digest of the token the browser holds: the token itself is never kept.
pub struct Sessions {
    lifetime: Duration,
    limit: usize,
    per_user: usize,
    state: Mutex<(u64, HashMap<[u8; 32], Kept>)>,
}

impl core::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sessions").finish_non_exhaustive()
    }
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

impl Sessions {
    /// Sessions that last `lifetime`, `limit` at once and `per_user` for one account.
    #[must_use]
    pub fn new(lifetime: Duration, limit: usize, per_user: usize) -> Self {
        Self {
            lifetime,
            limit,
            per_user: per_user.max(1),
            state: Mutex::new((0, HashMap::new())),
        }
    }

    /// A new session for `user`, and the token the browser keeps. An account that holds its share already loses its oldest
    /// session, so one account cannot fill the table. `None` when the whole table is full of live sessions of others.
    pub fn open(&self, user: &str, uid: u32) -> Option<(String, Session)> {
        self.open_with(user, uid, None, None, Vec::new())
    }

    /// [`Sessions::open`] for a sign-in by `method`, whose request carried a certificate with `fingerprint`, if it did, and which
    /// passed the steps `done`.
    pub fn open_with(
        &self,
        user: &str,
        uid: u32,
        method: Option<Method>,
        fingerprint: Option<String>,
        done: Vec<Factor>,
    ) -> Option<(String, Session)> {
        let password_at = (method == Some(Method::Password)).then(Instant::now);
        self.open_after(
            user,
            uid,
            method,
            password_at,
            fingerprint,
            done,
            None,
            None,
        )
    }

    /// [`Sessions::open_with`] for a sign-in whose password, if it had one, was typed at `password_at`: a step before the one
    /// that completed it.
    #[allow(
        clippy::too_many_arguments,
        reason = "everything a session records of the sign-in that made it"
    )]
    pub fn open_after(
        &self,
        user: &str,
        uid: u32,
        method: Option<Method>,
        password_at: Option<Instant>,
        fingerprint: Option<String>,
        done: Vec<Factor>,
        code_made: Option<i64>,
        passkey: Option<String>,
    ) -> Option<(String, Session)> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let token = BASE64URL_NOPAD.encode(&bytes);
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (next, map) = &mut *state;
        map.retain(|_, kept| kept.until > now);
        let mine: Vec<([u8; 32], u64)> = map
            .iter()
            .filter(|(_, kept)| kept.user == user)
            .map(|(key, kept)| (*key, kept.id))
            .collect();
        if mine.len() >= self.per_user {
            let mut oldest = mine;
            oldest.sort_by_key(|(_, id)| *id);
            for (key, _) in oldest.iter().take(oldest.len() + 1 - self.per_user) {
                map.remove(key);
            }
        }
        if map.len() >= self.limit {
            return None;
        }
        *next += 1;
        let id = *next;
        map.insert(
            digest(&token),
            Kept {
                user: user.to_owned(),
                uid,
                id,
                until: now.checked_add(self.lifetime)?,
                password_at,
                method,
                fingerprint: fingerprint.clone(),
                done: done.clone(),
                code_made,
                passkey: passkey.clone(),
            },
        );
        Some((
            token,
            Session {
                user: user.to_owned(),
                uid,
                id,
                password_at,
                method,
                fingerprint,
                done,
                code_made,
                passkey,
            },
        ))
    }

    /// The session `token` names, when it is still valid.
    pub fn get(&self, token: &str) -> Option<Session> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = digest(token);
        let kept = state.1.get(&key)?;
        if kept.until <= now {
            state.1.remove(&key);
            return None;
        }
        Some(Session {
            user: kept.user.clone(),
            uid: kept.uid,
            id: kept.id,
            password_at: kept.password_at,
            method: kept.method,
            fingerprint: kept.fingerprint.clone(),
            done: kept.done.clone(),
            code_made: kept.code_made,
            passkey: kept.passkey.clone(),
        })
    }

    /// Whether the session `id` is still valid.
    pub fn alive(&self, id: u64) -> bool {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .1
            .values()
            .any(|kept| kept.id == id && kept.until > now)
    }

    /// Ends the session `token` names.
    pub fn close(&self, token: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.remove(&digest(token));
    }

    /// Ends the sessions of `user` that a certificate signed in, except those of the certificate `keep` names: for when a pin is
    /// removed or replaced, so that what it let in does not outlive it.
    pub fn close_certificates(&self, user: &str, keep: Option<&str>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| {
            let by_certificate = kept.method == Some(Method::Certificate)
                || kept
                    .done
                    .iter()
                    .any(|done| done.method == Method::Certificate.id());
            kept.user != user
                || !by_certificate
                || (keep.is_some() && kept.fingerprint.as_deref() == keep)
        });
    }

    /// Ends the sessions of `user` that a passkey signed in: the one named by `credential`, or every one when it is `None`. For
    /// when a passkey is removed, so that what it let in does not outlive it.
    pub fn close_passkey(&self, user: &str, credential: Option<&str>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| {
            kept.user != user
                || kept
                    .passkey
                    .as_deref()
                    .is_none_or(|held| credential.is_some_and(|credential| credential != held))
        });
    }

    /// Ends the session `id` names, for a relay that finds it no longer stands.
    pub fn close_id(&self, id: u64) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| kept.id != id);
    }

    /// Ends every session of `user`.
    pub fn close_user(&self, user: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| kept.user != user);
    }
}

/// What a pending step is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A sign-in that has passed a step and is asked for the next.
    SignIn,
    /// A signed-in person who is asked their factors again before they may set a code up; bound to that session.
    Enrol,
}

/// A sign-in, or a setup of a second factor, that has passed some steps and waits for the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The account the steps named, as the system spells it.
    pub user: String,
    /// Its user id: the next step must name the same one.
    pub uid: u32,
    /// What the steps are for.
    pub purpose: Purpose,
    /// For [`Purpose::Enrol`], the session that asked.
    pub session: Option<u64>,
    /// The steps passed so far.
    pub done: Vec<Factor>,
    /// The tries made so far at the steps after the first, right or wrong.
    pub tries: usize,
    /// The method of the first step, which the session it opens records.
    pub first: Option<Method>,
    /// When a password step was passed, if one was.
    pub password_at: Option<Instant>,
    /// The fingerprint of the certificate a step carried: what a pin made from the session it opens may be.
    pub fingerprint: Option<String>,
    /// The id of the passkey a step was made with, which the session it opens records.
    pub passkey: Option<String>,
}

struct KeptPending {
    pending: Pending,
    id: u64,
    until: Instant,
}

/// Whether a try at a pending step may begin.
#[derive(Debug, PartialEq, Eq)]
pub enum Tried {
    /// The try is counted.
    Allowed,
    /// The pending state is gone, expired, or has used up its tries: the person starts again.
    Ended,
}

/// The pending steps of browsers, by the digest of the token the browser holds. Short-lived, bounded and held in memory only, so
/// a restart of the portal starts everyone's pending sign-in over.
pub struct Pendings {
    lifetime: Duration,
    max_tries: usize,
    limit: usize,
    per_user: usize,
    state: Mutex<(u64, HashMap<[u8; 32], KeptPending>)>,
}

impl core::fmt::Debug for Pendings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pendings").finish_non_exhaustive()
    }
}

impl Pendings {
    /// Pending steps that last `lifetime`, allow `max_tries` tries each, `limit` at once and `per_user` for one account.
    #[must_use]
    pub fn new(lifetime: Duration, max_tries: usize, limit: usize, per_user: usize) -> Self {
        Self {
            lifetime,
            max_tries,
            limit,
            per_user: per_user.max(1),
            state: Mutex::new((0, HashMap::new())),
        }
    }

    /// Keeps `pending` for the browser that holds the token returned, ending the oldest of the same account when it holds its
    /// share already. `None` when the table is full of live ones of others.
    pub fn open(&self, pending: Pending) -> Option<String> {
        self.open_for(pending, self.lifetime)
    }

    /// [`Pendings::open`] for a pending state that lasts `lifetime` rather than the usual.
    pub fn open_for(&self, pending: Pending, lifetime: Duration) -> Option<String> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let token = BASE64URL_NOPAD.encode(&bytes);
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (next, map) = &mut *state;
        map.retain(|_, kept| kept.until > now);
        let mut mine: Vec<([u8; 32], u64)> = map
            .iter()
            .filter(|(_, kept)| kept.pending.user == pending.user)
            .map(|(key, kept)| (*key, kept.id))
            .collect();
        if mine.len() >= self.per_user {
            mine.sort_by_key(|(_, id)| *id);
            for (key, _) in mine.iter().take(mine.len() + 1 - self.per_user) {
                map.remove(key);
            }
        }
        if map.len() >= self.limit {
            return None;
        }
        *next += 1;
        let id = *next;
        map.insert(
            digest(&token),
            KeptPending {
                pending,
                id,
                until: now.checked_add(lifetime)?,
            },
        );
        Some(token)
    }

    /// The pending state `token` names, while it lives.
    pub fn get(&self, token: &str) -> Option<Pending> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = digest(token);
        let kept = state.1.get(&key)?;
        if kept.until <= now {
            state.1.remove(&key);
            return None;
        }
        Some(kept.pending.clone())
    }

    /// Counts a try at the step `token` waits for. A pending state that has had its tries, or is gone, ends and is not given
    /// another: the person starts again.
    pub fn begin_try(&self, token: &str) -> Tried {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = digest(token);
        let Some(kept) = state.1.get_mut(&key) else {
            return Tried::Ended;
        };
        if kept.until <= now || kept.pending.tries >= self.max_tries {
            state.1.remove(&key);
            return Tried::Ended;
        }
        kept.pending.tries += 1;
        Tried::Allowed
    }

    /// Replaces the steps passed of the pending state `token` names; false when it is gone.
    pub fn record(&self, token: &str, pending: &Pending) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match state.1.get_mut(&digest(token)) {
            Some(kept) => {
                // The tries stay as the store counted them: a state read before a try began would otherwise give it back.
                let tries = kept.pending.tries;
                kept.pending = pending.clone();
                kept.pending.tries = tries;
                true
            }
            None => false,
        }
    }

    /// Ends the pending state `token` names.
    pub fn close(&self, token: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.remove(&digest(token));
    }

    /// Ends every pending state of `user`.
    pub fn close_user(&self, user: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| kept.pending.user != user);
    }

    /// How long a pending state lasts.
    #[must_use]
    pub const fn lifetime(&self) -> Duration {
        self.lifetime
    }
}

/// A sign-in that finished in the Office dialog and waits for the task pane to claim it.
///
/// The dialog is a window of its own, whose cookies the task pane's frame does not share, and the only thing that crosses from
/// one to the other is a string Office carries. So a sign-in that ends in the dialog opens no session there: it leaves this, and
/// the string names it. The pane's frame claims it once, and the session opens for the frame's own cookie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handoff {
    /// The account the steps named, as the system spells it.
    pub user: String,
    /// Its user id.
    pub uid: u32,
    /// The method the session will record.
    pub method: Option<Method>,
    /// When a password step was passed, if one was.
    pub password_at: Option<Instant>,
    /// The fingerprint of the certificate a step carried.
    pub fingerprint: Option<String>,
    /// The steps that signed it in.
    pub done: Vec<Factor>,
    /// When the one-time code that signed it in was set up, if one did.
    pub code_made: Option<i64>,
    /// The id of the passkey that signed it in, if one did.
    pub passkey: Option<String>,
}

struct KeptHandoff {
    handoff: Handoff,
    id: u64,
    until: Instant,
}

/// The sign-ins waiting to be claimed, by the digest of the string the dialog sent: single use, short-lived, bounded and held in
/// memory only.
pub struct Handoffs {
    lifetime: Duration,
    limit: usize,
    per_user: usize,
    state: Mutex<(u64, HashMap<[u8; 32], KeptHandoff>)>,
}

impl core::fmt::Debug for Handoffs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handoffs").finish_non_exhaustive()
    }
}

impl Handoffs {
    /// Handoffs that wait `lifetime`, `limit` at once and `per_user` for one account.
    #[must_use]
    pub fn new(lifetime: Duration, limit: usize, per_user: usize) -> Self {
        Self {
            lifetime,
            limit,
            per_user: per_user.max(1),
            state: Mutex::new((0, HashMap::new())),
        }
    }

    /// Keeps `handoff` and answers the string that claims it, ending the oldest of the same account when it holds its share
    /// already. `None` when the table is full of live ones of others.
    pub fn open(&self, handoff: Handoff) -> Option<String> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let token = BASE64URL_NOPAD.encode(&bytes);
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (next, map) = &mut *state;
        map.retain(|_, kept| kept.until > now);
        let mut mine: Vec<([u8; 32], u64)> = map
            .iter()
            .filter(|(_, kept)| kept.handoff.user == handoff.user)
            .map(|(key, kept)| (*key, kept.id))
            .collect();
        if mine.len() >= self.per_user {
            mine.sort_by_key(|(_, id)| *id);
            for (key, _) in mine.iter().take(mine.len() + 1 - self.per_user) {
                map.remove(key);
            }
        }
        if map.len() >= self.limit {
            return None;
        }
        *next += 1;
        let id = *next;
        map.insert(
            digest(&token),
            KeptHandoff {
                handoff,
                id,
                until: now.checked_add(self.lifetime)?,
            },
        );
        Some(token)
    }

    /// The handoff `token` names, taken: it is gone whether or not it was still valid, so a string is good for one claim.
    pub fn take(&self, token: &str) -> Option<Handoff> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = state.1.remove(&digest(token))?;
        (kept.until > now).then_some(kept.handoff)
    }
}

/// A passkey challenge a browser holds, waiting for the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ceremony {
    /// The challenge's key at the provider, which the answer is checked with.
    pub challenge: String,
    /// The account the challenge was made for, when an earlier step named one: the answer must be that account's.
    pub account: Option<(String, u32)>,
}

struct KeptCeremony {
    ceremony: Ceremony,
    until: Instant,
}

/// The passkey challenges browsers hold, by the digest of the token the browser keeps in its cookie: single use, short-lived,
/// bounded and held in memory only, so that an answer is taken only from the browser the challenge was made for.
pub struct Ceremonies {
    limit: usize,
    state: Mutex<HashMap<[u8; 32], KeptCeremony>>,
}

impl core::fmt::Debug for Ceremonies {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ceremonies").finish_non_exhaustive()
    }
}

impl Ceremonies {
    /// A store of at most `limit` challenges waiting at once.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Keeps `ceremony` for `lifetime` and answers the token its browser holds. The challenge the browser held before, named by
    /// `replacing`, is dropped: one browser has one at a time. `None` when the store is full of live ones of others.
    pub fn open(
        &self,
        ceremony: Ceremony,
        lifetime: Duration,
        replacing: Option<&str>,
    ) -> Option<String> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let token = BASE64URL_NOPAD.encode(&bytes);
        let now = Instant::now();
        let mut map = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, kept| kept.until > now);
        if let Some(old) = replacing {
            map.remove(&digest(old));
        }
        if map.len() >= self.limit {
            return None;
        }
        map.insert(
            digest(&token),
            KeptCeremony {
                ceremony,
                until: now.checked_add(lifetime)?,
            },
        );
        Some(token)
    }

    /// The challenge `token` names, taken: it is gone whether or not it was still valid, so a challenge is answered once.
    pub fn take(&self, token: &str) -> Option<Ceremony> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = state.remove(&digest(token))?;
        (kept.until > now).then_some(kept.ceremony)
    }

    /// How many challenges wait, for a test to read.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        let now = Instant::now();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|kept| kept.until > now)
            .count()
    }
}

/// The failed sign-ins inside a window, by address with account, by address and by account.
pub struct Throttle {
    window: Duration,
    per_pair: usize,
    per_address: usize,
    slow_after: usize,
    slow: Duration,
    per_person: usize,
    per_day: usize,
    day: Duration,
    state: Mutex<Counts>,
}

#[derive(Default)]
struct Counts {
    next: u64,
    pairs: BTreeMap<(Subject, IpAddr), Vec<(u64, Instant)>>,
    addresses: BTreeMap<IpAddr, Vec<(u64, Instant)>>,
    users: BTreeMap<Subject, Vec<(u64, Instant)>>,
    people: BTreeMap<u32, Vec<(u64, Instant)>>,
    days: BTreeMap<u32, Vec<(u64, Instant)>>,
}

impl core::fmt::Debug for Throttle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Throttle").finish_non_exhaustive()
    }
}

/// Whose failures a try counts toward. A certificate names no account, and its count is kept apart from every account's, by
/// the address it came from, so that no name can collide with it and no address can slow another's.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    /// A password sign-in for the account of this name, lower case.
    Account(String),
    /// A certificate sign-in from this address (an IPv6 one by its /64).
    Certificate(IpAddr),
    /// A passkey sign-in from this address (an IPv6 one by its /64): a passkey names its account only once it is right.
    Passkey(IpAddr),
}

/// A try that was let through, and counts as a failure until [`Throttle::succeeded`] says it was not.
#[derive(Debug)]
#[must_use]
pub struct Attempt {
    pair: (Subject, IpAddr),
    address: IpAddr,
    user: Subject,
    id: u64,
    /// How long the try waits before the accounts are asked: the account's failures, from anywhere, make it slower.
    pub wait: Duration,
}

/// A try at a second step that was let through, and counts against the person until [`Throttle::person_succeeded`] says it did
/// not fail.
#[derive(Debug)]
#[must_use]
pub struct PersonTry {
    uid: u32,
    id: u64,
}

/// An IPv6 address is the whole /64 it sits in, since one browser has all of it.
fn key_of(address: IpAddr) -> IpAddr {
    match address {
        // An IPv4 address a dual-stack socket reports as `::ffff:a.b.c.d` is that IPv4 address.
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some() => IpAddr::V4(
            v6.to_ipv4_mapped()
                .unwrap_or(core::net::Ipv4Addr::UNSPECIFIED),
        ),
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        other => other,
    }
}

fn recent(times: &mut Vec<(u64, Instant)>, window: Duration, now: Instant) -> usize {
    times.retain(|(_, at)| now.duration_since(*at) < window);
    times.len()
}

fn trim<K: Ord + Clone>(
    map: &mut BTreeMap<K, Vec<(u64, Instant)>>,
    window: Duration,
    now: Instant,
) {
    if map.len() <= MAX_TRACKED {
        return;
    }
    map.retain(|_, times| recent(times, window, now) > 0);
    while map.len() > MAX_TRACKED {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, times)| times.last().map(|(_, at)| *at))
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
}

impl Throttle {
    /// Refuses a try after `per_pair` failures of one address for one account in `window`, or `per_address` of one address for
    /// any, and slows each try for an account by `slow` for every failure past `slow_after`, from anywhere.
    #[must_use]
    pub fn new(
        window: Duration,
        per_pair: usize,
        per_address: usize,
        slow_after: usize,
        slow: Duration,
    ) -> Self {
        Self {
            window,
            per_pair,
            per_address,
            slow_after,
            slow,
            per_person: usize::MAX,
            per_day: usize::MAX,
            day: Duration::from_secs(24 * 3600),
            state: Mutex::new(Counts::default()),
        }
    }

    /// Refuses a try at a second step for a person after `limit` failures of it, from any address, in the window.
    #[must_use]
    pub fn with_person_limit(mut self, limit: usize) -> Self {
        self.per_person = limit;
        self
    }

    /// Also refuses a try at a second step for a person after `limit` failures of it, from any address, in `day`: a guesser who
    /// keeps under the window's limit is stopped by this one.
    #[must_use]
    pub fn with_person_daily(mut self, limit: usize, day: Duration) -> Self {
        self.per_day = limit;
        self.day = day;
        self
    }

    /// Lets a try at a step after the first, for the account with user id `uid`, through, or says which limit stopped it: the
    /// window's or the day's. Counted as a failure as it is let through, like [`Throttle::begin`]; whoever the try came from, the
    /// person's count rises, and it is forgotten when its failures leave the window. A refused try is not counted, so it does
    /// not hold the window open.
    ///
    /// # Errors
    ///
    /// Which limit was reached.
    pub fn begin_person(&self, uid: u32) -> Result<PersonTry, &'static str> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let seen = state
            .people
            .get_mut(&uid)
            .map_or(0, |times| recent(times, self.window, now));
        if seen >= self.per_person {
            return Err("in the window");
        }
        let today = state
            .days
            .get_mut(&uid)
            .map_or(0, |times| recent(times, self.day, now));
        if today >= self.per_day {
            return Err("in the day");
        }
        state.next += 1;
        let id = state.next;
        state.people.entry(uid).or_default().push((id, now));
        state.days.entry(uid).or_default().push((id, now));
        trim(&mut state.people, self.window, now);
        trim(&mut state.days, self.day, now);
        Ok(PersonTry { uid, id })
    }

    /// Forgets what was counted against the account with user id `uid`: an administrator's unlock.
    pub fn clear_person(&self, uid: u32) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.people.remove(&uid);
        state.days.remove(&uid);
    }

    /// The try at a second step did not fail: its count is taken back.
    pub fn person_succeeded(&self, attempt: PersonTry) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(times) = state.people.get_mut(&attempt.uid) {
            times.retain(|(each, _)| *each != attempt.id);
        }
        if let Some(times) = state.days.get_mut(&attempt.uid) {
            times.retain(|(each, _)| *each != attempt.id);
        }
    }

    /// Lets a try from `address` for `user` through, or not. The try is counted as a failure at once, in the same step that
    /// checked the limits, so that tries made together cannot all pass the check before any is counted; a success takes its
    /// count back. Asked before the accounts are, so a refused try costs the directory nothing.
    pub fn begin(&self, address: IpAddr, user: &str) -> Option<Attempt> {
        self.begin_for(address, Subject::Account(user.to_owned()))
    }

    /// [`Throttle::begin`] for a certificate sign-in from `address`.
    pub fn begin_certificate(&self, address: IpAddr) -> Option<Attempt> {
        self.begin_for(address, Subject::Certificate(key_of(address)))
    }

    /// [`Throttle::begin`] for a passkey sign-in from `address`.
    pub fn begin_passkey(&self, address: IpAddr) -> Option<Attempt> {
        self.begin_for(address, Subject::Passkey(key_of(address)))
    }

    fn begin_for(&self, address: IpAddr, user: Subject) -> Option<Attempt> {
        let now = Instant::now();
        let address = key_of(address);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let pair_key = (user.clone(), address);
        let by_pair = state
            .pairs
            .get_mut(&pair_key)
            .map_or(0, |times| recent(times, self.window, now));
        let by_address = state
            .addresses
            .get_mut(&address)
            .map_or(0, |times| recent(times, self.window, now));
        if by_pair >= self.per_pair || by_address >= self.per_address {
            return None;
        }
        let by_user = state
            .users
            .get_mut(&user)
            .map_or(0, |times| recent(times, self.window, now));
        let over = by_user.saturating_sub(self.slow_after).min(30);
        state.next += 1;
        let id = state.next;
        state
            .pairs
            .entry(pair_key.clone())
            .or_default()
            .push((id, now));
        state.addresses.entry(address).or_default().push((id, now));
        state.users.entry(user.clone()).or_default().push((id, now));
        trim(&mut state.pairs, self.window, now);
        trim(&mut state.addresses, self.window, now);
        trim(&mut state.users, self.window, now);
        Some(Attempt {
            pair: pair_key,
            address,
            user,
            id,
            wait: self.slow * u32::try_from(over).unwrap_or(30),
        })
    }

    /// How many failures `address` has in the window, for a test to read what a try counted.
    #[cfg(test)]
    pub(super) fn failures_from(&self, address: IpAddr) -> usize {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .addresses
            .get_mut(&key_of(address))
            .map_or(0, |times| recent(times, self.window, now))
    }

    /// The try did not fail: its count is taken back, so a login that works does not use up its address's tries.
    pub fn succeeded(&self, attempt: Attempt) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Attempt {
            pair,
            address,
            user,
            id,
            ..
        } = attempt;
        if let Some(times) = state.pairs.get_mut(&pair) {
            times.retain(|(each, _)| *each != id);
        }
        if let Some(times) = state.addresses.get_mut(&address) {
            times.retain(|(each, _)| *each != id);
        }
        if let Some(times) = state.users.get_mut(&user) {
            times.retain(|(each, _)| *each != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_certificate_sessions_leaves_the_others_and_the_pin_that_stays() {
        let sessions = Sessions::new(Duration::from_secs(60), 10, 10);
        let open = |user: &str, method, fingerprint: Option<&str>| {
            sessions
                .open_with(
                    user,
                    1,
                    Some(method),
                    fingerprint.map(str::to_owned),
                    Vec::new(),
                )
                .unwrap()
                .0
        };
        let by_password = open("a", Method::Password, Some("sha256:new"));
        let old = open("a", Method::Certificate, Some("sha256:old"));
        let new = open("a", Method::Certificate, Some("sha256:new"));
        let other = open("b", Method::Certificate, Some("sha256:old"));
        sessions.close_certificates("a", Some("sha256:new"));
        assert!(sessions.get(&old).is_none());
        assert!(sessions.get(&new).is_some() && sessions.get(&by_password).is_some());
        assert!(
            sessions.get(&other).is_some(),
            "another account's is not touched"
        );
        sessions.close_certificates("a", None);
        assert!(sessions.get(&new).is_none() && sessions.get(&by_password).is_some());
    }

    #[test]
    fn closing_passkey_sessions_leaves_the_others_and_the_passkeys_that_stay() {
        let sessions = Sessions::new(Duration::from_secs(60), 10, 10);
        let open = |user: &str, passkey: Option<&str>| {
            sessions
                .open_after(
                    user,
                    1,
                    Some(Method::Passkey),
                    None,
                    None,
                    Vec::new(),
                    None,
                    passkey.map(str::to_owned),
                )
                .unwrap()
                .0
        };
        let one = open("a", Some("one"));
        let two = open("a", Some("two"));
        let by_password = open("a", None);
        let other = open("b", Some("one"));
        sessions.close_passkey("a", Some("one"));
        assert!(sessions.get(&one).is_none());
        assert!(sessions.get(&two).is_some() && sessions.get(&by_password).is_some());
        assert!(
            sessions.get(&other).is_some(),
            "another account's is not touched"
        );
        sessions.close_passkey("a", None);
        assert!(sessions.get(&two).is_none() && sessions.get(&by_password).is_some());
    }

    #[test]
    fn a_challenge_is_answered_once_by_the_browser_that_holds_its_token_and_the_store_is_bounded() {
        let ceremonies = Ceremonies::new(2);
        let made = |challenge: &str| Ceremony {
            challenge: challenge.to_owned(),
            account: None,
        };
        let life = Duration::from_secs(60);
        let first = ceremonies.open(made("one"), life, None).unwrap();
        assert!(ceremonies.take("not-a-token").is_none());
        assert_eq!(ceremonies.take(&first), Some(made("one")));
        assert!(ceremonies.take(&first).is_none(), "taken once");
        let a = ceremonies.open(made("a"), life, None).unwrap();
        let b = ceremonies.open(made("b"), life, None).unwrap();
        assert!(ceremonies.open(made("c"), life, None).is_none(), "full");
        // A browser asking again gives up the challenge it held.
        let again = ceremonies.open(made("b2"), life, Some(&b)).unwrap();
        assert!(ceremonies.take(&b).is_none());
        assert_eq!(ceremonies.take(&again), Some(made("b2")));
        assert_eq!(ceremonies.take(&a), Some(made("a")));
        let short = ceremonies
            .open(made("s"), Duration::from_millis(1), None)
            .unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(ceremonies.take(&short).is_none(), "expired");
        assert_eq!(ceremonies.waiting(), 0);
    }

    #[test]
    fn a_passkeys_failures_never_count_against_an_account_of_any_name() {
        let throttle = throttle(2, 100, 100);
        let address: IpAddr = "192.0.2.7".parse().unwrap();
        for _ in 0..2 {
            assert!(throttle.begin_passkey(address).is_some());
        }
        assert!(
            throttle.begin_passkey(address).is_none(),
            "its own count ran out"
        );
        for name in ["passkey", "certificate"] {
            assert!(throttle.begin(address, name).is_some(), "{name:?}");
        }
    }

    #[test]
    fn a_certificates_failures_never_count_against_an_account_of_any_name() {
        let throttle = throttle(2, 100, 100);
        let address: IpAddr = "192.0.2.7".parse().unwrap();
        let other: IpAddr = "192.0.2.8".parse().unwrap();
        for _ in 0..2 {
            assert!(throttle.begin_certificate(address).is_some());
        }
        for name in ["certificate", "\0certificate@192.0.2.7", "192.0.2.7"] {
            assert!(throttle.begin(address, name).is_some(), "{name:?}");
        }
        assert!(
            throttle.begin_certificate(address).is_none(),
            "the certificate's own pair is full"
        );
        assert!(
            throttle.begin_certificate(other).is_some(),
            "another address has its own"
        );
    }

    #[test]
    fn a_session_is_found_by_its_token_and_not_after_it_closes() {
        let sessions = Sessions::new(Duration::from_secs(60), 4, 2);
        let (token, session) = sessions.open("canoatesta", 1001).unwrap();
        assert_eq!(sessions.get(&token), Some(session.clone()));
        assert!(sessions.alive(session.id));
        assert_eq!(sessions.get("not-the-token"), None);
        sessions.close(&token);
        assert_eq!(sessions.get(&token), None);
        assert!(!sessions.alive(session.id));
    }

    #[test]
    fn a_session_ends_when_its_lifetime_does_and_the_limit_counts_only_live_ones() {
        let sessions = Sessions::new(Duration::from_millis(30), 1, 1);
        let (token, _) = sessions.open("a", 1).unwrap();
        assert!(sessions.open("b", 2).is_none());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(sessions.get(&token), None);
        assert!(sessions.open("b", 2).is_some());
    }

    fn handoff(user: &str) -> Handoff {
        Handoff {
            user: user.to_owned(),
            uid: 1001,
            method: Some(Method::Password),
            password_at: None,
            fingerprint: None,
            done: Vec::new(),
            code_made: None,
            passkey: None,
        }
    }

    #[test]
    fn a_handoff_is_claimed_once_and_only_by_its_own_string() {
        let handoffs = Handoffs::new(Duration::from_secs(60), 4, 2);
        let token = handoffs.open(handoff("a")).unwrap();
        assert!(token.len() >= 43);
        assert_eq!(handoffs.take("not-the-string"), None);
        assert_eq!(handoffs.take(&token), Some(handoff("a")));
        assert_eq!(
            handoffs.take(&token),
            None,
            "a string is good for one claim"
        );
    }

    #[test]
    fn a_handoff_ends_with_its_lifetime_and_an_account_keeps_only_its_share() {
        let short = Handoffs::new(Duration::from_millis(30), 4, 2);
        let token = short.open(handoff("a")).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(short.take(&token), None);

        let handoffs = Handoffs::new(Duration::from_secs(60), 3, 2);
        let other = handoffs.open(handoff("b")).unwrap();
        let first = handoffs.open(handoff("a")).unwrap();
        let second = handoffs.open(handoff("a")).unwrap();
        let third = handoffs.open(handoff("a")).unwrap();
        assert_eq!(
            handoffs.take(&first),
            None,
            "the oldest of the account went"
        );
        assert!(handoffs.take(&second).is_some());
        assert!(handoffs.take(&third).is_some());
        assert!(
            handoffs.take(&other).is_some(),
            "another account's is not touched"
        );
    }

    #[test]
    fn two_tokens_are_never_alike() {
        let sessions = Sessions::new(Duration::from_secs(60), 4, 4);
        let (one, _) = sessions.open("a", 1).unwrap();
        let (two, _) = sessions.open("a", 1).unwrap();
        assert_ne!(one, two);
        assert!(one.len() >= 43);
    }

    #[test]
    fn an_account_that_holds_its_share_loses_its_oldest_session_and_nobody_elses() {
        let sessions = Sessions::new(Duration::from_secs(60), 100, 2);
        let (other, _) = sessions.open("b", 2).unwrap();
        let (first, _) = sessions.open("a", 1).unwrap();
        let (second, _) = sessions.open("a", 1).unwrap();
        let (third, _) = sessions.open("a", 1).unwrap();
        assert_eq!(sessions.get(&first), None, "the oldest went");
        assert!(sessions.get(&second).is_some() && sessions.get(&third).is_some());
        assert!(
            sessions.get(&other).is_some(),
            "another account's is not touched"
        );
    }

    fn throttle(per_pair: usize, per_address: usize, slow_after: usize) -> Throttle {
        Throttle::new(
            Duration::from_millis(200),
            per_pair,
            per_address,
            slow_after,
            Duration::from_millis(10),
        )
    }

    #[test]
    fn a_try_is_counted_as_it_is_let_through_so_tries_made_together_cannot_all_pass() {
        let throttle = alloc::sync::Arc::new(throttle(3, 100, 100));
        let address: IpAddr = "192.0.2.1".parse().unwrap();
        let passed = alloc::sync::Arc::new(core::sync::atomic::AtomicUsize::new(0));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let (throttle, passed) = (throttle.clone(), passed.clone());
                std::thread::spawn(move || {
                    if let Some(attempt) = throttle.begin(address, "a") {
                        passed.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
                        core::mem::forget(attempt);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(passed.load(core::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn a_success_takes_its_count_back_and_a_failure_keeps_it() {
        let throttle = throttle(2, 100, 100);
        let address: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..5 {
            throttle.succeeded(throttle.begin(address, "a").unwrap());
        }
        let _kept = throttle.begin(address, "a").unwrap();
        let _kept = throttle.begin(address, "a").unwrap();
        assert!(throttle.begin(address, "a").is_none());
    }

    #[test]
    fn the_limit_is_for_an_address_and_an_account_together_and_an_account_is_only_slowed() {
        let throttle = throttle(2, 4, 1);
        let (one, two): (IpAddr, IpAddr) =
            ("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap());
        let _a = throttle.begin(one, "a").unwrap();
        let _b = throttle.begin(one, "a").unwrap();
        assert!(
            throttle.begin(one, "a").is_none(),
            "this address for this account is at its limit"
        );
        assert!(
            throttle.begin(one, "b").is_some(),
            "another account from there is not"
        );
        assert!(throttle.begin(one, "c").is_some());
        assert!(
            throttle.begin(one, "d").is_none(),
            "the address is at its own limit"
        );
        // Another address is never refused for an account, only made to wait once the account has many failures.
        let slowed = throttle.begin(two, "a").unwrap();
        assert!(slowed.wait > Duration::ZERO, "{:?}", slowed.wait);
        let fresh = throttle.begin(two, "z").unwrap();
        assert_eq!(fresh.wait, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(260));
        assert!(throttle.begin(one, "a").is_some(), "the window passed");
    }

    #[test]
    fn an_ipv4_address_a_dual_stack_socket_reports_in_ipv6_form_is_the_ipv4_address() {
        let throttle = throttle(100, 2, 100);
        let plain: IpAddr = "192.0.2.1".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        let _a = throttle.begin(plain, "a").unwrap();
        let _b = throttle.begin(mapped, "b").unwrap();
        assert!(
            throttle.begin(plain, "c").is_none(),
            "one address, counted once"
        );
        let other: IpAddr = "::ffff:192.0.2.2".parse().unwrap();
        assert!(
            throttle.begin(other, "c").is_some(),
            "another is not one with `::`"
        );
    }

    #[test]
    fn a_success_takes_back_its_own_count_and_no_other_made_in_the_same_instant() {
        let throttle = throttle(100, 100, 100);
        let address: IpAddr = "192.0.2.1".parse().unwrap();
        let (first, second) = (
            throttle.begin(address, "a").unwrap(),
            throttle.begin(address, "a").unwrap(),
        );
        throttle.succeeded(first);
        let _kept = second;
        let third = throttle.begin(address, "a").unwrap();
        // Two failures stand (the second try and the third); the limit of three lets one more and then no more.
        let guarded = self::throttle(100, 3, 100);
        let (a, b) = (
            guarded.begin(address, "a").unwrap(),
            guarded.begin(address, "a").unwrap(),
        );
        guarded.succeeded(a);
        let _c = guarded.begin(address, "a").unwrap();
        let _d = guarded.begin(address, "a").unwrap();
        assert!(guarded.begin(address, "a").is_none());
        drop((b, third));
    }

    #[test]
    fn an_ipv6_address_is_counted_by_its_64() {
        let throttle = throttle(100, 2, 100);
        let one: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let same: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        let _a = throttle.begin(one, "a").unwrap();
        let _b = throttle.begin(same, "b").unwrap();
        assert!(throttle.begin(one, "c").is_none());
        assert!(throttle.begin(other, "c").is_some());
    }

    fn pending(user: &str) -> Pending {
        Pending {
            user: user.to_owned(),
            uid: 1,
            purpose: Purpose::SignIn,
            session: None,
            done: Vec::new(),
            tries: 0,
            first: Some(Method::Password),
            password_at: None,
            fingerprint: None,
            passkey: None,
        }
    }

    #[test]
    fn a_pending_state_ends_after_its_tries_and_recording_steps_never_gives_a_try_back() {
        let pendings = Pendings::new(Duration::from_secs(60), 2, 10, 4);
        let token = pendings.open(pending("a")).unwrap();
        let stale = pendings.get(&token).unwrap();
        assert_eq!(pendings.begin_try(&token), Tried::Allowed);
        assert_eq!(pendings.begin_try(&token), Tried::Allowed);
        // A state read before the tries and written back after them does not undo them.
        assert!(pendings.record(&token, &stale));
        assert_eq!(pendings.get(&token).unwrap().tries, 2);
        assert_eq!(pendings.begin_try(&token), Tried::Ended);
        assert!(pendings.get(&token).is_none());
        assert_eq!(pendings.begin_try("not-a-token"), Tried::Ended);
    }

    #[test]
    fn a_pending_state_ends_with_its_time_and_an_account_keeps_only_its_share() {
        let short = Pendings::new(Duration::from_millis(30), 5, 10, 1);
        let token = short.open(pending("a")).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(short.get(&token).is_none());
        assert_eq!(short.begin_try(&token), Tried::Ended);
        let one = Pendings::new(Duration::from_secs(60), 5, 10, 1);
        let first = one.open(pending("a")).unwrap();
        let other = one.open(pending("b")).unwrap();
        let second = one.open(pending("a")).unwrap();
        assert!(
            one.get(&first).is_none() && one.get(&second).is_some() && one.get(&other).is_some()
        );
        one.close_user("a");
        assert!(one.get(&second).is_none() && one.get(&other).is_some());
        let full = Pendings::new(Duration::from_secs(60), 5, 1, 4);
        full.open(pending("a")).unwrap();
        assert!(
            full.open(pending("b")).is_none(),
            "the table is full of others"
        );
        // A token is long and never kept as itself.
        assert!(one.open(pending("c")).unwrap().len() >= 43);
    }

    #[test]
    fn a_persons_failures_from_any_address_stop_the_next_and_leave_the_window() {
        let throttle = throttle(100, 100, 100).with_person_limit(2);
        let one = throttle.begin_person(7).unwrap();
        let two = throttle.begin_person(7).unwrap();
        assert!(throttle.begin_person(7).is_err(), "the limit is reached");
        assert!(
            throttle.begin_person(8).is_ok(),
            "another person has their own"
        );
        // A refused try is not counted, so it does not hold the window open; a right one takes its count back.
        throttle.person_succeeded(one);
        assert!(throttle.begin_person(7).is_ok());
        drop(two);
        std::thread::sleep(Duration::from_millis(250));
        assert!(throttle.begin_person(7).is_ok(), "failures left the window");
    }
}
