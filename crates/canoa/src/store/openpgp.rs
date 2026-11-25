//! Verifying an OpenPGP signature made by a key the caller pins: what a catalog source does with a publisher's detached signature
//! (Anthropic signs each release manifest) or clearsigned checksums (Node signs `SHASUMS256.txt`). The extension that reads the
//! publisher ships the publisher's public key as data, so the keys here are the caller's own and nothing is looked up or trusted
//! beyond them.
//!
//! This reads the part of OpenPGP (RFC 4880, as RFC 9580 keeps it) that checking a signature needs: armor, packets, version 4
//! public keys (RSA and Ed25519) and version 4 signatures of a document or of canonical text, and checks the signature over the
//! document with the key it names. A key's expiry and revocation, its user ids and the binding of a subkey to its primary key are
//! not weighed: a key block the extension ships is the list of keys it trusts, and a signature made by any key in it counts. A
//! signature by MD5 or SHA-1, or by an algorithm not named here, is refused.
//!
//! No crate of the OpenPGP family is used: they bring the previous generation of the RustCrypto crates beside the one this
//! workspace holds, and read far more than a signature needs. The checking is `rsa`'s PKCS#1 v1.5 and `ed25519-dalek`'s, over
//! digests of `sha2`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rsa::BoxedUint;
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature as RsaSignature, VerifyingKey as RsaVerifyingKey};
use rsa::signature::hazmat::PrehashVerifier as _;
use sha2::{Digest as _, Sha256, Sha384, Sha512};

/// Who made a signature that checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Signer {
    /// The fingerprint of the key that made it, in upper-case hexadecimal.
    pub fingerprint: String,
    /// The key's 64-bit id, in upper-case hexadecimal.
    pub key_id: String,
}

enum Material {
    Rsa(RsaPublicKey),
    Ed25519(ed25519_dalek::VerifyingKey),
}

struct Key {
    fingerprint: [u8; 20],
    material: Material,
}

impl Key {
    fn signer(&self) -> Signer {
        Signer {
            fingerprint: hex_upper(&self.fingerprint),
            key_id: hex_upper(&self.fingerprint[12..]),
        }
    }
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// An ASCII-armored block's bytes, whatever its type.
fn dearmor(text: &str) -> Result<Vec<u8>, String> {
    let mut lines = text.lines().map(str::trim_end);
    lines
        .by_ref()
        .find(|line| line.starts_with("-----BEGIN PGP"))
        .ok_or("no armored block")?;
    // The armor headers end at the first blank line.
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
    }
    let mut body = String::new();
    for line in lines {
        if line.starts_with('=') || line.starts_with("-----END") {
            break;
        }
        body.push_str(line.trim());
    }
    data_encoding::BASE64
        .decode(body.as_bytes())
        .map_err(|error| format!("the armor is not base64: {error}"))
}

/// The packets of `bytes`, as `(tag, body)`.
fn packets(bytes: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let head = bytes[at];
        if head & 0x80 == 0 {
            return Err("a packet does not start where one should".to_owned());
        }
        at += 1;
        let (tag, length) = if head & 0x40 != 0 {
            let first = *bytes.get(at).ok_or("a packet is cut short")?;
            at += 1;
            let length = match first {
                0..=191 => usize::from(first),
                192..=223 => {
                    let second = *bytes.get(at).ok_or("a packet is cut short")?;
                    at += 1;
                    ((usize::from(first) - 192) << 8) + usize::from(second) + 192
                }
                255 => {
                    let four = bytes.get(at..at + 4).ok_or("a packet is cut short")?;
                    at += 4;
                    u32::from_be_bytes([four[0], four[1], four[2], four[3]]) as usize
                }
                _ => return Err("a packet of partial length is not read".to_owned()),
            };
            (head & 0x3F, length)
        } else {
            let (width, tag) = match head & 0x03 {
                0 => (1, (head >> 2) & 0x0F),
                1 => (2, (head >> 2) & 0x0F),
                2 => (4, (head >> 2) & 0x0F),
                _ => return Err("a packet of indeterminate length is not read".to_owned()),
            };
            let size = bytes.get(at..at + width).ok_or("a packet is cut short")?;
            at += width;
            (
                tag,
                size.iter()
                    .fold(0usize, |sum, byte| (sum << 8) | usize::from(*byte)),
            )
        };
        let body = bytes.get(at..at + length).ok_or("a packet is cut short")?;
        at += length;
        found.push((tag, body));
    }
    Ok(found)
}

