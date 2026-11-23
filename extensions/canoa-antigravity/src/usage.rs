//! Antigravity's quota, read two ways by `coqui_acp_harness::antigravity`. From the login Antigravity stores and Google's
//! service, through the one reader this process keeps for the entry's settings, which says who is signed in, the plan and the
//! groups of models with the weekly and five-hour window of each, as `agy`'s `/usage` lists them, with nothing started. From
//! the language server `agy acp` serves on loopback, which gives each model's remaining fraction and reset time and no groups:
//! in a session, when there is no stored login to read. The host decides when to read; this is what a reading is: which
//! processes to look at, which port answered last, which models the agent listed.

use alloc::string::ToString;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use canoa::usage_reader::{
    AccountDetails, Failure, ProcessMembers, ReaderInfo, Reading, Retry, UsageReader, UsageReading,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_harness::antigravity::{
    AcpModel, AgyAccount, AgyQuota, BUCKET_PREFIX, LoginReading, ReadError, ReadLimits,
};
use serde_json::Value;

use crate::login::{AgySettings, reader};

/// Why a reading of the stored login gave nothing: there is none.
pub const NOT_SIGNED_IN: &str = "Antigravity is not signed in: add its account to sign in";

/// How long after a session opens each further try waits, while the agent has no port yet.
const RETRIES_MS: [u64; 4] = [2_000, 4_000, 8_000, 16_000];
/// How long a private agent waits between tries for its port.
const PRIVATE_RETRIES_MS: [u64; 5] = [500, 1_000, 2_000, 4_000, 8_000];

/// How Antigravity's quota is asked for: from its stored login whenever no session is open, and in a session once it has been
/// open for a while with no turn running, or when its first turn ends, since the language server comes up a moment after the
/// session does. It is configured as the account provider is, from the entry's `account_settings`.
#[derive(Default, Debug, Clone, coqui_acp_api::guatiao::Provider)]
#[provider(
    UsageReader,
    id = "antigravity_usage",
    config = AgySettings,
    new = AgyUsage::default,
    crate = ::coqui_acp_api::guatiao
)]
pub struct AgyUsage {
    settings: AgySettings,
}

impl From<AgySettings> for AgyUsage {
    fn from(settings: AgySettings) -> Self {
        Self { settings }
    }
}

impl UsageReader for AgyUsage {
    fn describe(&self) -> Result<ReaderInfo, ProviderError> {
        let mut info = ReaderInfo::new("antigravity", BUCKET_PREFIX);
        info.settle_ms = 10_000;
        info.retries_ms = RETRIES_MS.to_vec();
        info.cache_ms = 15_000;
        info.private_limit_ms = 40_000;
        info.private_retries_ms = PRIVATE_RETRIES_MS.to_vec();
        info.stored = true;
        Ok(info)
    }

    fn open(
        &self,
        members: Object<dyn ProcessMembers>,
    ) -> Result<Object<dyn UsageReading>, ProviderError> {
        Ok(Quota::new(members, self.settings.clone()).into_object())
    }
}

/// One agent's quota: the reader remembers the port that answered and the host asks again without listing any ports.
struct Quota {
    members: Object<dyn ProcessMembers>,
    /// How the stored login is read.
    settings: AgySettings,
    reader: Mutex<AgyQuota>,
    models: Mutex<Vec<AcpModel>>,
    /// Whether the agent's port was not its own process's, so the whole tree is looked through.
    wide: AtomicBool,
}

