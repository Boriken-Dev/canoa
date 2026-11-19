//! X509 Certificate builder

use alloc::vec;
use core::fmt;
use der::{Encode, asn1::BitString, referenced::OwnedToRef};
use signature::{
    AsyncRandomizedSigner, AsyncSigner, Keypair, RandomizedSigner, Signer, rand_core::CryptoRng,
};
use spki::{
    DynSignatureAlgorithmIdentifier, EncodePublicKey, ObjectIdentifier, SignatureBitStringEncoding,
};

use crate::{
    AlgorithmIdentifier, SubjectPublicKeyInfo,
    certificate::{self, Certificate, TbsCertificate, Version},
    crl::{CertificateList, RevokedCert, TbsCertList},
    ext::{
        Extensions, ToExtension,
        pkix::{AuthorityKeyIdentifier, CrlNumber, SubjectKeyIdentifier},
    },
    serial_number::SerialNumber,
    time::{Time, Validity},
};

pub mod profile;

use self::profile::BuilderProfile;

#[deprecated(
    since = "0.3.0",
    note = "please use `x509_cert::builder::profile::BuilderProfile` instead"
)]
pub use self::profile::BuilderProfile as Profile;

#[deprecated(
    since = "0.3.0",
    note = "please use `x509_cert::request::RequestBuilder` instead"
)]
pub use crate::request::RequestBuilder;

pub(crate) const NULL_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("0.0.0");

/// Error type
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// ASN.1 DER-related errors.
    Asn1(der::Error),

    /// Public key errors propagated from the [`spki::Error`] type.
    PublicKey(spki::Error),

    /// Signing error propagated for the [`signature::Error`] type.
    Signature(signature::Error),

    /// Each RelativeDistinguishedName MUST contain exactly one AttributeTypeAndValue.
    NonUniqueRdn,

    /// Each Name MUST NOT contain more than one instance of a given
    /// AttributeTypeAndValue across all RelativeDistinguishedNames unless explicitly
    /// allowed in these Requirements
    NonUniqueATV,

    /// Non-ordered attribute or invalid attribute
    InvalidAttribute {
        /// Offending [`ObjectIdentifier`]
        oid: ObjectIdentifier,
    },

    /// Not all required elements were specified
    MissingAttributes,
}

impl core::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Asn1(err) => write!(f, "ASN.1 error: {err}"),
            Error::PublicKey(err) => write!(f, "public key error: {err}"),
            Error::Signature(err) => write!(f, "signature error: {err}"),
            Error::NonUniqueRdn => write!(
                f,
                "Each RelativeDistinguishedName MUST contain exactly one AttributeTypeAndValue."
            ),
            Error::NonUniqueATV => write!(
                f,
                "Each Name MUST NOT contain more than one instance of a given AttributeTypeAndValue"
            ),
            Error::InvalidAttribute { oid } => write!(
                f,
                "Non-ordered attribute or invalid attribute found (oid={oid})"
            ),
            Error::MissingAttributes => write!(f, "Not all required elements were specified"),
        }
    }
}

impl From<der::Error> for Error {
    fn from(err: der::Error) -> Error {
        Error::Asn1(err)
    }
}

impl From<spki::Error> for Error {
    fn from(err: spki::Error) -> Error {
        Error::PublicKey(err)
    }
}

impl From<signature::Error> for Error {
    fn from(err: signature::Error) -> Error {
        Error::Signature(err)
    }
}

/// Result type
pub type Result<T> = core::result::Result<T, Error>;