/// One multi-precision integer: its bits as a count, then its bytes.
fn mpi<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a [u8], String> {
    let count = bytes.get(*at..*at + 2).ok_or("an integer is cut short")?;
    let bits = usize::from(u16::from_be_bytes([count[0], count[1]]));
    let length = bits.div_ceil(8);
    *at += 2;
    let value = bytes
        .get(*at..*at + length)
        .ok_or("an integer is cut short")?;
    *at += length;
    Ok(value)
}

fn uint(bytes: &[u8]) -> Result<BoxedUint, String> {
    let bits = (bytes.len() * 8).div_ceil(64).max(1) * 64;
    BoxedUint::from_be_slice(
        bytes,
        u32::try_from(bits).map_err(|_| "an integer is too large")?,
    )
    .map_err(|_| "an integer is not one".to_owned())
}

/// The armored blocks of `text`, which may hold several one after another.
fn blocks(text: &str) -> Vec<&str> {
    const BEGIN: &str = "-----BEGIN PGP";
    let starts: Vec<usize> = text.match_indices(BEGIN).map(|(at, _)| at).collect();
    starts
        .iter()
        .enumerate()
        .map(|(each, start)| &text[*start..starts.get(each + 1).copied().unwrap_or(text.len())])
        .collect()
}

/// The version 4 public keys (and subkeys) of the armored blocks of `armored`.
fn keys_of(armored: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for block in blocks(armored) {
        keys_in(block, &mut keys)?;
    }
    if keys.is_empty() {
        return Err("the text holds no key this reads (RSA or Ed25519, version 4)".to_owned());
    }
    Ok(keys)
}

fn keys_in(armored: &str, keys: &mut Vec<Key>) -> Result<(), String> {
    let bytes = dearmor(armored)?;
    for (tag, body) in packets(&bytes)? {
        if tag != 6 && tag != 14 {
            continue;
        }
        if body.first() != Some(&4) || body.len() < 6 {
            continue;
        }
        let algorithm = body[5];
        let mut at = 6;
        let material = match algorithm {
            1 | 3 => {
                let n = mpi(body, &mut at)?;
                let e = mpi(body, &mut at)?;
                let key = RsaPublicKey::new(uint(n)?, uint(e)?)
                    .map_err(|error| format!("an RSA key is not one: {error}"))?;
                Material::Rsa(key)
            }
            22 => {
                let length = usize::from(*body.get(at).ok_or("a key is cut short")?);
                at += 1 + length;
                let point = mpi(body, &mut at)?;
                let raw = point
                    .strip_prefix(&[0x40])
                    .filter(|raw| raw.len() == 32)
                    .ok_or("an Ed25519 key is not 32 bytes")?;
                let mut fixed = [0u8; 32];
                fixed.copy_from_slice(raw);
                Material::Ed25519(
                    ed25519_dalek::VerifyingKey::from_bytes(&fixed)
                        .map_err(|error| format!("an Ed25519 key is not one: {error}"))?,
                )
            }
            _ => continue,
        };
        // The fingerprint of a version 4 key: SHA-1 of 0x99, the packet's length as two bytes and the packet.
        let mut hash = sha1::Sha1::new();
        hash.update([0x99]);
        hash.update(u16::try_from(body.len()).unwrap_or(u16::MAX).to_be_bytes());
        hash.update(body);
        let mut fingerprint = [0u8; 20];
        fingerprint.copy_from_slice(&hash.finalize());
        keys.push(Key {
            fingerprint,
            material,
        });
    }
    Ok(())
}

/// The fingerprints of the keys in the armored blocks `keys`, in upper-case hexadecimal: what a source names as the keys it
/// verifies with.
///
/// # Errors
///
/// Why a block is not armor, or holds no key this reads.
pub fn fingerprints(keys: &[&str]) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for block in keys {
        found.extend(keys_of(block)?.iter().map(|key| key.signer().fingerprint));
    }
    Ok(found)
}

struct Signature<'a> {
    kind: u8,
    algorithm: u8,
    hash: u8,
    /// The signature's version, type, algorithms and hashed subpackets, which the digest covers.
    header: &'a [u8],
    issuer: Option<Vec<u8>>,
    prefix: [u8; 2],
    material: &'a [u8],
}

