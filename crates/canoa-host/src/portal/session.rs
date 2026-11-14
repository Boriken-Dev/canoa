//! Signed-in browsers and the count of failed sign-ins.

use alloc::collections::BTreeMap;
use core::net::IpAddr;
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
}

struct Kept {
    user: String,
    uid: u32,
    id: u64,
    until: Instant,
}

/// The sessions of signed-in browsers, by the digest of the token the browser holds: the token itself is never kept.
pub struct Sessions {
    lifetime: Duration,
    limit: usize,
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
    /// Sessions that last `lifetime`, `limit` at once.
    #[must_use]
    pub fn new(lifetime: Duration, limit: usize) -> Self {
        Self {
            lifetime,
            limit,
            state: Mutex::new((0, HashMap::new())),
        }
    }

    /// A new session for `user`, and the token the browser keeps. `None` when the limit is reached and nothing has expired.
    pub fn open(&self, user: &str, uid: u32) -> Option<(String, Session)> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let token = BASE64URL_NOPAD.encode(&bytes);
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (next, map) = &mut *state;
        if map.len() >= self.limit {
            map.retain(|_, kept| kept.until > now);
            if map.len() >= self.limit {
                return None;
            }
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
            },
        );
        Some((
            token,
            Session {
                user: user.to_owned(),
                uid,
                id,
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
}

/// The failed sign-ins inside a window, by address and by name.
pub struct Throttle {
    window: Duration,
    per_address: usize,
    per_user: usize,
    state: Mutex<Counts>,
}

#[derive(Default)]
struct Counts {
    addresses: BTreeMap<IpAddr, Vec<Instant>>,
    users: BTreeMap<String, Vec<Instant>>,
}

impl core::fmt::Debug for Throttle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Throttle").finish_non_exhaustive()
    }
}

fn recent(times: &mut Vec<Instant>, window: Duration, now: Instant) -> usize {
    times.retain(|at| now.duration_since(*at) < window);
    times.len()
}

fn trim<K: Ord + Clone>(map: &mut BTreeMap<K, Vec<Instant>>, window: Duration, now: Instant) {
    if map.len() <= MAX_TRACKED {
        return;
    }
    map.retain(|_, times| recent(times, window, now) > 0);
    while map.len() > MAX_TRACKED {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, times)| times.last().copied())
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
}

impl Throttle {
    /// Refuses a sign-in from an address after `per_address` failures in `window`, and for a name after `per_user`.
    #[must_use]
    pub fn new(window: Duration, per_address: usize, per_user: usize) -> Self {
        Self {
            window,
            per_address,
            per_user,
            state: Mutex::new(Counts::default()),
        }
    }

    /// Whether a sign-in from `address` for `user` may be tried now. Asked before the system's accounts are, so a refused
    /// attempt costs the directory nothing, and can lock nobody out there.
    pub fn allows(&self, address: IpAddr, user: &str) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let by_address = state
            .addresses
            .get_mut(&address)
            .is_none_or(|times| recent(times, self.window, now) < self.per_address);
        let by_user = state
            .users
            .get_mut(user)
            .is_none_or(|times| recent(times, self.window, now) < self.per_user);
        by_address && by_user
    }

    /// Counts a failed sign-in. Neither count is cleared by a success, so a login that works cannot buy more guesses.
    pub fn failed(&self, address: IpAddr, user: &str) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.addresses.entry(address).or_default().push(now);
        state.users.entry(user.to_owned()).or_default().push(now);
        trim(&mut state.addresses, self.window, now);
        trim(&mut state.users, self.window, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_found_by_its_token_and_not_after_it_closes() {
        let sessions = Sessions::new(Duration::from_secs(60), 4);
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
        let sessions = Sessions::new(Duration::from_millis(30), 1);
        let (token, _) = sessions.open("a", 1).unwrap();
        assert!(sessions.open("b", 2).is_none());
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(sessions.get(&token), None);
        assert!(sessions.open("b", 2).is_some());
    }

    #[test]
    fn two_tokens_are_never_alike() {
        let sessions = Sessions::new(Duration::from_secs(60), 4);
        let (one, _) = sessions.open("a", 1).unwrap();
        let (two, _) = sessions.open("a", 1).unwrap();
        assert_ne!(one, two);
        assert!(one.len() >= 43);
    }

    #[test]
    fn failures_by_address_and_by_name_stop_a_try_until_the_window_passes() {
        let throttle = Throttle::new(Duration::from_millis(80), 2, 2);
        let one: IpAddr = "192.0.2.1".parse().unwrap();
        let two: IpAddr = "192.0.2.2".parse().unwrap();
        assert!(throttle.allows(one, "a"));
        throttle.failed(one, "a");
        throttle.failed(one, "b");
        assert!(!throttle.allows(one, "c"), "the address is at its limit");
        assert!(throttle.allows(two, "c"), "another address is not");
        throttle.failed(two, "a");
        assert!(!throttle.allows(two, "a"), "the name is at its limit");
        std::thread::sleep(Duration::from_millis(120));
        assert!(throttle.allows(one, "a"));
    }
}