/// X509 Certificate builder
///
#[cfg_attr(feature = "std", doc = "```")]
#[cfg_attr(not(feature = "std"), doc = "```ignore")]
/// use der::Decode;
/// use x509_cert::spki::SubjectPublicKeyInfo;
/// use x509_cert::builder::{CertificateBuilder, Builder, profile};
/// use x509_cert::name::Name;
/// use x509_cert::serial_number::SerialNumber;
/// use x509_cert::time::Validity;
/// use std::str::FromStr;
///
/// # const RSA_2048_DER: &[u8] = &[0x30u8,0x82,0x01,0x22,0x30,0x0d,0x06,0x09,0x2a,0x86,0x48,0x86,0xf7,0x0d,0x01,0x01,0x01,0x05,0x00,0x03,0x82,0x01,0x0f,0x00,0x30,0x82,0x01,0x0a,0x02,0x82,0x01,0x01,0x00,0xb6,0xc4,0x2c,0x51,0x5f,0x10,0xa6,0xaa,0xf2,0x82,0xc6,0x3e,0xdb,0xe2,0x42,0x43,0xa1,0x70,0xf3,0xfa,0x26,0x33,0xbd,0x48,0x33,0x63,0x7f,0x47,0xca,0x4f,0x6f,0x36,0xe0,0x3a,0x5d,0x29,0xef,0xc3,0x19,0x1a,0xc8,0x0f,0x39,0x0d,0x87,0x4b,0x39,0xe3,0x0f,0x41,0x4f,0xce,0xc1,0xfc,0xa0,0xed,0x81,0xe5,0x47,0xed,0xc2,0xcd,0x38,0x2c,0x76,0xf6,0x1c,0x90,0x18,0x97,0x3d,0xb9,0xfa,0x53,0x79,0x72,0xa7,0xc7,0x01,0xf6,0xb7,0x7e,0x09,0x82,0xdf,0xc1,0x5f,0xc0,0x19,0x27,0xee,0x5e,0x7c,0xd9,0x4b,0x4f,0x59,0x9f,0xf0,0x70,0x13,0xa7,0xc8,0x28,0x1b,0xdf,0x22,0xdc,0xbc,0x9a,0xd7,0xca,0xbb,0x7c,0x43,0x11,0xc9,0x82,0xf5,0x8e,0xdb,0x72,0x13,0xad,0x45,0x58,0xb3,0x32,0x26,0x6d,0x74,0x3a,0xed,0x81,0x92,0xd1,0x88,0x4c,0xad,0xb8,0xb1,0x47,0x39,0xa8,0xda,0xda,0x66,0xdc,0x97,0x08,0x06,0xd9,0xc7,0xac,0x45,0x0c,0xb1,0x3d,0x0d,0x7c,0x57,0x5f,0xb1,0x98,0x53,0x4f,0xc6,0x1b,0xc4,0x1b,0xc0,0xf0,0x57,0x4e,0x0e,0x01,0x30,0xc7,0xbb,0xbf,0xbd,0xfd,0xc9,0xf6,0xa6,0xe2,0xe3,0xe2,0xaf,0xf1,0xcb,0xea,0xc8,0x9b,0xa5,0x78,0x84,0x52,0x8d,0x55,0xcf,0xb0,0x83,0x27,0xa1,0xe8,0xc8,0x9f,0x4e,0x00,0x3c,0xf2,0x88,0x8e,0x93,0x32,0x41,0xd9,0xd6,0x95,0xbc,0xbb,0xac,0xdc,0x90,0xb4,0x4e,0x3e,0x09,0x5f,0xa3,0x70,0x58,0xea,0x25,0xb1,0x3f,0x5e,0x29,0x5c,0xbe,0xac,0x6d,0xe8,0x38,0xab,0x8c,0x50,0xaf,0x61,0xe2,0x98,0x97,0x5b,0x87,0x2f,0x02,0x03,0x01,0x00,0x01];
/// # const RSA_2048_PRIV_DER: &[u8] = &[0x30u8,0x82,0x04,0xa3,0x02,0x01,0x00,0x02,0x82,0x01,0x01,0x00,0xb6,0xc4,0x2c,0x51,0x5f,0x10,0xa6,0xaa,0xf2,0x82,0xc6,0x3e,0xdb,0xe2,0x42,0x43,0xa1,0x70,0xf3,0xfa,0x26,0x33,0xbd,0x48,0x33,0x63,0x7f,0x47,0xca,0x4f,0x6f,0x36,0xe0,0x3a,0x5d,0x29,0xef,0xc3,0x19,0x1a,0xc8,0x0f,0x39,0x0d,0x87,0x4b,0x39,0xe3,0x0f,0x41,0x4f,0xce,0xc1,0xfc,0xa0,0xed,0x81,0xe5,0x47,0xed,0xc2,0xcd,0x38,0x2c,0x76,0xf6,0x1c,0x90,0x18,0x97,0x3d,0xb9,0xfa,0x53,0x79,0x72,0xa7,0xc7,0x01,0xf6,0xb7,0x7e,0x09,0x82,0xdf,0xc1,0x5f,0xc0,0x19,0x27,0xee,0x5e,0x7c,0xd9,0x4b,0x4f,0x59,0x9f,0xf0,0x70,0x13,0xa7,0xc8,0x28,0x1b,0xdf,0x22,0xdc,0xbc,0x9a,0xd7,0xca,0xbb,0x7c,0x43,0x11,0xc9,0x82,0xf5,0x8e,0xdb,0x72,0x13,0xad,0x45,0x58,0xb3,0x32,0x26,0x6d,0x74,0x3a,0xed,0x81,0x92,0xd1,0x88,0x4c,0xad,0xb8,0xb1,0x47,0x39,0xa8,0xda,0xda,0x66,0xdc,0x97,0x08,0x06,0xd9,0xc7,0xac,0x45,0x0c,0xb1,0x3d,0x0d,0x7c,0x57,0x5f,0xb1,0x98,0x53,0x4f,0xc6,0x1b,0xc4,0x1b,0xc0,0xf0,0x57,0x4e,0x0e,0x01,0x30,0xc7,0xbb,0xbf,0xbd,0xfd,0xc9,0xf6,0xa6,0xe2,0xe3,0xe2,0xaf,0xf1,0xcb,0xea,0xc8,0x9b,0xa5,0x78,0x84,0x52,0x8d,0x55,0xcf,0xb0,0x83,0x27,0xa1,0xe8,0xc8,0x9f,0x4e,0x00,0x3c,0xf2,0x88,0x8e,0x93,0x32,0x41,0xd9,0xd6,0x95,0xbc,0xbb,0xac,0xdc,0x90,0xb4,0x4e,0x3e,0x09,0x5f,0xa3,0x70,0x58,0xea,0x25,0xb1,0x3f,0x5e,0x29,0x5c,0xbe,0xac,0x6d,0xe8,0x38,0xab,0x8c,0x50,0xaf,0x61,0xe2,0x98,0x97,0x5b,0x87,0x2f,0x02,0x03,0x01,0x00,0x01,0x02,0x82,0x01,0x00,0x7e,0xcc,0x83,0x62,0xc0,0xed,0xb0,0x74,0x11,0x64,0x21,0x5e,0x22,0xf7,0x4a,0xb9,0xd9,0x1b,0xa0,0x69,0x00,0x70,0x0c,0xf6,0x36,0x90,0xe5,0x11,0x4d,0x8e,0xe6,0xbd,0xcf,0xbb,0x2e,0x3f,0x96,0x14,0x69,0x2a,0x67,0x7a,0x08,0x3f,0x16,0x8a,0x5e,0x52,0xe5,0x96,0x8e,0x64,0x07,0xb9,0xd9,0x7c,0x6e,0x0e,0x40,0x64,0xf8,0x2d,0xa0,0xb7,0x58,0xa1,0x4f,0x17,0xb9,0xb7,0xd4,0x1f,0x5f,0x48,0xe2,0x8d,0x65,0x51,0x70,0x4f,0x56,0xe6,0x9e,0x7a,0xa9,0xfa,0x63,0x0f,0xc7,0x64,0x28,0xc0,0x6d,0x25,0xe4,0x55,0xdc,0xfc,0x55,0xb7,0xac,0x2b,0x4f,0x76,0x64,0x3f,0xde,0xd3,0xfe,0x15,0xff,0x78,0xab,0xb2,0x7e,0x65,0xac,0xc4,0xaa,0xd0,0xbd,0xf6,0xdb,0x27,0xef,0x60,0xa6,0x91,0x0c,0x5c,0x4a,0x08,0x5e,0xd4,0x32,0x75,0xab,0x19,0xc1,0xd9,0x97,0xa3,0x2c,0x6e,0xff,0xce,0x7d,0xf2,0xd1,0x93,0x5f,0x6e,0x60,0x1e,0xed,0xe1,0x61,0xa1,0x2b,0x5c,0xc2,0x7c,0xa2,0x1f,0x81,0xd2,0xc9,0x9c,0x3d,0x1e,0xa0,0x8e,0x90,0xe3,0x05,0x3a,0xb0,0x9b,0xef,0xa7,0x24,0xde,0xf0,0xd0,0xc3,0xa3,0xc1,0xe9,0x74,0x0c,0x0d,0x9f,0x76,0x12,0x6a,0x14,0x9e,0xc0,0xaa,0x7d,0x80,0x78,0x20,0x54,0x84,0x25,0x4d,0x95,0x1d,0xb0,0x7c,0x4c,0xf9,0x1f,0xb6,0x45,0x4c,0x09,0x65,0x88,0xfd,0x59,0x24,0xdb,0xab,0xeb,0x35,0x9c,0xa2,0x02,0x52,0x68,0xd0,0x04,0xf9,0xd6,0x6e,0xb3,0xd6,0xf7,0xad,0xc1,0x13,0x9b,0xad,0x40,0xf1,0x6d,0xde,0x63,0x9e,0x11,0x64,0x73,0x76,0xc1,0x02,0x81,0x81,0x00,0xdc,0xc0,0x61,0x24,0x2d,0x4e,0x92,0xaf,0xae,0xe7,0x2a,0xc5,0x13,0xca,0x65,0xb9,0xf7,0x70,0x36,0xf9,0xbd,0x7e,0x0e,0x6e,0x61,0x46,0x1a,0x7e,0xf7,0x65,0x42,0x25,0xec,0x15,0x3c,0x7e,0x5c,0x31,0xa6,0x15,0x7a,0x6e,0x5a,0x13,0xff,0x6e,0x17,0x8e,0x87,0x58,0xc1,0xcb,0x33,0xd9,0xd6,0xbb,0xe3,0x17,0x9e,0xf1,0x89,0x98,0xe4,0x22,0xec,0xdc,0xbe,0xd7,0x8f,0x4e,0xcf,0xdb,0xe5,0xf4,0xfc,0xd8,0xae,0xc2,0xc9,0xd0,0xdc,0x86,0x47,0x3c,0xa9,0xbd,0x16,0xd9,0xd2,0x38,0xd2,0x1f,0xb5,0xdd,0xef,0xbe,0xb1,0x43,0xca,0x61,0xd0,0xbd,0x6a,0xa8,0xd9,0x1f,0x33,0xa0,0x97,0x79,0x0e,0x96,0x40,0xdb,0xc9,0x10,0x85,0xdc,0x5f,0x26,0x34,0x3b,0xa3,0x13,0x8f,0x6b,0x2d,0x67,0x02,0x81,0x81,0x00,0xd3,0xf3,0x14,0x75,0x7e,0x40,0xe9,0x54,0x83,0x6f,0x92,0xbe,0x24,0x23,0x6a,0xf2,0xf0,0xda,0x04,0xa3,0x46,0x53,0xc1,0x80,0xaf,0x67,0xe9,0x60,0x08,0x6d,0x93,0xfd,0xe6,0x5c,0xb2,0x3e,0xfd,0x9d,0x09,0x37,0x47,0x62,0xf5,0x98,0x1e,0x36,0x18,0x49,0xaf,0x68,0xcd,0xd7,0x53,0x94,0xff,0x6a,0x4e,0x06,0xeb,0x69,0xb2,0x09,0xe4,0x22,0x8d,0xb2,0xdf,0xa7,0x0e,0x40,0xf7,0xf9,0x75,0x0a,0x52,0x81,0x76,0x64,0x7b,0x78,0x8d,0x0e,0x57,0x77,0xa2,0xcb,0x8b,0x22,0xe3,0xcd,0x26,0x7f,0xf7,0x0b,0x4f,0x3b,0x02,0xd3,0xaa,0xfb,0x0e,0x18,0xc5,0x90,0xa5,0x64,0xb0,0x31,0x88,0xb0,0xaa,0x5f,0xc4,0x81,0x56,0xb0,0x76,0x22,0x21,0x42,0x43,0xbd,0x12,0x27,0xef,0xa7,0xf2,0xf9,0x02,0x81,0x81,0x00,0xce,0x68,0xb7,0xac,0x1b,0x0d,0x10,0x0d,0x63,0x6e,0x55,0x48,0x87,0x53,0xc5,0xc0,0x98,0x43,0xfd,0xb3,0x90,0xe2,0x70,0x5d,0xf7,0x68,0x94,0x57,0xc9,0xbd,0x8d,0x97,0x65,0xe3,0x09,0x78,0x61,0x7e,0x2e,0xfc,0x80,0x48,0xf4,0xc3,0x24,0x20,0x6d,0xb8,0x60,0x87,0xb6,0x54,0xe9,0x7b,0xb3,0xd4,0x64,0xe7,0xee,0x3f,0x8c,0xd8,0x3f,0xe1,0x04,0x36,0xf7,0xdf,0x18,0xe9,0xa9,0x63,0xc4,0xe6,0x49,0x11,0xd6,0x7e,0xde,0x34,0x04,0x2f,0x2e,0x26,0xe3,0xd3,0xa1,0xad,0x34,0x6a,0xda,0xd6,0xb9,0xb7,0xf6,0x77,0x08,0xcb,0x09,0x4e,0x62,0xde,0xe9,0xff,0x4d,0x5d,0x66,0x69,0xaf,0x98,0x8a,0xf2,0x25,0x5d,0x1c,0xe8,0xed,0x31,0x7c,0x6a,0x7d,0x86,0x91,0xda,0x35,0x4d,0x12,0xdb,0x02,0x81,0x80,0x25,0xf6,0xe5,0x94,0x42,0x20,0x28,0x6b,0x4d,0xfb,0xbf,0x42,0x35,0xc0,0xee,0x58,0x43,0xd2,0x19,0x80,0x91,0x89,0x51,0x20,0xd6,0xca,0x7b,0x20,0x0b,0x82,0x6d,0x3e,0xce,0x73,0x8e,0x2e,0x00,0x49,0x8f,0xac,0x0a,0x2a,0x6c,0xa9,0x69,0xc7,0xf0,0xc3,0xca,0x1a,0xb0,0xbc,0x40,0x29,0x71,0x32,0xbe,0x75,0x38,0xd7,0xbe,0xdf,0x4c,0xb0,0xef,0xc6,0xb9,0x8e,0xf7,0xdb,0xa5,0x4f,0x56,0xaa,0x99,0xaa,0xbc,0xe5,0x34,0xc4,0x9c,0x27,0x94,0x7d,0x46,0x78,0xc5,0x1c,0x63,0xc7,0x8c,0x7c,0xe1,0x68,0x72,0x31,0xb4,0xc8,0xeb,0x58,0x7a,0xe6,0xef,0x04,0x80,0xcb,0xaf,0x4f,0xc0,0x17,0x3c,0xfd,0x58,0x7a,0x7e,0x67,0xaf,0x51,0x5f,0xb9,0xb9,0xde,0x75,0x11,0x18,0x39,0x72,0x29,0x02,0x81,0x80,0x31,0x99,0x54,0x06,0xd4,0x06,0x20,0x7c,0xad,0xea,0xea,0x35,0xb3,0x8d,0x04,0x0c,0x5f,0x8a,0x9a,0x1a,0xe0,0x82,0x7e,0x9e,0xd0,0x6b,0x15,0x3d,0x83,0xb6,0x82,0x19,0x35,0xb4,0xb3,0x6a,0x82,0xbe,0x9d,0x56,0xc7,0x91,0xb5,0x8c,0x27,0x27,0x1a,0x57,0x93,0xd5,0x3a,0x1d,0x65,0x7c,0x08,0x99,0x79,0x60,0xb1,0x43,0x3e,0x51,0x71,0x98,0x7f,0x45,0x2f,0x14,0x4a,0x7c,0x72,0x30,0x6d,0x63,0xe1,0xd3,0xff,0xc0,0xb7,0x1b,0x75,0xab,0x08,0xf2,0xe4,0x5a,0x48,0x2e,0x98,0x84,0x51,0xcb,0xe4,0x78,0xe1,0x2e,0xb2,0x28,0xd0,0x74,0x56,0xc9,0x24,0xb6,0x6f,0x6c,0xed,0x04,0x8d,0x85,0x3f,0x53,0x3e,0x31,0xa6,0x86,0x14,0xf1,0xc3,0xce,0x6d,0x8e,0xc9,0x98,0x3c,0xe7,0x2a,0xf7];
/// # use rsa::{pkcs1v15::SigningKey, pkcs1::DecodeRsaPrivateKey};
/// # use sha2::Sha256;
/// # use std::time::Duration;
/// # use der::referenced::RefToOwned;
/// # fn rsa_signer() -> SigningKey<Sha256> {
/// #     let private_key = rsa::RsaPrivateKey::from_pkcs1_der(RSA_2048_PRIV_DER).unwrap();
/// #     let signing_key = SigningKey::<Sha256>::new(private_key);
/// #     signing_key
/// # }
///
/// let serial_number = SerialNumber::from(42u32);
/// let validity = Validity::from_now(Duration::new(5, 0)).unwrap();
/// let subject = Name::from_str("CN=World domination corporation,O=World domination Inc,C=US").unwrap();
/// let profile = profile::cabf::Root::new(false,subject).expect("Create root profile");
///
/// let pub_key = SubjectPublicKeyInfo::try_from(RSA_2048_DER).expect("get rsa pub key");
///
/// let mut signer = rsa_signer();
/// let mut builder = CertificateBuilder::new(
///     profile,
///     serial_number,
///     validity,
///     pub_key,
/// )
/// .expect("Create certificate builder");
///
/// let cert = builder.build(&signer).expect("Create certificate");
/// ```
pub struct CertificateBuilder<P> {
    tbs: TbsCertificate,
    extensions: Extensions,
    profile: P,
}

