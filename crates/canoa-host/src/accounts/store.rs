//! Where an account's credentials rest. A store keeps bytes by account id; what they mean is [`super::Record`]'s.
//!
//! The operating system's credential store is the default. Windows' Credential Manager refuses a credential over 2560
//! bytes and a Codex `auth.json` is several times that, so a record is cut into chunks of [`CHUNK`] bytes, each its own
//! entry, and an index entry names the generation that is current. A new generation's chunks are written before the index
//! points at them, so a write that stops half way leaves the old record whole. Entry names hold a digest of the account's
//! id, never the id, and the service holds one of the host's home, so two accounts or two homes never share an entry.
//!
//! The file store is for a host with no credential store: a file per account sealed with AES-256-GCM to a key in the same
//! directory, so it protects against a copied file and not against whoever can read the directory.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use aes_gcm::aead::{Aead, Generate, Key, KeyInit, Payload};
use aes_gcm::aes::cipher::consts::U12;
use aes_gcm::{Aes256Gcm, Nonce};
use keyring_core::{Entry, Error as KeyError};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize as _;

/// The most one entry of the operating system's store holds; under Windows' 2560.
const CHUNK: usize = 2048;
/// What a sealed file starts with, so another file is not read as one.
const MAGIC: &[u8] = b"CANOA-ACCOUNT\x01";
/// The length of the sealing key and of a nonce.
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// The most a sealed file or a stored record may hold; a record is a few kilobytes.
const MAX_RECORD: usize = 1 << 20;
/// How many generations a removal looks for when the index is damaged and cannot say.
const DAMAGED_SWEEP: u64 = 16;

/// Why a store could not be used. It carries the store's words and never a credential.
#[derive(Debug)]
pub(crate) struct StoreError(pub(crate) String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for StoreError {}

fn failed(what: &str, error: impl fmt::Display) -> StoreError {
    StoreError(format!("{what}: {error}"))
}

/// Bytes kept by account id.
pub(crate) trait SecretStore: Send + Sync + fmt::Debug {
    /// What the store is called in a message.
    fn describe(&self) -> &'static str;

    /// The bytes of `id`; `None` when it has none.
    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Replaces the bytes of `id`.
    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// Forgets `id`; not having it is no error.
    fn remove(&self, id: &str) -> Result<(), StoreError>;
}

/// A digest of `text`, as 32 hex digits: a name that holds no character a store reads as structure.
pub(crate) fn tag(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The service under which `home`'s accounts rest, so two homes of one user never share a credential.
pub(crate) fn service_for(home: &Path) -> String {
    format!("dev.boriken.canoa.{}", &tag(&home.to_string_lossy())[..16])
}

/// One operation on the operating system's store at a time in this process, whichever store object it comes through. That
/// makes one writer of an account's entries, and it keeps Windows' Credential Manager from answering "no such credential"
/// for a chunk just written while another thread deletes or writes elsewhere, which it was seen to do.
pub(crate) fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs one call on the operating system's store, again after a moment when the platform failed it: Windows answers
/// "not enough memory" to a write when the machine is short of it, and the call is good a moment later.
fn patiently<T>(mut call: impl FnMut() -> Result<T, KeyError>) -> Result<T, KeyError> {
    let mut tries = 0;
    loop {
        match call() {
            Err(KeyError::PlatformFailure(_)) if tries < 4 => {
                tries += 1;
                std::thread::sleep(core::time::Duration::from_millis(150));
            }
            other => return other,
        }
    }
}

/// Makes the platform's credential store the default of `keyring_core`, once.
fn use_platform_store() -> Result<(), StoreError> {
    static ONCE: OnceLock<Result<(), String>> = OnceLock::new();
    ONCE.get_or_init(|| {
        #[cfg(target_os = "windows")]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new();
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        ))]
        let store = zbus_secret_service_keyring_store::Store::new();
        #[cfg(any(windows, unix))]
        {
            store
                .map(|store| keyring_core::set_default_store(store))
                .map_err(|error| error.to_string())
        }
        #[cfg(not(any(windows, unix)))]
        {
            Err("this platform has no credential store".to_owned())
        }
    })
    .clone()
    .map_err(|why| {
        StoreError(format!(
            "the operating system's credential store is not available ({why}); set [host] account_store = \"file\" for a host with none"
        ))
    })
}