fn signature_of(body: &[u8]) -> Result<Signature<'_>, String> {
    if body.first() != Some(&4) {
        return Err("only version 4 signatures are read".to_owned());
    }
    if body.len() < 6 {
        return Err("a signature is cut short".to_owned());
    }
    let hashed_length = usize::from(u16::from_be_bytes([body[4], body[5]]));
    let hashed = body
        .get(6..6 + hashed_length)
        .ok_or("a signature is cut short")?;
    let header = &body[..6 + hashed_length];
    let mut at = 6 + hashed_length;
    let unhashed_length = {
        let two = body.get(at..at + 2).ok_or("a signature is cut short")?;
        usize::from(u16::from_be_bytes([two[0], two[1]]))
    };
    at += 2;
    let unhashed = body
        .get(at..at + unhashed_length)
        .ok_or("a signature is cut short")?;
    at += unhashed_length;
    let prefix = body.get(at..at + 2).ok_or("a signature is cut short")?;
    let issuer = [hashed, unhashed]
        .into_iter()
        .find_map(|subpackets| issuer_of(subpackets).transpose())
        .transpose()?;
    Ok(Signature {
        kind: body[1],
        algorithm: body[2],
        hash: body[3],
        header,
        issuer,
        prefix: [prefix[0], prefix[1]],
        material: &body[at + 2..],
    })
}

/// The issuer a run of subpackets names: a fingerprint (the last 20 bytes of its subpacket) or a 64-bit key id.
fn issuer_of(subpackets: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut at = 0;
    let mut id = None;
    while at < subpackets.len() {
        let first = subpackets[at];
        let (length, used) = match first {
            0..=191 => (usize::from(first), 1),
            192..=254 => {
                let second = *subpackets.get(at + 1).ok_or("a subpacket is cut short")?;
                (
                    ((usize::from(first) - 192) << 8) + usize::from(second) + 192,
                    2,
                )
            }
            255 => {
                let four = subpackets
                    .get(at + 1..at + 5)
                    .ok_or("a subpacket is cut short")?;
                (
                    u32::from_be_bytes([four[0], four[1], four[2], four[3]]) as usize,
                    5,
                )
            }
        };
        let body = subpackets
            .get(at + used..at + used + length)
            .ok_or("a subpacket is cut short")?;
        at += used + length;
        match body.first().map(|kind| kind & 0x7F) {
            Some(33) if body.len() == 22 => return Ok(Some(body[2..].to_vec())),
            Some(16) if body.len() == 9 => id = Some(body[1..].to_vec()),
            _ => {}
        }
    }
    Ok(id)
}