impl<P> CertificateBuilder<P>
where
    P: BuilderProfile,
{
    /// Creates a new certificate builder
    pub fn new(
        profile: P,
        serial_number: SerialNumber,
        mut validity: Validity,
        subject_public_key_info: SubjectPublicKeyInfo,
    ) -> Result<Self> {
        let signature_alg = AlgorithmIdentifier {
            oid: NULL_OID,
            parameters: None,
        };

        let subject = profile.get_subject();
        let issuer = profile.get_issuer(&subject);

        validity.not_before.rfc5280_adjust_utc_time()?;
        validity.not_after.rfc5280_adjust_utc_time()?;

        let tbs = TbsCertificate {
            version: Version::V3,
            serial_number,
            signature: signature_alg,
            issuer,
            validity,
            subject,
            subject_public_key_info,
            extensions: None,

            // We will not generate unique identifier because as per RFC5280 Section 4.1.2.8:
            //   CAs conforming to this profile MUST NOT generate
            //   certificates with unique identifiers.
            //
            // https://datatracker.ietf.org/doc/html/rfc5280#section-4.1.2.8
            issuer_unique_id: None,
            subject_unique_id: None,
        };

        let extensions = Extensions::default();
        Ok(Self {
            tbs,
            extensions,
            profile,
        })
    }

    /// Add an extension to this certificate
    ///
    /// Extensions need to implement [`ToExtension`], examples may be found in
    /// in [`ToExtension` documentation](../ext/trait.ToExtension.html#examples) or
    /// [the implementors](../ext/trait.ToExtension.html#implementors).
    pub fn add_extension<E: ToExtension>(
        &mut self,
        extension: E,
    ) -> core::result::Result<(), E::Error> {
        let ext = extension.to_extension(&self.tbs.subject, &self.extensions)?;
        self.extensions.push(ext);
        Ok(())
    }
}