/// The operating system's credential store, through `keyring-core`: one entry per chunk, under `service`.
pub(crate) struct KeyringStore {
    service: String,
}

impl fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyringStore")
            .field("service", &self.service)
            .finish()
    }
}

/// What the index entry of an account says.
#[derive(serde::Serialize, serde::Deserialize)]
struct Index {
    generation: u64,
    chunks: usize,
    len: usize,
}

/// Why an index could not be had.
enum IndexError {
    /// The store failed; nothing is known about the account.
    Unreadable(StoreError),
    /// The entry is there and is not an index of ours.
    Damaged,
}

impl IndexError {
    fn into_error(self) -> StoreError {
        match self {
            Self::Unreadable(error) => error,
            Self::Damaged => StoreError(
                "the credential store's index for this account is damaged: remove the account and make it again"
                    .into(),
            ),
        }
    }
}

impl KeyringStore {
    pub(crate) fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub(crate) fn entry(&self, user: &str) -> Result<Entry, StoreError> {
        use_platform_store()?;
        // Windows' Credential Manager roams an enterprise credential with the profile; this one stays on this machine.
        let modifiers: std::collections::HashMap<&str, &str> = if cfg!(windows) {
            std::collections::HashMap::from([("persistence", "local")])
        } else {
            std::collections::HashMap::new()
        };
        Entry::new_with_modifiers(&self.service, user, &modifiers)
            .map_err(|error| failed("the credential store refused an entry", error))
    }

    pub(crate) fn index_user(id: &str) -> String {
        format!("acct.{}", tag(id))
    }

    pub(crate) fn chunk_user(id: &str, generation: u64, number: usize) -> String {
        format!("acct.{}.{generation}.{number}", tag(id))
    }

    fn index(&self, id: &str) -> Result<Option<Index>, IndexError> {
        let entry = self
            .entry(&Self::index_user(id))
            .map_err(IndexError::Unreadable)?;
        // Windows' Credential Manager was seen to answer "no such credential" for one written a moment before, under load: an
        // account is taken to have none only after it has said so a few times.
        let mut asked = 0;
        let read = loop {
            match patiently(|| entry.get_secret()) {
                Err(KeyError::NoEntry) if asked < 2 => {
                    asked += 1;
                    std::thread::sleep(core::time::Duration::from_millis(100));
                }
                other => break other,
            }
        };
        match read {
            Ok(mut bytes) => {
                let index = serde_json::from_slice(&bytes);
                bytes.zeroize();
                index.map(Some).map_err(|_| IndexError::Damaged)
            }
            Err(KeyError::NoEntry) => Ok(None),
            Err(error) => Err(IndexError::Unreadable(failed(
                "the credential store could not be read",
                error,
            ))),
        }
    }

    /// Deletes every chunk of `generation`, however many there are.
    fn forget_generation(&self, id: &str, generation: u64) {
        for number in 0.. {
            let Ok(entry) = self.entry(&Self::chunk_user(id, generation, number)) else {
                return;
            };
            if patiently(|| entry.delete_credential()).is_err() {
                return;
            }
        }
    }

    /// The persistence Windows keeps the account's index with.
    #[cfg(all(test, windows))]
    pub(crate) fn persistence_of(&self, id: &str) -> Option<String> {
        self.entry(&Self::index_user(id))
            .ok()?
            .get_attributes()
            .ok()?
            .get("persistence")
            .cloned()
    }
}

