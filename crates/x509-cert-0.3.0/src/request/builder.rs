use alloc::vec;

use der::{Encode, asn1::BitString};
use signature::Keypair;
use spki::{
    AlgorithmIdentifier, DynSignatureAlgorithmIdentifier, EncodePublicKey, SubjectPublicKeyInfo,
};

use crate::{
    builder::{Builder, Error, NULL_OID, Result},
    ext::ToExtension,
    name::Name,
    request::{CertReq, CertReqInfo, ExtensionReq, attributes::AsAttribute},
};

/// Builder for X509 Certificate Requests (CSR)
///
/// ```
/// # use p256::{pkcs8::DecodePrivateKey, NistP256, ecdsa::DerSignature};
/// # const PKCS8_PRIVATE_KEY_DER: &[u8] = &[0x30u8,0x81,0x87,0x02,0x01,0x00,0x30,0x13,0x06,0x07,0x2a,0x86,0x48,0xce,0x3d,0x02,0x01,0x06,0x08,0x2a,0x86,0x48,0xce,0x3d,0x03,0x01,0x07,0x04,0x6d,0x30,0x6b,0x02,0x01,0x01,0x04,0x20,0x69,0x62,0x41,0x71,0x56,0x1a,0x63,0x34,0x0d,0xe0,0xe7,0xd8,0x69,0xf2,0xa0,0x54,0x92,0x55,0x8e,0x1a,0x04,0x86,0x8b,0x6a,0x9f,0x85,0x4a,0x86,0x67,0x88,0x18,0x8d,0xa1,0x44,0x03,0x42,0x00,0x04,0x1c,0xac,0xff,0xb5,0x5f,0x2f,0x2c,0xef,0xd8,0x9d,0x89,0xeb,0x37,0x4b,0x26,0x81,0x15,0x24,0x52,0x80,0x2d,0xee,0xa0,0x99,0x16,0x06,0x81,0x37,0xd8,0x39,0xcf,0x7f,0xc4,0x81,0xa4,0x44,0x92,0x30,0x4d,0x7e,0xf6,0x6a,0xc1,0x17,0xbe,0xfe,0x83,0xa8,0xd0,0x8f,0x15,0x5f,0x2b,0x52,0xf9,0xf6,0x18,0xdd,0x44,0x70,0x29,0x04,0x8e,0x0f];
/// # fn ecdsa_signer() -> ecdsa::SigningKey<NistP256> {
/// #     let secret_key = p256::SecretKey::from_pkcs8_der(PKCS8_PRIVATE_KEY_DER).unwrap();
/// #     ecdsa::SigningKey::from(secret_key)
/// # }
/// use x509_cert::{
///     builder::{Builder, RequestBuilder},
///     ext::pkix::{name::GeneralName, SubjectAltName},
///     name::Name,
/// };
/// use std::str::FromStr;
///
/// use std::net::{IpAddr, Ipv4Addr};
/// let subject = Name::from_str("CN=service.domination.world").unwrap();
///
/// let signer = ecdsa_signer();
/// let mut builder = RequestBuilder::new(subject).expect("Create certificate request");
/// builder
///     .add_extension(&SubjectAltName(vec![GeneralName::from(IpAddr::V4(
///         Ipv4Addr::new(192, 0, 2, 0),
///     ))]))
///     .unwrap();
///
/// let cert_req = builder.build::<_, DerSignature>(&signer).unwrap();
/// ```
pub struct RequestBuilder {
    info: CertReqInfo,
    extension_req: ExtensionReq,
}

impl RequestBuilder {
    /// Creates a new certificate request builder
    pub fn new(subject: Name) -> Result<Self> {
        let version = Default::default();

        let algorithm = AlgorithmIdentifier {
            oid: NULL_OID,
            parameters: None,
        };
        let public_key = SubjectPublicKeyInfo {
            algorithm,
            subject_public_key: BitString::from_bytes(&[]).expect("unable to parse empty object"),
        };

        let attributes = Default::default();
        let extension_req = Default::default();

        Ok(Self {
            info: CertReqInfo {
                version,
                subject,
                public_key,
                attributes,
            },
            extension_req,
        })
    }

    /// Add an extension to this certificate request
    ///
    /// Extensions need to implement [`ToExtension`], examples may be found in
    /// in [`ToExtension` documentation](../ext/trait.ToExtension.html#examples) or
    /// [the implementors](../ext/trait.ToExtension.html#implementors).
    pub fn add_extension<E: ToExtension>(
        &mut self,
        extension: E,
    ) -> core::result::Result<(), E::Error> {
        let ext = extension.to_extension(&self.info.subject, &self.extension_req.0)?;

        self.extension_req.0.push(ext);

        Ok(())
    }

    /// Add an attribute to this certificate request
    pub fn add_attribute<A: AsAttribute>(&mut self, attribute: &A) -> Result<()> {
        let attr = attribute.to_attribute()?;

        self.info.attributes.insert(attr)?;
        Ok(())
    }
}

impl Builder for RequestBuilder {
    type Output = CertReq;

    fn finalize<S>(&mut self, signer: &S) -> Result<vec::Vec<u8>>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        let verifying_key = signer.verifying_key();
        let public_key = SubjectPublicKeyInfo::from_key(&verifying_key)?;
        self.info.public_key = public_key;

        self.info
            .attributes
            .insert(self.extension_req.clone().try_into()?)?;

        self.info.to_der().map_err(Error::from)
    }

    fn assemble<S>(self, signature: BitString, signer: &S) -> Result<Self::Output>
    where
        S: Keypair + DynSignatureAlgorithmIdentifier,
        S::VerifyingKey: EncodePublicKey,
    {
        let algorithm = signer.signature_algorithm_identifier()?;

        Ok(CertReq {
            info: self.info,
            algorithm,
            signature,
        })
    }
}
