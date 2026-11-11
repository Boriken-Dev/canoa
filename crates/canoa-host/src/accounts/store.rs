//! Where an account's credentials rest. A store keeps bytes by account id; what they mean is [`super::Record`]'s.
//!
//! The operating system's credential store is the default. Windows' Credential Manager refuses a credential over 2560
//! bytes and a Codex `auth.json` is several times that, so a record is cut into chunks of [`CHUNK`] bytes, each its own
//! entry, and an index entry names the generation that is current. A new generation's chunks are written before the index
//! points at them, so a write that stops half way leaves the old record whole.
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

use aes_gcm::aead::{Aead, Generate, Key, KeyInit, Payload};
use aes_gcm::aes::cipher::consts::U12;
use aes_gcm::{Aes256Gcm, Nonce};
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

/// The operating system's credential store, through the `keyring` crate: one entry per chunk, under `service`.
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

impl KeyringStore {
    pub(crate) fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, user: &str) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(&self.service, user).map_err(|error| {
            failed(
                "the operating system's credential store is not available (set [host] account_store = \"file\" for a host with none)",
                error,
            )
        })
    }

    fn index(&self, id: &str) -> Result<Option<Index>, StoreError> {
        match self.entry(id)?.get_secret() {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| StoreError("the credential store's index is not one of ours".into())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(failed("the credential store could not be read", error)),
        }
    }

    fn chunk_user(id: &str, generation: u64, number: usize) -> String {
        format!("{id}#{generation}#{number}")
    }

    fn forget_chunks(&self, id: &str, index: &Index) {
        for number in 0..index.chunks {
            if let Ok(entry) = self.entry(&Self::chunk_user(id, index.generation, number)) {
                let _ = entry.delete_credential();
            }
        }
    }
}

impl SecretStore for KeyringStore {
    fn describe(&self) -> &'static str {
        "the operating system's credential store"
    }

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(index) = self.index(id)? else {
            return Ok(None);
        };
        if index.len > MAX_RECORD {
            return Err(StoreError(
                "a stored record is larger than any we write".into(),
            ));
        }
        let mut whole = Vec::with_capacity(index.len);
        for number in 0..index.chunks {
            let entry = self.entry(&Self::chunk_user(id, index.generation, number))?;
            let mut chunk = entry
                .get_secret()
                .map_err(|error| failed("a chunk of the stored record is missing", error))?;
            whole.extend_from_slice(&chunk);
            chunk.zeroize();
        }
        if whole.len() != index.len {
            whole.zeroize();
            return Err(StoreError("a stored record is the wrong length".into()));
        }
        Ok(Some(whole))
    }

    fn put(&self, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let before = self.index(id).ok().flatten();
        let generation = before.as_ref().map_or(1, |index| index.generation + 1);
        let chunks = bytes.len().div_ceil(CHUNK).max(1);
        let now = Index {
            generation,
            chunks,
            len: bytes.len(),
        };
        for (number, part) in bytes.chunks(CHUNK).enumerate() {
            self.entry(&Self::chunk_user(id, generation, number))?
                .set_secret(part)
                .map_err(|error| failed("the credential store refused a credential", error))?;
        }
        if bytes.is_empty() {
            self.entry(&Self::chunk_user(id, generation, 0))?
                .set_secret(&[])
                .map_err(|error| failed("the credential store refused a credential", error))?;
        }
        let index = serde_json::to_vec(&now).map_err(|error| failed("index", error))?;
        self.entry(id)?
            .set_secret(&index)
            .map_err(|error| failed("the credential store refused a credential", error))?;
        if let Some(before) = before {
            self.forget_chunks(id, &before);
        }
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<(), StoreError> {
        if let Some(index) = self.index(id)? {
            self.forget_chunks(id, &index);
        }
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(failed("the credential store could not forget it", error)),
        }
    }
}

/// A file per account, sealed to a key beside it: see the module's documentation.
#[derive(Debug)]
pub(crate) struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub(crate) fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The file of `id`: a digest, since an id may hold any character a path cannot.
    pub(crate) fn path_of(&self, id: &str) -> PathBuf {
        let digest = Sha256::digest(id.as_bytes());
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.sealed"))
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("store.key")
    }

    /// The sealing key, made when there is none.
    fn key(&self) -> Result<Vec<u8>, StoreError> {
        let path = self.key_path();
        match fs::read(&path) {
            Ok(key) if key.len() == KEY_LEN => return Ok(key),
            Ok(mut key) => {
                key.zeroize();
                return Err(StoreError(format!(
                    "{} is not a sealing key of ours",
                    path.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failed("the sealing key could not be read", error)),
        }
        fs::create_dir_all(&self.dir).map_err(|error| failed("the account directory", error))?;
        let mut fresh = Key::<Aes256Gcm>::generate();
        let written = private_file(&path).and_then(|mut file| file.write_all(&fresh));
        let key = fresh.to_vec();
        fresh.zeroize();
        match written {
            Ok(()) => Ok(key),
            // Another process made it first: its key is the one.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                fs::read(&path).map_err(|error| failed("the sealing key could not be read", error))
            }
            Err(error) => Err(failed("the sealing key could not be written", error)),
        }
    }
}

/// A new file only this user can read where the system lets the host say so, and only where nothing is.
fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
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
        let beside = path.with_extension(format!("tmp{}", std::process::id()));
        let _ = fs::remove_file(&beside);
        let written = private_file(&beside)
            .and_then(|mut file| file.write_all(&sealed).and_then(|()| file.sync_all()))
            .and_then(|()| fs::rename(&beside, &path));
        if let Err(error) = written {
            let _ = fs::remove_file(&beside);
            return Err(failed("the account could not be written", error));
        }
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
