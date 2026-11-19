//! The operating system's credential store as a test uses it, when the test is about that store and nothing else is. Every other
//! test of this crate keeps its credentials in memory (see [`super::KeyringStore::new`]), so these are the only ones that can leave
//! an entry behind, and each leaves none: a [`RealStore`] removes every entry its store asked for when it is dropped, whether
//! the test ended or panicked, and [`no_entry_of_this_run_is_left_in_the_operating_systems_store`] says so afterwards.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks are std's"
)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use keyring_core::Error as KeyError;

use super::{Backend, KeyringStore, patiently};

/// The entries a store asked for.
type Touched = Arc<Mutex<BTreeSet<String>>>;

/// One test at a time in the real store, so that the check at the end never sees a test half way.
static RUN: Mutex<()> = Mutex::new(());

/// Every service the real-store tests of this process used, with the entries each asked for.
static MADE: Mutex<Vec<(String, Touched)>> = Mutex::new(Vec::new());

/// What every service of this run starts with: nothing else's does, so what holds it was made by this process.
fn prefix() -> String {
    format!("dev.boriken.canoa.test.{}", std::process::id())
}

/// A service of this run in the operating system's store, emptied when it is dropped.
pub(crate) struct RealStore {
    service: String,
    touched: Touched,
    _run: MutexGuard<'static, ()>,
}

impl RealStore {
    /// A service of its own, named for this process, and the turn in the real store.
    pub(crate) fn new() -> Self {
        let run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let service = format!("{}.{}", prefix(), NEXT.fetch_add(1, Ordering::SeqCst));
        let touched = Touched::default();
        MADE.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((service.clone(), Arc::clone(&touched)));
        Self {
            service,
            touched,
            _run: run,
        }
    }

    /// A store on the operating system's, whatever the process asked for, under this service. Each call makes one that
    /// shares the service and the list of entries to remove.
    pub(crate) fn store(&self) -> KeyringStore {
        KeyringStore {
            backend: Backend::Platform,
            touched: Some(Arc::clone(&self.touched)),
            ..KeyringStore::new(self.service.clone())
        }
    }

    pub(crate) fn service(&self) -> &str {
        &self.service
    }
}

impl Drop for RealStore {
    fn drop(&mut self) {
        let left = forget(&self.service, &self.touched);
        assert!(
            left.is_empty() || std::thread::panicking(),
            "{} entries of the service {} were not removed",
            left.len(),
            self.service
        );
    }
}

/// The entries of `service` that are in the real store now, by name.
fn present(service: &str, touched: &Touched) -> Vec<String> {
    let probe = KeyringStore {
        backend: Backend::Platform,
        ..KeyringStore::new(service)
    };
    let users: Vec<String> = touched
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect();
    users
        .into_iter()
        .filter(|user| {
            probe
                .entry_raw(user)
                .is_ok_and(|entry| entry.get_secret().is_ok())
        })
        .collect()
}

/// Deletes every entry of `service` the tests asked for, and waits, bounded, for the store to agree; Windows' Credential
/// Manager shows a deleted entry for a moment longer under load. What is still shown after that is returned.
fn forget(service: &str, touched: &Touched) -> Vec<String> {
    let remover = KeyringStore {
        backend: Backend::Platform,
        ..KeyringStore::new(service)
    };
    let users: Vec<String> = touched
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect();
    for user in &users {
        if let Ok(entry) = remover.entry_raw(user) {
            let _ = patiently(|| match entry.delete_credential() {
                Err(KeyError::NoEntry) => Ok(()),
                other => other,
            });
        }
    }
    let mut left = present(service, touched);
    for _ in 0..20 {
        if left.is_empty() {
            break;
        }
        std::thread::sleep(core::time::Duration::from_millis(250));
        for user in &left {
            if let Ok(entry) = remover.entry_raw(user) {
                let _ = entry.delete_credential();
            }
        }
        left = present(service, touched);
    }
    left
}

/// The targets Windows' Credential Manager lists that hold a service of this run, by its own `cmdkey /list`, which prints names
/// and never values. Empty where there is no `cmdkey`.
fn listed_by_the_system() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let Ok(output) = std::process::Command::new("cmdkey").arg("/list").output() else {
        return Vec::new();
    };
    let needle = format!("{}.", prefix());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(&needle))
        .map(|line| line.trim().to_owned())
        .collect()
}

/// What the real-store tests of this process left in the operating system's store, and removes it.
fn left_behind() -> Vec<String> {
    let _run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
    let made: Vec<(String, Touched)> = MADE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(service, touched)| (service.clone(), Arc::clone(touched)))
        .collect();
    let mut left = Vec::new();
    for (service, touched) in &made {
        for user in present(service, touched) {
            left.push(format!("{user} of {service}"));
        }
        forget(service, touched);
    }
    left.extend(listed_by_the_system());
    left
}

/// Nothing of this run is in the operating system's store once the tests about it have ended: neither an entry a test asked for
/// nor any entry Windows lists under a service of this process. It holds whichever test ran last, since it takes the same turn.
#[test]
fn no_entry_of_this_run_is_left_in_the_operating_systems_store() {
    let left = left_behind();
    assert!(
        left.is_empty(),
        "the real-store tests left entries behind: {left:?}"
    );
}

/// The guard is what makes a failing test leave nothing: a test that panics after it stored something has its entries removed
/// as the panic unwinds.
#[test]
fn a_test_that_panics_after_storing_leaves_nothing_in_the_operating_systems_store() {
    use crate::accounts::store::SecretStore as _;

    let made: Mutex<Option<(String, Touched)>> = Mutex::new(None);
    let _outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let real = RealStore::new();
        *made.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((real.service().to_owned(), Arc::clone(&real.touched)));
        let store = real.store();
        if let Err(error) = store.put("account", &[b'x'; 5_000]) {
            assert!(
                crate::accounts::tests::store_unavailable(&error.to_string()),
                "the store refused a record: {error}"
            );
            return;
        }
        panic!("a test that fails after it stored something");
    }));
    let made = made.into_inner().unwrap_or_else(PoisonError::into_inner);
    let (service, touched) = made.expect("the test made a store");
    // The unwinding dropped the guard, which took the turn back; this asks as the next test would.
    let _run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        present(&service, &touched).is_empty(),
        "an entry of a test that panicked is still in the store"
    );
}