impl SecretStore for KeyringStore {
    fn describe(&self) -> &'static str {
        "the operating system's credential store"
    }

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let _turn = one_at_a_time();
        let Some(index) = self.index(id).map_err(IndexError::into_error)? else {
            return Ok(None);
        };
        if index.len > MAX_RECORD {
            return Err(StoreError(
                "a stored record is larger than any we write".into(),
            ));
        }
        let mut whole = Vec::with_capacity(index.len);
        for number in 0..index.chunks {
            let read = self
                .entry(&Self::chunk_user(id, index.generation, number))
                .and_then(|entry| {
                    patiently(|| entry.get_secret())
                        .map_err(|error| failed("a chunk of the stored record is missing", error))
                });
            match read {
                Ok(mut chunk) => {
                    whole.extend_from_slice(&chunk);
                    chunk.zeroize();
                }
                Err(error) => {
                    whole.zeroize();
                    return Err(error);
                }
            }
        }
        if whole.len() != index.len {
            whole.zeroize();
            return Err(StoreError("a stored record is the wrong length".into()));
        }
        Ok(Some(whole))
    }

    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _turn = one_at_a_time();
        // An index that cannot be read is not a reason to start again at generation 1 over what it names.
        let before = self.index(id).map_err(IndexError::into_error)?;
        let generation = before.as_ref().map_or(1, |index| index.generation + 1);
        // What an earlier put that stopped half way left under this generation.
        self.forget_generation(id, generation);
        let chunks = bytes.len().div_ceil(CHUNK).max(1);
        let parts: Vec<&[u8]> = if bytes.is_empty() {
            vec![&[][..]]
        } else {
            bytes.chunks(CHUNK).collect()
        };
        for (number, part) in parts.into_iter().enumerate() {
            let entry = self.entry(&Self::chunk_user(id, generation, number))?;
            let written = patiently(|| entry.set_secret(part))
                .map_err(|error| failed("the credential store refused a credential", error));
            if let Err(error) = written {
                self.forget_generation(id, generation);
                return Err(error);
            }
        }
        let mut index = serde_json::to_vec(&Index {
            generation,
            chunks,
            len: bytes.len(),
        })
        .map_err(|error| failed("index", error))?;
        let index_entry = self.entry(&Self::index_user(id))?;
        let pointed = patiently(|| index_entry.set_secret(&index))
            .map_err(|error| failed("the credential store refused a credential", error));
        index.zeroize();
        if let Err(error) = pointed {
            self.forget_generation(id, generation);
            return Err(error);
        }
        if let Some(before) = before {
            self.forget_generation(id, before.generation);
        }
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<(), StoreError> {
        let _turn = one_at_a_time();
        let newest = match self.index(id) {
            Ok(Some(index)) => index.generation,
            Ok(None) => 0,
            Err(IndexError::Damaged) => DAMAGED_SWEEP,
            Err(error @ IndexError::Unreadable(_)) => return Err(error.into_error()),
        };
        // Every generation there may be: a put that failed left chunks under one the index never named.
        for generation in 1..=newest + 1 {
            self.forget_generation(id, generation);
        }
        let index_entry = self.entry(&Self::index_user(id))?;
        match patiently(|| index_entry.delete_credential()) {
            Ok(()) | Err(KeyError::NoEntry) => Ok(()),
            Err(error) => Err(failed("the credential store could not forget it", error)),
        }
    }
}

/// A file per account, sealed to a key beside it: see the module's documentation.
#[derive(Debug)]
pub(crate) struct FileStore {
    dir: PathBuf,
}

