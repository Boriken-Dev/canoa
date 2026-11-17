//! Watching an account's login while a connection or a call runs on it: the account provider looks at the files the harness's
//! program keeps its login in, and tells the host's keeper of a login the program renewed or lost behind its back.

use std::sync::Arc;

use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountProvider, AccountWatch, ProcessSpawner, WatchRequest,
};

/// What it takes to watch one account, ready to begin once the harness has placed the login in the account's directory.
///
/// A watch reports what it finds when it begins, and a login that is not in the directory is lost: begin it after the
/// harness's `start` or `models` has returned, never before.
pub(crate) struct Watcher {
    provider: Arc<dyn AccountProvider>,
    grant: AccountGrant,
    spawner: Object<dyn ProcessSpawner>,
    keeper: Object<dyn AccountKeeper>,
}

impl Watcher {
    pub(crate) fn new(
        provider: Arc<dyn AccountProvider>,
        grant: AccountGrant,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Self {
        Self {
            provider,
            grant,
            spawner,
            keeper,
        }
    }

    /// Begins the watch. One the provider refuses is logged, with its reason and no credential, and the account goes
    /// unwatched: a renewal the program makes is then heard at the next connection, from the stored copy's expiry.
    pub(crate) fn begin(self) -> Watching {
        let id = self.grant.id.clone();
        match self
            .provider
            .watch(WatchRequest::new(self.grant), self.spawner, self.keeper)
        {
            Ok(watch) => Watching(Some(watch)),
            Err(error) => {
                tracing::warn!(
                    "account {id} is not watched for a renewed or lost login: {}",
                    error.message()
                );
                Watching(None)
            }
        }
    }
}

/// A watch that is running. Letting go of it ends it with one last look at the account's files, so a login the program renewed
/// since the last poll is stored before the account is given back.
pub(crate) struct Watching(Option<Object<dyn AccountWatch>>);

impl Drop for Watching {
    fn drop(&mut self) {
        if let Some(watch) = self.0.take() {
            let _ = watch.stop();
        }
    }
}
