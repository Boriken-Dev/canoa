//! Antigravity's quota. `agy acp` serves its own language server on loopback once a session is open, and
//! `coqui_acp_harness::antigravity` reads the account's remaining fraction and reset time for each model from it. The host decides
//! when to read; this is what a reading is: which processes to look at, which port answered last, which models the agent listed.

use alloc::string::ToString;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use canoa::usage_reader::{
    Failure, ProcessMembers, ReaderInfo, Reading, Retry, UsageReader, UsageReading,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_harness::antigravity::{AcpModel, AgyQuota, BUCKET_PREFIX, ReadError, ReadLimits};
use serde_json::Value;

/// How long after a session opens each further try waits, while the agent has no port yet.
const RETRIES_MS: [u64; 4] = [2_000, 4_000, 8_000, 16_000];
/// How long a private agent waits between tries for its port.
const PRIVATE_RETRIES_MS: [u64; 5] = [500, 1_000, 2_000, 4_000, 8_000];

/// How Antigravity's quota is asked for: once a session has been open for a while with no turn running, or when its first turn
/// ends, since the language server comes up a moment after the session does.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(UsageReader, id = "antigravity_usage", crate = ::coqui_acp_api::guatiao)]
pub struct AgyUsage;

impl UsageReader for AgyUsage {
    fn describe(&self) -> Result<ReaderInfo, ProviderError> {
        let mut info = ReaderInfo::new("antigravity", BUCKET_PREFIX);
        info.settle_ms = 10_000;
        info.retries_ms = RETRIES_MS.to_vec();
        info.cache_ms = 15_000;
        info.private_limit_ms = 40_000;
        info.private_retries_ms = PRIVATE_RETRIES_MS.to_vec();
        Ok(info)
    }

    fn open(
        &self,
        members: Object<dyn ProcessMembers>,
    ) -> Result<Object<dyn UsageReading>, ProviderError> {
        Ok(Quota::new(members).into_object())
    }
}

/// One agent's quota: the reader remembers the port that answered and the host asks again without listing any ports.
struct Quota {
    members: Object<dyn ProcessMembers>,
    reader: Mutex<AgyQuota>,
    models: Mutex<Vec<AcpModel>>,
    /// Whether the agent's port was not its own process's, so the whole tree is looked through.
    wide: AtomicBool,
}

impl Quota {
    fn new(members: Object<dyn ProcessMembers>) -> Self {
        Self {
            members,
            reader: Mutex::new(AgyQuota::new(ReadLimits::default())),
            models: Mutex::new(Vec::new()),
            wide: AtomicBool::new(false),
        }
    }

    /// `root` and the processes below it; `root` alone when the host cannot say.
    fn tree(&self, root: u32) -> Vec<u32> {
        self.members
            .members(root)
            .ok()
            .filter(|found| found.contains(&root))
            .unwrap_or_else(|| alloc::vec![root])
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl UsageReading for Quota {
    fn opened(&self, result: &str) -> Result<(), ProviderError> {
        let result: Value = serde_json::from_str(result).map_err(|_| {
            ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, "the result is not JSON")
        })?;
        let models = models_of(&result);
        if !models.is_empty() {
            *lock(&self.models) = models;
        }
        Ok(())
    }

    fn read(&self, root: u32, tree: bool) -> Result<Reading, ProviderError> {
        let models = lock(&self.models).clone();
        let mut reader = lock(&self.reader);
        // The agent keeps its port for its life: ports are listed for the first reading and after a failure.
        if let Some(buckets) = reader.read_remembered(&models) {
            return Ok(Reading::read(buckets));
        }
        let wide = tree || self.wide.load(Ordering::SeqCst);
        let mut read = reader.read(
            &if wide {
                self.tree(root)
            } else {
                alloc::vec![root]
            },
            &models,
        );
        if !wide && matches!(read, Err(ReadError::NoPort | ReadError::NotOwned)) {
            let owners = self.tree(root);
            if owners.len() > 1 {
                self.wide.store(true, Ordering::SeqCst);
                read = reader.read(&owners, &models);
            }
        }
        Ok(match read {
            Ok(buckets) => Reading::read(buckets),
            Err(error) => Reading::failed(Failure::new(error.to_string(), retry_of(&error))),
        })
    }
}

/// Whether a reading that failed for `error` is worth asking for again: while the agent has no port yet, and when a port did not answer.
const fn retry_of(error: &ReadError) -> Retry {
    match error {
        ReadError::NoPort => Retry::NotYet,
        ReadError::Io(_) => Retry::NoAnswer,
        _ => Retry::Never,
    }
}

/// The models a `session/new` answer lists: `models.availableModels[].{modelId, name}`.
fn models_of(result: &Value) -> Vec<AcpModel> {
    result["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model.get("modelId")?.as_str()?;
            let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
            Some(AcpModel::new(id, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A host that finds only the process asked about.
    struct Alone;

    impl ProcessMembers for Alone {
        fn members(&self, root: u32) -> Result<Vec<u32>, ProviderError> {
            Ok(alloc::vec![root])
        }
    }

    #[test]
    fn the_description_is_one_the_host_accepts() {
        let info = AgyUsage.describe().expect("described");
        assert_eq!(info.problem(), None);
        assert_eq!(info.id, "antigravity");
        assert_eq!(info.bucket_prefix, BUCKET_PREFIX);
        assert_eq!(info.retries_ms, RETRIES_MS);
    }

    #[test]
    fn the_models_a_session_lists_are_read_with_the_id_for_a_name_that_is_missing() {
        let result = json!({"models": {"availableModels": [
            {"modelId": "a", "name": "Alpha"},
            {"modelId": "b"},
            {"name": "no id"},
        ]}});
        let models = models_of(&result);
        assert_eq!(
            models,
            [AcpModel::new("a", "Alpha"), AcpModel::new("b", "b")]
        );
        assert!(models_of(&json!({})).is_empty());
    }

    #[test]
    fn a_result_that_is_not_json_is_refused() {
        let reading = AgyUsage.open(Alone.into_object()).expect("a reading opens");
        assert!(reading.opened("not json").is_err());
        assert!(reading.opened("{}").is_ok());
    }

    #[test]
    fn a_process_with_no_port_is_read_again_later() {
        let reading = AgyUsage.open(Alone.into_object()).expect("a reading opens");
        // No process has this id, so nothing of it listens.
        let read = reading.read(u32::MAX - 1, false).expect("answered");
        let failure = read.failure.expect("nothing to read");
        assert_eq!(failure.retry, Retry::NotYet, "{}", failure.why);
        assert!(read.buckets.is_empty());
    }
}