impl Quota {
    fn new(members: Object<dyn ProcessMembers>, settings: AgySettings) -> Self {
        Self {
            members,
            settings,
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

    /// The account and quota from the stored login, with no process. No login, or one that cannot be read, is final; a
    /// service that did not answer is worth asking again, and the account is said all the same.
    fn stored(&self, models: &[AcpModel]) -> Reading {
        let shared = reader(&self.settings);
        let read = crate::login::lock(&shared).read(models);
        match read {
            Ok(LoginReading::SignedIn(signed)) => {
                let account = account_details(&signed.account);
                match signed.usage {
                    Ok(buckets) => Reading::read(buckets).with_account(account),
                    Err(error) => Reading::failed(Failure::new(
                        alloc::format!("Antigravity's quota was not read: {error}"),
                        Retry::NoAnswer,
                    ))
                    .with_account(account),
                }
            }
            Ok(_) => Reading::failed(Failure::new(NOT_SIGNED_IN, Retry::Never)),
            Err(error) => Reading::failed(Failure::new(
                alloc::format!("Antigravity's login was not read: {error}"),
                Retry::Never,
            )),
        }
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
        let stored = self.stored(&models);
        if root == 0 || (stored.failure.is_none() && !stored.buckets.is_empty()) {
            return Ok(stored);
        }
        let mut reader = lock(&self.reader);
        // The agent keeps its port for its life: ports are listed for the first reading and after a failure.
        if let Some(buckets) = reader.read_remembered(&models) {
            return Ok(Reading::read(buckets).with_account(details(&reader)));
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
            Ok(buckets) => Reading::read(buckets).with_account(details(&reader)),
            Err(error) => Reading::failed(Failure::new(error.to_string(), retry_of(&error))),
        })
    }
}

/// Who the reader's last answer said is signed in: the email, else the name, and the plan.
fn details(reader: &AgyQuota) -> AccountDetails {
    reader.account().map(account_details).unwrap_or_default()
}

fn account_details(account: &AgyAccount) -> AccountDetails {
    AccountDetails::new(
        account.email.clone().or_else(|| account.name.clone()),
        account.plan.clone(),
    )
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
    use coqui_acp_api::usage::{Coverage, Status as BucketStatus};
    use serde_json::json;

    /// A host that finds only the process asked about.
    struct Alone;

    impl ProcessMembers for Alone {
        fn members(&self, root: u32) -> Result<Vec<u32>, ProviderError> {
            Ok(alloc::vec![root])
        }
    }

    /// A reader whose stored login is looked for only in `home`, never in the keyring.
    fn isolated(home: &std::path::Path) -> AgyUsage {
        AgyUsage::from(AgySettings {
            home: Some(home.display().to_string()),
            keyring: false,
            ..AgySettings::default()
        })
    }

    /// A loopback stand-in for Google's token endpoint and the Cloud Code service: answers the userinfo, the plan and the
    /// quota summary of one account, and refuses everything else. It ends when the test drops it.
    struct Google {
        address: std::net::SocketAddr,
        stop: std::sync::Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    const SUMMARY: &str = r#"{"groups": [
        {"displayName": "Gemini Models",
         "description": "Models within this group: Gemini Alpha, Gemini Beta",
         "buckets": [
           {"bucketId": "g-weekly", "displayName": "Weekly Limit Remaining", "window": "weekly",
            "remainingFraction": 0.25, "resetTime": "2030-01-08T12:00:00Z"},
           {"bucketId": "g-5h", "displayName": "Five Hour Limit Remaining", "window": "5h",
            "remainingFraction": 1, "resetTime": "2030-01-01T17:00:00Z"}]},
        {"displayName": "Claude and GPT models",
         "description": "Models within this group: Claude Gamma",
         "buckets": [
           {"bucketId": "c-weekly", "displayName": "Weekly Limit Remaining", "window": "weekly",
            "resetTime": "2030-01-09T00:00:00Z"},
           {"bucketId": "c-5h", "displayName": "Five Hour Limit Remaining", "window": "5h",
            "disabled": true, "remainingFraction": 1, "resetTime": "2030-01-01T17:00:00Z"}]}]}"#;

