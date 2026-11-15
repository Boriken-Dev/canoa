//! Signed-in browsers and the count of failed sign-ins.

use alloc::collections::BTreeMap;
use core::net::{IpAddr, Ipv6Addr};
use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use data_encoding::BASE64URL_NOPAD;
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
}

struct Kept {
    user: String,
    uid: u32,
    id: u64,
    until: Instant,
    password_at: Option<Instant>,
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
        self.open_by(user, uid, false)
    }

    /// [`Sessions::open`] for a sign-in that `password` says was, or was not, a password's.
    pub fn open_by(&self, user: &str, uid: u32, password: bool) -> Option<(String, Session)> {
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
                password_at: password.then_some(now),
            },
        );
        Some((
            token,
            Session {
                user: user.to_owned(),
                uid,
                id,
                password_at: password.then_some(now),
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

    /// Ends every session of `user`.
    pub fn close_user(&self, user: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1.retain(|_, kept| kept.user != user);
    }
}

/// The failed sign-ins inside a window, by address with account, by address and by account.
pub struct Throttle {
    window: Duration,
    per_pair: usize,
    per_address: usize,
    slow_after: usize,
    slow: Duration,
    state: Mutex<Counts>,
}

#[derive(Default)]
struct Counts {
    next: u64,
    pairs: BTreeMap<(String, IpAddr), Vec<(u64, Instant)>>,
    addresses: BTreeMap<IpAddr, Vec<(u64, Instant)>>,
    users: BTreeMap<String, Vec<(u64, Instant)>>,
}

impl core::fmt::Debug for Throttle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Throttle").finish_non_exhaustive()
    }
}

/// A try that was let through, and counts as a failure until [`Throttle::succeeded`] says it was not.
#[derive(Debug)]
#[must_use]
pub struct Attempt {
    pair: (String, IpAddr),
    address: IpAddr,
    user: String,
    id: u64,
    /// How long the try waits before the accounts are asked: the account's failures, from anywhere, make it slower.
    pub wait: Duration,
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
            state: Mutex::new(Counts::default()),
        }
    }

    /// Lets a try from `address` for `user` through, or not. The try is counted as a failure at once, in the same step that
    /// checked the limits, so that tries made together cannot all pass the check before any is counted; a success takes its
    /// count back. Asked before the accounts are, so a refused try costs the directory nothing.
    pub fn begin(&self, address: IpAddr, user: &str) -> Option<Attempt> {
        let now = Instant::now();
        let address = key_of(address);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let pair_key = (user.to_owned(), address);
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
            .get_mut(user)
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
        state
            .users
            .entry(user.to_owned())
            .or_default()
            .push((id, now));
        trim(&mut state.pairs, self.window, now);
        trim(&mut state.addresses, self.window, now);
        trim(&mut state.users, self.window, now);
        Some(Attempt {
            pair: pair_key,
            address,
            user: user.to_owned(),
            id,
            wait: self.slow * u32::try_from(over).unwrap_or(30),
        })
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
}
