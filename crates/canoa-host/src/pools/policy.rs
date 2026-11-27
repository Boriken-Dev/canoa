//! Which account of a pool takes a turn: `coqui-acp-pool`'s round robin over what [`super::gather`] says of each account. A pool
//! leaves out an account that does not list the model, that another connection holds, or whose allowance covering the model is
//! used up; among the rest it takes them in turn, at a new session and at a turn that starts once the prompt cache has lapsed, so
//! no warm cache is given up. A session whose account runs out moves to the next.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the memory shared between threads is std's"
)]

use std::sync::LazyLock;

use coqui_acp_pool::{Ask, Chosen, Member, Pool, SharedMemory, Standing, Strategy};

/// The turns of every pool of this process share one memory, so the next account is the one after the last any session took.
static MEMORY: LazyLock<SharedMemory> = LazyLock::new(SharedMemory::new);

/// The pool `name` as the shared code sees it: `accounts` in order, taken by round robin.
fn pool_of(name: &str, accounts: &[String]) -> Pool<()> {
    accounts.iter().fold(
        Pool::new(name).strategy(Strategy::RoundRobin),
        |pool, account| pool.member(Member::new(account.clone(), ())),
    )
}

/// The account of the pool `pool` (`accounts`, in its order, each with its `standings`) that takes the turn `ask` describes.
///
/// # Errors
///
/// None can take it: why each cannot, and when the soonest of those that are out of usage refills.
pub(crate) fn decide(
    pool: &str,
    accounts: &[String],
    standings: &[Standing],
    ask: &Ask<'_>,
) -> Result<Chosen, String> {
    decide_with(&MEMORY, pool, accounts, standings, ask)
}

/// What leaves an account nothing for a turn on `model` at `now`: the shared pool code's checks, applied to the one account.
pub(crate) fn blocker(
    standing: &Standing,
    model: Option<&str>,
    now: i64,
) -> Option<coqui_acp_pool::Reason> {
    let mut ask = Ask::new(now);
    if let Some(model) = model {
        ask = ask.model(model);
    }
    let pool = pool_of("account", &["account".to_owned()]);
    coqui_acp_pool::decide(
        &pool,
        &mut coqui_acp_pool::Memory::new(),
        core::slice::from_ref(standing),
        &ask,
    )
    .err()
    .and_then(|none| none.reasons.into_iter().next())
    .map(|skipped| skipped.reason)
}

/// [`decide`] on a memory of the caller's.
///
/// # Errors
///
/// As [`decide`].
pub(crate) fn decide_with(
    memory: &SharedMemory,
    pool: &str,
    accounts: &[String],
    standings: &[Standing],
    ask: &Ask<'_>,
) -> Result<Chosen, String> {
    let pool = pool_of(pool, accounts);
    memory
        .with(|memory| coqui_acp_pool::decide(&pool, memory, standings, ask))
        .map_err(|none| none.message("account"))
}