/// Trait for X509 builders
///
/// This trait defines the interface between builder and the signers.
pub trait Builder: Sized {
    /// Type built by this builder
    type Output: Sized;

    /// Assemble the final object from signature.
    fn assemble<S>(self, signature: BitString, signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey;

    /// Finalize and return a serialization of the object for signature.
    fn finalize<S>(&mut self, signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey;

    /// Run the object through the signer and build it.
    ///
    /// # Notes
    ///
    /// When using ECDSA signers, the `Signature` parameter will need to be explicit
    /// as multiple implementation of [`signature::Signer`] with various signature
    /// are available.
    ///
    /// This would look like:
    #[cfg_attr(feature = "std", doc = "```no_run")]
    #[cfg_attr(not(feature = "std"), doc = "```ignore")]
    /// # use p256::elliptic_curve::Generate;
    /// # use rand::rng;
    /// # use std::{
    /// #     str::FromStr,
    /// #     time::Duration
    /// # };
    /// # use x509_cert::{
    /// #     builder::{self, CertificateBuilder, Builder},
    /// #     name::Name,
    /// #     serial_number::SerialNumber,
    /// #     spki::SubjectPublicKeyInfo,
    /// #     time::Validity
    /// # };
    /// #
    /// # let mut rng = rng();
    /// # let signer = p256::ecdsa::SigningKey::generate_from_rng(&mut rng);
    /// # let builder = CertificateBuilder::new(
    /// #     builder::profile::cabf::Root::new(
    /// #         false,
    /// #         Name::from_str("CN=World domination corporation").unwrap()
    /// #     ).unwrap(),
    /// #     SerialNumber::from(42u32),
    /// #     Validity::from_now(Duration::new(5, 0)).unwrap(),
    /// #     SubjectPublicKeyInfo::from_key(signer.verifying_key()).unwrap()
    /// # ).unwrap();
    /// let certificate = builder.build::<_, ecdsa::der::Signature<_>>(&signer).unwrap();
    /// ```
    fn build<S, Signature>(mut self, signer: &S) -> Result<Self::Output>
    where
        S: Signer<Signature>,
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
        Signature: SignatureBitStringEncoding,
    {
        let blob = self.finalize(signer)?;

        let signature = signer.try_sign(&blob)?.to_bitstring()?;

        self.assemble(signature, signer)
    }

    /// Run the object through the signer and build it.
    ///
    /// # Notes
    ///
    /// When using ECDSA signers, the `Signature` parameter will need to be explicit
    /// as multiple implementation of [`signature::Signer`] with various signature
    /// are available.
    ///
    /// This would look like:
    #[cfg_attr(feature = "std", doc = "```no_run")]
    #[cfg_attr(not(feature = "std"), doc = "```ignore")]
    /// # use p256::elliptic_curve::Generate;
    /// # use rand::rng;
    /// # use std::{
    /// #     str::FromStr,
    /// #     time::Duration
    /// # };
    /// # use x509_cert::{
    /// #     builder::{self, CertificateBuilder, Builder},
    /// #     name::Name,
    /// #     serial_number::SerialNumber,
    /// #     spki::SubjectPublicKeyInfo,
    /// #     time::Validity
    /// # };
    /// #
    /// # let mut rng = rng();
    /// # let signer = p256::ecdsa::SigningKey::generate_from_rng(&mut rng);
    /// # let builder = CertificateBuilder::new(
    /// #     builder::profile::cabf::Root::new(
    /// #         false,
    /// #         Name::from_str("CN=World domination corporation").unwrap()
    /// #     ).unwrap(),
    /// #     SerialNumber::from(42u32),
    /// #     Validity::from_now(Duration::new(5, 0)).unwrap(),
    /// #     SubjectPublicKeyInfo::from_key(signer.verifying_key()).unwrap()
    /// # ).unwrap();
    /// let certificate = builder.build_with_rng::<_, ecdsa::der::Signature<_>, _>(
    ///     &signer,
    ///     &mut rng
    /// ).unwrap();
    /// ```
    fn build_with_rng<S, Signature, R>(mut self, signer: &S, rng: &mut R) -> Result<Self::Output>
    where
        S: RandomizedSigner<Signature>,
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
        Signature: SignatureBitStringEncoding,
        R: CryptoRng + ?Sized,
    {
        let blob = self.finalize(signer)?;

        let signature = signer.try_sign_with_rng(rng, &blob)?.to_bitstring()?;

        self.assemble(signature, signer)
    }
}

impl<P> Builder for CertificateBuilder<P>
where
    P: BuilderProfile,
{
    type Output = Certificate;

    fn finalize<S>(&mut self, cert_signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        let verifying_key = cert_signer.verifying_key();
        let signer_pub = SubjectPublicKeyInfo::from_key(&verifying_key)?;

        self.tbs.signature = cert_signer.signature_algorithm_identifier()?;

        let mut default_extensions = self.profile.build_extensions(
            self.tbs.subject_public_key_info.owned_to_ref(),
            signer_pub.owned_to_ref(),
            &self.tbs,
        )?;

        self.extensions.append(&mut default_extensions);

        if !self.extensions.is_empty() {
            self.tbs.extensions = Some(self.extensions.clone());
        }

        if self.tbs.extensions.is_none() {
            if self.tbs.issuer_unique_id.is_some() || self.tbs.subject_unique_id.is_some() {
                self.tbs.version = Version::V2;
            } else {
                self.tbs.version = Version::V1;
            }
        }

        self.tbs.to_der().map_err(Error::from)
    }

    fn assemble<S>(self, signature: BitString, _signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        let signature_algorithm = self.tbs.signature.clone();

        Ok(Certificate {
            tbs_certificate: self.tbs,
            signature_algorithm,
            signature,
        })
    }
}

/// Trait for async X509 builders
///
/// This trait defines the interface between builder and the signers.
///
/// This is the async counterpart of [`Builder`].
#[allow(async_fn_in_trait)]
pub trait AsyncBuilder: Sized {
    /// Type built by this builder
    type Output: Sized;

    /// Assemble the final object from signature.
    fn assemble<S>(self, signature: BitString, signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey;

    /// Finalize and return a serialization of the object for signature.
    fn finalize<S>(&mut self, signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey;

    /// Run the object through the signer and build it.
    ///
    /// # Notes
    ///
    /// When using ECDSA signers, the `Signature` parameter will need to be explicit
    /// as multiple implementation of [`signature::AsyncSigner`] with various signature
    /// are available.
    ///
    /// This would look like:
    #[cfg_attr(feature = "std", doc = "```no_run")]
    #[cfg_attr(not(feature = "std"), doc = "```ignore")]
    /// # use p256::elliptic_curve::Generate;
    /// # use rand::rng;
    /// # use std::{
    /// #     str::FromStr,
    /// #     time::Duration
    /// # };
    /// # use x509_cert::{
    /// #     builder::{self, CertificateBuilder, AsyncBuilder},
    /// #     name::Name,
    /// #     serial_number::SerialNumber,
    /// #     spki::SubjectPublicKeyInfo,
    /// #     time::Validity
    /// # };
    /// #
    /// # async fn build() -> builder::Result<()> {
    /// # let mut rng = rng();
    /// # let signer = p256::ecdsa::SigningKey::generate_from_rng(&mut rng);
    /// # let builder = CertificateBuilder::new(
    /// #     builder::profile::cabf::Root::new(
    /// #         false,
    /// #         Name::from_str("CN=World domination corporation").unwrap()
    /// #     ).unwrap(),
    /// #     SerialNumber::from(42u32),
    /// #     Validity::from_now(Duration::new(5, 0)).unwrap(),
    /// #     SubjectPublicKeyInfo::from_key(signer.verifying_key()).unwrap()
    /// # ).unwrap();
    /// let certificate = builder.build_async::<_, ecdsa::der::Signature<_>>(&signer).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn build_async<S, Signature>(mut self, signer: &S) -> Result<Self::Output>
    where
        S: AsyncSigner<Signature>,
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
        Signature: SignatureBitStringEncoding,
    {
        let blob = self.finalize(signer)?;

        let signature = signer.sign_async(&blob).await?.to_bitstring()?;

        self.assemble(signature, signer)
    }

    /// Run the object through the signer and build it.
    ///
    /// # Notes
    ///
    /// When using ECDSA signers, the `Signature` parameter will need to be explicit
    /// as multiple implementation of [`signature::AsyncSigner`] with various signature
    /// are available.
    ///
    /// This would look like:
    #[cfg_attr(feature = "std", doc = "```no_run")]
    #[cfg_attr(not(feature = "std"), doc = "```ignore")]
    /// # use p256::elliptic_curve::Generate;
    /// # use rand::rng;
    /// # use std::{
    /// #     str::FromStr,
    /// #     time::Duration
    /// # };
    /// # use x509_cert::{
    /// #     builder::{self, CertificateBuilder, AsyncBuilder},
    /// #     name::Name,
    /// #     serial_number::SerialNumber,
    /// #     spki::SubjectPublicKeyInfo,
    /// #     time::Validity
    /// # };
    /// #
    /// # async fn build() -> builder::Result<()> {
    /// # let mut rng = rng();
    /// # let signer = p256::ecdsa::SigningKey::generate_from_rng(&mut rng);
    /// # let builder = CertificateBuilder::new(
    /// #     builder::profile::cabf::Root::new(
    /// #         false,
    /// #         Name::from_str("CN=World domination corporation").unwrap()
    /// #     ).unwrap(),
    /// #     SerialNumber::from(42u32),
    /// #     Validity::from_now(Duration::new(5, 0)).unwrap(),
    /// #     SubjectPublicKeyInfo::from_key(signer.verifying_key()).unwrap()
    /// # ).unwrap();
    /// let certificate = builder.build_with_rng_async::<_, ecdsa::der::Signature<_>, _>(&signer, &mut rng).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn build_with_rng_async<S, Signature, R>(
        mut self,
        signer: &S,
        rng: &mut R,
    ) -> Result<Self::Output>
    where
        S: AsyncRandomizedSigner<Signature>,
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
        Signature: SignatureBitStringEncoding,
        R: CryptoRng + ?Sized,
    {
        let blob = self.finalize(signer)?;

        let signature = signer
            .try_sign_with_rng_async(rng, &blob)
            .await?
            .to_bitstring()?;

        self.assemble(signature, signer)
    }
}

impl<T> AsyncBuilder for T
where
    T: Builder,
{
    type Output = <T as Builder>::Output;

    fn assemble<S>(self, signature: BitString, signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        <T as Builder>::assemble(self, signature, signer)
    }

    fn finalize<S>(&mut self, signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        <T as Builder>::finalize(self, signer)
    }
}

/// X.509 CRL builder
pub struct CrlBuilder<P = certificate::Rfc5280>
where
    P: certificate::Profile,
{
    tbs: TbsCertList<P>,
}

impl<P> CrlBuilder<P>
where
    P: certificate::Profile,
{
    /// Create a `CrlBuilder` with the given issuer and the given monotonic [`CrlNumber`]
    #[cfg(feature = "std")]
    pub fn new(issuer: &Certificate, crl_number: CrlNumber) -> der::Result<Self> {
        let this_update = Time::now()?;
        Self::new_with_this_update(issuer, crl_number, this_update)
    }

    /// Create a `CrlBuilder` with the given issuer, a given monotonic [`CrlNumber`], and valid
    /// from the given `this_update` start validity date.
    pub fn new_with_this_update(
        issuer: &Certificate,
        crl_number: CrlNumber,
        this_update: Time,
    ) -> der::Result<Self> {
        // Replaced later when the finalize is called
        let signature_alg = AlgorithmIdentifier {
            oid: NULL_OID,
            parameters: None,
        };

        let issuer_name = issuer.tbs_certificate.subject().clone();

        let mut crl_extensions = Extensions::new();
        crl_extensions.push(crl_number.to_extension(&issuer_name, &crl_extensions)?);
        let aki = match issuer
            .tbs_certificate
            .get_extension::<AuthorityKeyIdentifier>()?
        {
            Some((_, aki)) => aki,
            None => {
                let ski = SubjectKeyIdentifier::try_from(
                    issuer
                        .tbs_certificate
                        .subject_public_key_info()
                        .owned_to_ref(),
                )?;
                AuthorityKeyIdentifier {
                    // KeyIdentifier must be the same as subjectKeyIdentifier
                    key_identifier: Some(ski.0.clone()),
                    // other fields must not be present.
                    ..Default::default()
                }
            }
        };
        crl_extensions.push(aki.to_extension(&issuer_name, &crl_extensions)?);

        let tbs = TbsCertList {
            version: Version::V2,
            signature: signature_alg,
            issuer: issuer_name,
            this_update,
            next_update: None,
            revoked_certificates: None,
            crl_extensions: Some(crl_extensions),
        };

        Ok(Self { tbs })
    }

    /// Make the CRL valid until the given `next_update`
    pub fn with_next_update(mut self, next_update: Option<Time>) -> Self {
        self.tbs.next_update = next_update;
        self
    }

    /// Add certificates to the revocation list
    pub fn with_certificates<I>(mut self, revoked: I) -> Self
    where
        I: Iterator<Item = RevokedCert<P>>,
    {
        let certificates = self
            .tbs
            .revoked_certificates
            .get_or_insert_with(vec::Vec::new);

        let mut revoked: vec::Vec<RevokedCert<P>> = revoked.collect();
        certificates.append(&mut revoked);

        self
    }
}

impl<P> Builder for CrlBuilder<P>
where
    P: certificate::Profile,
{
    type Output = CertificateList<P>;

    fn finalize<S>(&mut self, cert_signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        self.tbs.signature = cert_signer.signature_algorithm_identifier()?;

        self.tbs.to_der().map_err(Error::from)
    }

    fn assemble<S>(self, signature: BitString, _signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        let signature_algorithm = self.tbs.signature.clone();

        Ok(CertificateList {
            tbs_cert_list: self.tbs,
            signature_algorithm,
            signature,
        })
    }
}