/// The digest a signature covers: the document, then the signature's own header and a trailer that says how long it was.
fn digest(hash: u8, data: &[u8], header: &[u8]) -> Result<Vec<u8>, String> {
    let mut trailer = alloc::vec![0x04, 0xFF];
    trailer.extend_from_slice(
        &u32::try_from(header.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    macro_rules! hashed {
        ($algorithm:ty) => {{
            let mut each = <$algorithm>::new();
            each.update(data);
            each.update(header);
            each.update(&trailer);
            each.finalize().to_vec()
        }};
    }
    match hash {
        8 => Ok(hashed!(Sha256)),
        9 => Ok(hashed!(Sha384)),
        10 => Ok(hashed!(Sha512)),
        other => Err(format!(
            "a signature by hash {other} is refused: SHA-256, SHA-384 and SHA-512 are read"
        )),
    }
}

fn pad(bytes: &[u8], length: usize) -> Result<Vec<u8>, String> {
    let bytes = bytes
        .iter()
        .copied()
        .skip_while(|byte| *byte == 0)
        .collect::<Vec<u8>>();
    if bytes.len() > length {
        return Err("a signature is longer than its key".to_owned());
    }
    let mut padded = alloc::vec![0u8; length - bytes.len()];
    padded.extend_from_slice(&bytes);
    Ok(padded)
}

fn check(key: &Key, signature: &Signature<'_>, digest: &[u8]) -> Result<(), String> {
    let mut at = 0;
    match (&key.material, signature.algorithm) {
        (Material::Rsa(public), 1 | 3) => {
            use rsa::traits::PublicKeyParts as _;
            let value = mpi(signature.material, &mut at)?;
            let bytes = pad(value, public.size())?;
            let signature_bytes = RsaSignature::try_from(bytes.as_slice())
                .map_err(|error| format!("the signature is not an RSA one: {error}"))?;
            let outcome = match signature.hash {
                8 => RsaVerifyingKey::<Sha256>::new(public.clone())
                    .verify_prehash(digest, &signature_bytes),
                9 => RsaVerifyingKey::<Sha384>::new(public.clone())
                    .verify_prehash(digest, &signature_bytes),
                _ => RsaVerifyingKey::<Sha512>::new(public.clone())
                    .verify_prehash(digest, &signature_bytes),
            };
            outcome.map_err(|_| "the signature does not match the document".to_owned())
        }
        (Material::Ed25519(public), 22) => {
            let r = pad(mpi(signature.material, &mut at)?, 32)?;
            let s = pad(mpi(signature.material, &mut at)?, 32)?;
            let mut raw = [0u8; 64];
            raw[..32].copy_from_slice(&r);
            raw[32..].copy_from_slice(&s);
            public
                .verify_strict(digest, &ed25519_dalek::Signature::from_bytes(&raw))
                .map_err(|_| "the signature does not match the document".to_owned())
        }
        _ => Err("the key and the signature are of different algorithms".to_owned()),
    }
}

/// Checks that the armored signature `signature` over `data` was made by a key in one of the armored blocks `keys`.
///
/// # Errors
///
/// Why the signature is no signature this reads, names a key that is not among `keys`, or does not match `data`.
pub fn verify_detached(keys: &[&str], data: &[u8], signature: &str) -> Result<Signer, String> {
    let bytes = dearmor(signature)?;
    let found = packets(&bytes)?;
    let (_, body) = found
        .iter()
        .find(|(tag, _)| *tag == 2)
        .ok_or("the block holds no signature")?;
    verify(keys, data, body)
}

fn verify(keys: &[&str], data: &[u8], body: &[u8]) -> Result<Signer, String> {
    let signature = signature_of(body)?;
    if signature.kind != 0x00 && signature.kind != 0x01 {
        return Err(format!(
            "a signature of type {} is not one of a document",
            signature.kind
        ));
    }
    let mut all = Vec::new();
    for block in keys {
        all.extend(keys_of(block)?);
    }
    let candidates: Vec<&Key> = all
        .iter()
        .filter(|key| match &signature.issuer {
            Some(issuer) if issuer.len() == 20 => key.fingerprint.as_slice() == issuer.as_slice(),
            Some(issuer) => key.fingerprint[12..] == issuer[..],
            None => true,
        })
        .collect();
    let Some(key) = candidates.first() else {
        let named = signature
            .issuer
            .as_deref()
            .map_or_else(|| "no key".to_owned(), hex_upper);
        return Err(format!(
            "the signature is by {named}, which is not one of the keys this accepts"
        ));
    };
    let digest = digest(signature.hash, data, signature.header)?;
    if digest.get(..2) != Some(&signature.prefix[..]) {
        return Err("the signature does not match the document".to_owned());
    }
    check(key, &signature, &digest)?;
    Ok(key.signer())
}

/// Checks the clearsigned message `message` (`-----BEGIN PGP SIGNED MESSAGE-----`) against the keys, and answers who signed it and
/// the text the signature covers, with the dash-escaping undone and the line endings as `\n`.
///
/// # Errors
///
/// Why the message is not clearsigned, or its signature is refused as in [`verify_detached`].
pub fn verify_cleartext(keys: &[&str], message: &str) -> Result<(Signer, String), String> {
    let mut lines = message.lines();
    if lines.next().map(str::trim_end) != Some("-----BEGIN PGP SIGNED MESSAGE-----") {
        return Err("the text is not a clearsigned message".to_owned());
    }
    // The armor headers (`Hash: SHA256`) end at a blank line.
    for line in lines.by_ref() {
        if line.trim_end().is_empty() {
            break;
        }
    }
    let mut text: Vec<String> = Vec::new();
    let mut signature = String::new();
    let mut in_signature = false;
    for line in lines {
        if in_signature {
            signature.push_str(line);
            signature.push('\n');
        } else if line.trim_end() == "-----BEGIN PGP SIGNATURE-----" {
            in_signature = true;
            signature.push_str(line);
            signature.push('\n');
        } else {
            text.push(line.strip_prefix("- ").unwrap_or(line).to_owned());
        }
    }
    if !in_signature {
        return Err("the message has no signature".to_owned());
    }
    // What is signed: the lines with trailing spaces and tabs taken off, joined by CR LF, the last line's ending left out.
    let signed: Vec<&str> = text
        .iter()
        .map(|line| line.trim_end_matches([' ', '\t', '\r']))
        .collect();
    let canonical = signed.join("\r\n");
    let bytes = dearmor(&signature)?;
    let found = packets(&bytes)?;
    let (_, body) = found
        .iter()
        .find(|(tag, _)| *tag == 2)
        .ok_or("the block holds no signature")?;
    let who = verify(keys, canonical.as_bytes(), body)?;
    Ok((who, signed.join("\n") + "\n"))
}

/// A short form of a fingerprint for a person to read: its last 16 hexadecimal digits.
#[must_use]
pub fn short(fingerprint: &str) -> String {
    fingerprint
        .get(fingerprint.len().saturating_sub(16)..)
        .unwrap_or(fingerprint)
        .to_string()
}
