//! Identities from files and bytes: PEM, DER, encrypted PKCS#8 and PKCS#12.

use alloc::vec::Vec;
use std::path::Path;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use zeroize::Zeroizing;

use crate::error::{Error, Reason};
use crate::identity::Identity;
use crate::password::KeyPassword;
use crate::settings::KeySettings;

fn credential(what: &'static str) -> Error {
    Error::Credential(Reason::new(what))
}

fn is_pem(bytes: &[u8]) -> bool {
    bytes.windows(11).any(|window| window == b"-----BEGIN ")
}

fn pem_chain(text: &[u8]) -> Result<Vec<CertificateDer<'static>>, Error> {
    CertificateDer::pem_slice_iter(text)
        .collect::<Result<_, _>>()
        .map_err(|_| credential("the certificate text is not PEM"))
}

/// The private key of PEM text, decrypting an encrypted PKCS#8 key with `password`.
pub(crate) fn pem_private_key(
    text: &[u8],
    password: &KeyPassword,
    settings: &KeySettings,
) -> Result<PrivateKeyDer<'static>, Error> {
    if text
        .windows(29)
        .any(|window| window == b"BEGIN ENCRYPTED PRIVATE KEY--")
    {
        return decrypt_pem(text, password, settings);
    }
    PrivateKeyDer::from_pem_slice(text).map_err(|_| credential("the key text is not PEM"))
}

#[cfg(feature = "encrypted-keys")]
fn decrypt_pem(
    text: &[u8],
    password: &KeyPassword,
    settings: &KeySettings,
) -> Result<PrivateKeyDer<'static>, Error> {
    let text = core::str::from_utf8(text).map_err(|_| credential("the key text is not PEM"))?;
    let (_, document) = pkcs8::SecretDocument::from_pem(text)
        .map_err(|_| credential("the encrypted key does not decode"))?;
    let info = pkcs8::EncryptedPrivateKeyInfoRef::try_from(document.as_bytes())
        .map_err(|_| credential("the encrypted key does not decode"))?;
    check_kdf_cost(&info, settings)?;
    let password = password
        .resolve_bytes()?
        .ok_or_else(|| credential("the key is encrypted and no password was given"))?;
    let document = info
        .decrypt(&*password)
        .map_err(|_| credential("the password does not decrypt the key"))?;
    Ok(PrivateKeyDer::from(
        rustls::pki_types::PrivatePkcs8KeyDer::from(document.as_bytes().to_vec()),
    ))
}