/// A name for a file being written, which no other writer in this process or another has.
fn unique_suffix() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!(
        "{}.{nanos}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Makes what was just renamed or linked into `dir` survive a crash, where the system lets a directory be synced.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

impl FileStore {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The file of `id`: a digest, since an id may hold any character a path cannot.
    pub(crate) fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.sealed", tag(id)))
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("store.key")
    }

    fn read_key(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
        match fs::read(path) {
            Ok(key) if key.len() == KEY_LEN => Ok(Some(key)),
            Ok(mut key) => {
                key.zeroize();
                Err(StoreError(format!(
                    "{} is not a sealing key of ours",
                    path.display()
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(failed("the sealing key could not be read", error)),
        }
    }

    /// The sealing key, made when there is none: written whole beside its place and linked in without replacing a key
    /// another writer made first, so no one reads half of it.
    fn key(&self) -> Result<Vec<u8>, StoreError> {
        let path = self.key_path();
        if let Some(key) = Self::read_key(&path)? {
            return Ok(key);
        }
        super::private::create_dir(&self.dir)
            .map_err(|error| failed("the account directory", error))?;
        let mut fresh = Key::<Aes256Gcm>::generate();
        let beside = self.dir.join(format!("store.key.{}", unique_suffix()));
        let written = super::private::create_file(&beside)
            .and_then(|mut file| file.write_all(&fresh).and_then(|()| file.sync_all()))
            .and_then(|()| fs::hard_link(&beside, &path));
        let _ = fs::remove_file(&beside);
        let key = fresh.to_vec();
        fresh.zeroize();
        match written {
            Ok(()) => {
                sync_dir(&self.dir);
                Ok(key)
            }
            // Another writer linked its key first: that one is the key, once it is a whole one.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let mut ours = key;
                ours.zeroize();
                Self::read_key(&path)?.ok_or_else(|| StoreError("the sealing key vanished".into()))
            }
            Err(error) => {
                let mut ours = key;
                ours.zeroize();
                Err(failed("the sealing key could not be written", error))
            }
        }
    }
}

impl SecretStore for FileStore {
    fn describe(&self) -> &'static str {
        "a sealed file in the host's home"
    }

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let sealed = match fs::read(self.path_of(id)) {
            Ok(sealed) => sealed,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(failed("the account could not be read", error)),
        };
        if sealed.len() > MAX_RECORD
            || sealed.len() < MAGIC.len() + NONCE_LEN
            || !sealed.starts_with(MAGIC)
        {
            return Err(StoreError("the account file is not one of ours".into()));
        }
        let mut key = self.key()?;
        let cipher = Aes256Gcm::new_from_slice(&key);
        key.zeroize();
        let cipher = cipher.map_err(|_| StoreError("the sealing key is unusable".into()))?;
        let (nonce, text) = sealed[MAGIC.len()..].split_at(NONCE_LEN);
        let nonce = Nonce::<U12>::try_from(nonce)
            .map_err(|_| StoreError("the account file is damaged".into()))?;
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: text,
                    aad: id.as_bytes(),
                },
            )
            .map(Some)
            .map_err(|_| StoreError("the account file does not open with this host's key".into()))
    }

    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let mut key = self.key()?;
        let cipher = Aes256Gcm::new_from_slice(&key);
        key.zeroize();
        let cipher = cipher.map_err(|_| StoreError("the sealing key is unusable".into()))?;
        let nonce = Nonce::<U12>::generate();
        let text = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: bytes,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| StoreError("the account could not be sealed".into()))?;
        let mut sealed = Vec::with_capacity(MAGIC.len() + NONCE_LEN + text.len());
        sealed.extend_from_slice(MAGIC);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&text);
        let path = self.path_of(id);
        // Beside its place and then over it, so nobody reads half of it.
        let beside = self
            .dir
            .join(format!("{}.tmp.{}", tag(id), unique_suffix()));
        let written = super::private::create_file(&beside)
            .and_then(|mut file| file.write_all(&sealed).and_then(|()| file.sync_all()))
            .and_then(|()| fs::rename(&beside, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&beside);
            return Err(failed("the account could not be written", error));
        }
        sync_dir(&self.dir);
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<(), StoreError> {
        match fs::remove_file(self.path_of(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(failed("the account could not be removed", error)),
        }
    }
}