    impl Google {
        fn start() -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
            let address = listener.local_addr().expect("its address");
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let stopping = std::sync::Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap_or(0) == 0 {
                            break;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split(' ').nth(1).unwrap_or("").to_owned();
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0_u8; length];
                    let _ = stream.read_exact(&mut body);
                    let (status, answer) = match path.as_str() {
                        "/userinfo" => (200, r#"{"email": "person@example.test"}"#.to_owned()),
                        "/v1internal:loadCodeAssist" => (200, "{}".to_owned()),
                        "/v1internal:retrieveUserQuotaSummary" => (200, SUMMARY.to_owned()),
                        _ => (404, "{}".to_owned()),
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{answer}",
                        answer.len()
                    );
                }
            });
            Self {
                address,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Google {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = std::net::TcpStream::connect(self.address);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// A reader of a login kept in `home`, asking `google` and nothing else.
    fn signed_in(home: &std::path::Path, google: &Google) -> AgyUsage {
        let gemini = home.join(".gemini");
        std::fs::create_dir_all(&gemini).expect("a home");
        std::fs::write(
            gemini.join("jetski-standalone-oauth-token"),
            r#"{"token": {"access_token": "stored", "refresh_token": "kept", "expiry": "2099-01-01T00:00:00Z"},
                "auth_method": "consumer", "id_token_email": "person@example.test", "project_id": "p",
                "user_tier": "t", "tier_display_name": "Plan"}"#,
        )
        .expect("a login");
        let base = alloc::format!("http://{}", google.address);
        AgyUsage::from(AgySettings {
            home: Some(home.display().to_string()),
            keyring: false,
            userinfo_url: alloc::format!("{base}/userinfo"),
            service: Some(base),
            ..AgySettings::default()
        })
    }

    #[test]
    fn the_stored_login_reads_the_groups_of_models_with_their_windows() {
        let home = std::env::temp_dir().join(format!("canoa-agy-groups-{}", std::process::id()));
        let google = Google::start();
        let reading = signed_in(&home, &google)
            .open(Alone.into_object())
            .expect("a reading opens");
        reading
            .opened(
                r#"{"models": {"availableModels": [
                {"modelId": "alpha-high", "name": "Gemini Alpha (High)"},
                {"modelId": "gamma", "name": "Claude Gamma"}]}}"#,
            )
            .expect("models");

        let read = reading.read(0, false).expect("answered");
        assert!(read.failure.is_none(), "{:?}", read.failure);
        let shown: Vec<_> = read
            .buckets
            .iter()
            .map(|bucket| {
                (
                    bucket.group.as_deref().unwrap_or(""),
                    bucket.label.as_deref().unwrap_or(""),
                    bucket.status.clone(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("Gemini Models", "Weekly Limit Remaining", BucketStatus::Ok),
                (
                    "Gemini Models",
                    "Five Hour Limit Remaining",
                    BucketStatus::Ok
                ),
                (
                    "Claude and GPT models",
                    "Weekly Limit Remaining",
                    BucketStatus::Exhausted
                ),
                (
                    "Claude and GPT models",
                    "Five Hour Limit Remaining",
                    BucketStatus::Disabled
                ),
            ]
        );
        assert!(
            read.buckets
                .iter()
                .all(|bucket| bucket.id.starts_with(BUCKET_PREFIX))
        );
        assert_eq!(read.buckets[0].model_names, ["Gemini Alpha", "Gemini Beta"]);
        // One of the group's two models is not the agent's, so the group stops none of the agent's.
        assert_eq!(read.buckets[0].models, Coverage::Unknown);
        assert_eq!(
            read.buckets[2].models,
            Coverage::Only {
                models: alloc::vec!["gamma".to_string()]
            }
        );
        assert_eq!(
            read.account.and_then(|account| account.plan).as_deref(),
            Some("Plan")
        );

        // With a login stored, a session's reading is the same: the language server has no groups to give.
        let in_session = reading.read(u32::MAX - 1, false).expect("answered");
        assert_eq!(in_session.buckets.len(), 4);
        assert!(in_session.failure.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_settings_name_agys_command_line_to_the_service() {
        assert!(
            AgySettings::default()
                .user_agent
                .starts_with("antigravity/cli/")
        );
    }

    #[test]
    fn the_description_is_one_the_host_accepts() {
        let info = AgyUsage::default().describe().expect("described");
        assert_eq!(info.problem(), None);
        assert_eq!(info.id, "antigravity");
        assert_eq!(info.bucket_prefix, BUCKET_PREFIX);
        assert_eq!(info.retries_ms, RETRIES_MS);
        assert!(info.stored);
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
        let home = std::env::temp_dir().join(format!("canoa-agy-json-{}", std::process::id()));
        let reading = isolated(&home)
            .open(Alone.into_object())
            .expect("a reading opens");
        assert!(reading.opened("not json").is_err());
        assert!(reading.opened("{}").is_ok());
    }

    #[test]
    fn a_process_with_no_port_is_read_again_later() {
        let home = std::env::temp_dir().join(format!("canoa-agy-port-{}", std::process::id()));
        let reading = isolated(&home)
            .open(Alone.into_object())
            .expect("a reading opens");
        // No process has this id, so nothing of it listens.
        let read = reading.read(u32::MAX - 1, false).expect("answered");
        let failure = read.failure.expect("nothing to read");
        assert_eq!(failure.retry, Retry::NotYet, "{}", failure.why);
        assert!(read.buckets.is_empty());
    }

    #[test]
    fn with_no_process_and_no_login_stored_it_says_so_for_good() {
        let home = std::env::temp_dir().join(format!("canoa-agy-none-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".gemini")).expect("a home");
        let reading = isolated(&home)
            .open(Alone.into_object())
            .expect("a reading opens");
        let read = reading.read(0, false).expect("answered");
        let failure = read.failure.expect("no login");
        assert_eq!(failure.why, NOT_SIGNED_IN);
        assert_eq!(failure.retry, Retry::Never);
        assert!(read.account.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}
