//! The OpenPGP verifier over what two publishers really signed: Anthropic's release manifest (a detached signature by an RSA key) and
//! Node.js's `SHASUMS256.txt` (a clearsigned message by an Ed25519 key of the release team). The documents, signatures and keys are the
//! ones the two extensions ship and test with.
#![cfg(feature = "store")]

use canoa::store::openpgp::{fingerprints, verify_cleartext, verify_detached};

const CLAUDE_KEY: &str = include_str!("../../../extensions/canoa-claude/keys/claude-code.asc");
const MANIFEST: &str =
    include_str!("../../../extensions/canoa-claude/tests/fixtures/manifest_2.1.332.json");
const MANIFEST_SIGNATURE: &str =
    include_str!("../../../extensions/canoa-claude/tests/fixtures/manifest_2.1.332.json.sig");
const OTHER_SIGNATURE: &str =
    include_str!("../../../extensions/canoa-claude/tests/fixtures/manifest_2.1.326.json.sig");

const NODE_KEYS: &str = include_str!("../../../extensions/canoa-node/keys/release-keys.asc");
const SHASUMS: &str =
    include_str!("../../../extensions/canoa-node/tests/fixtures/SHASUMS256_v26.13.1.txt.asc");

#[test]
fn a_keys_fingerprint_is_the_one_its_publisher_names() {
    assert_eq!(
        fingerprints(&[CLAUDE_KEY]).expect("a key"),
        ["31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE"]
    );
}

#[test]
fn a_manifest_the_publisher_signed_verifies_and_names_the_signer() {
    let who = verify_detached(&[CLAUDE_KEY], MANIFEST.as_bytes(), MANIFEST_SIGNATURE)
        .expect("a good signature");
    assert_eq!(who.fingerprint, "31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE");
    assert_eq!(who.key_id, "BAA929FF1A7ECACE");
}

#[test]
fn one_changed_byte_a_signature_of_another_document_or_a_key_that_is_not_pinned_is_refused() {
    let changed = MANIFEST.replacen("2.1.332", "2.1.333", 1);
    let refused = verify_detached(&[CLAUDE_KEY], changed.as_bytes(), MANIFEST_SIGNATURE)
        .expect_err("another document");
    assert!(refused.contains("does not match"), "{refused}");
    let other = verify_detached(&[CLAUDE_KEY], MANIFEST.as_bytes(), OTHER_SIGNATURE)
        .expect_err("another release's signature");
    assert!(other.contains("does not match"), "{other}");
    let stranger = verify_detached(&[NODE_KEYS], MANIFEST.as_bytes(), MANIFEST_SIGNATURE)
        .expect_err("a key nobody pinned");
    assert!(stranger.contains("not one of the keys"), "{stranger}");
    assert!(verify_detached(&[CLAUDE_KEY], b"", "not armor").is_err());
}

#[test]
fn a_block_of_several_keys_is_every_key_in_it_and_the_clearsigned_checksums_verify() {
    let all = fingerprints(&[NODE_KEYS]).expect("keys");
    assert!(all.len() >= 8, "{all:?}");
    let (who, text) = verify_cleartext(&[NODE_KEYS], SHASUMS).expect("a good signature");
    assert_eq!(
        who.fingerprint, "5BE8A3F6C8A5C01D106C0AD820B1A390B168D356",
        "the signer is the release team's Ed25519 key"
    );
    assert!(text.starts_with("65e55fb13dd6d9d4"), "{text}");
    assert!(text.contains("node-v26.13.1-win-x64.zip"));
    assert!(!text.contains("PGP"), "only the signed text is answered");
}

#[test]
fn clearsigned_text_that_was_edited_is_refused() {
    let edited = SHASUMS.replacen("node-v26.13.1-win-x64.zip", "node-v26.13.1-win-x64.exe", 1);
    let refused = verify_cleartext(&[NODE_KEYS], &edited).expect_err("edited");
    assert!(refused.contains("does not match"), "{refused}");
    assert!(verify_cleartext(&[CLAUDE_KEY], SHASUMS).is_err());
    assert!(verify_cleartext(&[NODE_KEYS], "just text").is_err());
}