/// Refuses a key whose key derivation asks for more than `settings` allow; `pkcs5` checks its
/// bounds only when it builds parameters, not when it decodes them.
#[cfg(feature = "encrypted-keys")]
fn check_kdf_cost(
    info: &pkcs8::EncryptedPrivateKeyInfoRef<'_>,
    settings: &KeySettings,
) -> Result<(), Error> {
    use pkcs8::pkcs5::pbes2::Kdf;
    let Some(parameters) = info.encryption_algorithm.pbes2() else {
        return Ok(());
    };
    match &parameters.kdf {
        Kdf::Pbkdf2(params) if params.iteration_count > settings.max_pbkdf2_iterations => Err(
            credential("the key asks for more PBKDF2 iterations than the settings allow"),
        ),
        Kdf::Scrypt(params) => {
            let blocks = u128::from(params.cost_parameter) + u128::from(params.parallelization);
            let memory = blocks.saturating_mul(128 * u128::from(params.block_size));
            if memory > u128::from(settings.max_scrypt_memory) {
                return Err(credential(
                    "the key asks for more scrypt memory than the settings allow",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(not(feature = "encrypted-keys"))]
fn decrypt_pem(
    _text: &[u8],
    _password: &KeyPassword,
    _settings: &KeySettings,
) -> Result<PrivateKeyDer<'static>, Error> {
    Err(Error::Unsupported(Reason::new(
        "an encrypted key needs the `encrypted-keys` feature",
    )))
}

impl Identity {
    fn from_parts(
        chain: Vec<CertificateDer<'static>>,
        key: &PrivateKeyDer<'_>,
        label: &str,
    ) -> Result<Self, Error> {
        let key = rustls::crypto::ring::sign::any_supported_type(key)
            .map_err(|_| credential("the key is not one rustls can sign with"))?;
        Self::new(chain, key, label)
    }

    /// An identity from DER: the certificates (the leaf first) and the private key (PKCS#8, PKCS#1
    /// or SEC1, unencrypted).
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for no certificate or a key `rustls` cannot sign with.
    pub fn from_der<I, C>(chain: I, key: &[u8]) -> Result<Self, Error>
    where
        I: IntoIterator<Item = C>,
        C: AsRef<[u8]>,
    {
        let chain: Vec<CertificateDer<'static>> = chain
            .into_iter()
            .map(|der| CertificateDer::from(der.as_ref().to_vec()))
            .collect();
        let key = PrivateKeyDer::try_from(key).map_err(|_| credential("the key is not DER"))?;
        Self::from_parts(chain, &key, "der")
    }

    /// Like [`from_pem`](Self::from_pem), for a key that may be an encrypted PKCS#8 key
    /// (`-----BEGIN ENCRYPTED PRIVATE KEY-----`); `password` is asked for only then. Without the
    /// `encrypted-keys` feature an encrypted key is [`Error::Unsupported`].
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for text that holds no certificate or key, a missing or wrong
    /// password, or a key `rustls` cannot sign with.
    pub fn from_pem_with(
        certificates: &[u8],
        key: &[u8],
        password: &KeyPassword,
    ) -> Result<Self, Error> {
        Self::from_pem_with_settings(certificates, key, password, &KeySettings::default())
    }

    /// Like [`from_pem_with`](Self::from_pem_with), with the key derivation bounds of `settings`
    /// instead of the defaults.
    ///
    /// # Errors
    ///
    /// As [`from_pem_with`](Self::from_pem_with); [`Error::Credential`] too for a key asking for
    /// more work than `settings` allow, refused before any is done.
    pub fn from_pem_with_settings(
        certificates: &[u8],
        key: &[u8],
        password: &KeyPassword,
        settings: &KeySettings,
    ) -> Result<Self, Error> {
        let chain = pem_chain(certificates)?;
        let key = pem_private_key(key, password, settings)?;
        Self::from_parts(chain, &key, "pem")
    }

    /// An identity from files: PEM or DER certificates in `certificate`, and the private key in
    /// `key`, PEM (PKCS#8, PKCS#1 or SEC1; encrypted PKCS#8 with `password`) or DER.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when a file cannot be read, otherwise as
    /// [`from_pem_with`](Self::from_pem_with).
    pub fn load(
        certificate: impl AsRef<Path>,
        key: impl AsRef<Path>,
        password: KeyPassword,
    ) -> Result<Self, Error> {
        Self::load_with_settings(certificate, key, password, &KeySettings::default())
    }

    /// Like [`load`](Self::load), with the key derivation bounds of `settings`.
    ///
    /// # Errors
    ///
    /// As [`from_pem_with_settings`](Self::from_pem_with_settings), and [`Error::Io`] for a file
    /// that cannot be read.
    pub fn load_with_settings(
        certificate: impl AsRef<Path>,
        key: impl AsRef<Path>,
        password: KeyPassword,
        settings: &KeySettings,
    ) -> Result<Self, Error> {
        let certificates = std::fs::read(certificate)?;
        let key = Zeroizing::new(std::fs::read(key)?);
        match (is_pem(&certificates), is_pem(&key)) {
            (true, true) => Self::from_pem_with_settings(&certificates, &key, &password, settings),
            (true, false) => {
                let key = PrivateKeyDer::try_from(key.as_slice())
                    .map_err(|_| credential("the key is not DER"))?;
                Self::from_parts(pem_chain(&certificates)?, &key, "file")
            }
            (false, true) => {
                let key = pem_private_key(&key, &password, settings)?;
                let chain = alloc::vec![CertificateDer::from(certificates)];
                Self::from_parts(chain, &key, "file")
            }
            (false, false) => Self::from_der([certificates], &key),
        }
    }

    /// An identity from one file holding the certificates and the key: combined PEM, or, with the
    /// `pkcs12` feature, a PKCS#12 file (found by its shape, opened with `password`).
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the file cannot be read, otherwise as [`load`](Self::load).
    pub fn load_combined(path: impl AsRef<Path>, password: KeyPassword) -> Result<Self, Error> {
        Self::load_combined_with_settings(path, password, &KeySettings::default())
    }

    /// Like [`load_combined`](Self::load_combined), with the key derivation bounds of `settings`
    /// for a PEM key. A PKCS#12 file is not covered: see [`KeySettings`].
    ///
    /// # Errors
    ///
    /// As [`load_combined`](Self::load_combined), and as
    /// [`from_pem_with_settings`](Self::from_pem_with_settings).
    pub fn load_combined_with_settings(
        path: impl AsRef<Path>,
        password: KeyPassword,
        settings: &KeySettings,
    ) -> Result<Self, Error> {
        let bytes = Zeroizing::new(std::fs::read(path)?);
        if is_pem(&bytes) {
            return Self::from_pem_with_settings(&bytes, &bytes, &password, settings);
        }
        #[cfg(feature = "pkcs12")]
        if bytes.first() == Some(&0x30) {
            return Self::from_pkcs12(&bytes, &password);
        }
        Err(credential("a file with no PEM text needs a separate key"))
    }

    /// An identity from a PKCS#12 file: the key and its chain, legacy RC2 and 3DES ciphers
    /// included. `password` is asked for only to open the file.
    ///
    /// # Errors
    ///
    /// [`Error::Credential`] for a file that is not PKCS#12, a wrong password or no private key.
    #[cfg(feature = "pkcs12")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pkcs12")))]
    pub fn from_pkcs12(data: &[u8], password: &KeyPassword) -> Result<Self, Error> {
        let password = password.resolve()?;
        let store = p12_keystore::KeyStore::from_pkcs12(
            data,
            password.as_ref().map_or("", crate::secret::Secret::expose),
            p12_keystore::Pkcs12ImportPolicy::Strict,
        )
        .map_err(|_| credential("the PKCS#12 file is unreadable or the password is wrong"))?;
        let (_, chain) = store
            .private_key_chain()
            .ok_or_else(|| credential("the PKCS#12 file holds no private key"))?;
        let certificates = chain
            .certs()
            .iter()
            .map(|certificate| CertificateDer::from(certificate.as_der().to_vec()))
            .collect();
        let key = PrivateKeyDer::try_from(chain.key().as_der())
            .map_err(|_| credential("the PKCS#12 key is not usable"))?;
        Self::from_parts(certificates, &key, "pkcs12")
    }
}
