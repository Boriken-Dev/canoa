//! The application's half of a sealed question, played in a test: what the person types is sealed to the key the host asked
//! with, and every line the link carried is kept to be searched for what must never be in it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    dead_code,
    reason = "tests name what they use from std, and not every test uses all of it"
)]

use std::time::SystemTime;

use coqui_acp::secret::{
    DEFAULT_LIFETIME, IdentityId, LinkKey, LinkRole, Method, Prover, RequestParams, RequestReply,
    Verifier, unprotected,
};
use coqui_link::auth::Identity;
use serde_json::{Value, json};

/// What the person does when the host puts a form.
pub enum Typed {
    /// Fills it in.
    Fills(&'static [(&'static str, &'static str)]),
    /// Declines.
    Declines,
    /// Answers with the values sealed again after they were bound, so the binding covers other bytes.
    Forged(&'static [(&'static str, &'static str)]),
    /// Closes the link instead of answering.
    Leaves,
}

/// The application's side of the link: its half of the binding, and what it saw.
pub struct App {
    pub prover: Prover,
    pub verifier: Verifier,
    pub host: IdentityId,
    /// Every line the link carried, either way.
    pub traffic: Vec<String>,
}

impl App {
    pub fn new(secret: &[u8]) -> Self {
        let key = LinkKey::derive(secret).expect("a key");
        let app = key.identity(LinkRole::App);
        Self {
            prover: Prover::linked(&key, LinkRole::App),
            verifier: Verifier::builder(app)
                .link(&key, LinkRole::App)
                .expect("the link's")
                .build(),
            host: key.identity(LinkRole::Host),
            traffic: Vec::new(),
        }
    }

    /// The half of a paired key: its own key proves the answer, and the host it trusts is the one whose key is `host_spki`
    /// (a `SubjectPublicKeyInfo`).
    pub fn for_key(client: &Identity, host_spki: &[u8]) -> Self {
        let pem = match client {
            Identity::WebCrypto(identity) => identity.to_pkcs8_pem().expect("a PEM"),
            Identity::Noise(identity) => identity.to_pkcs8_pem().expect("a PEM"),
            _ => panic!("a method these tests do not play"),
        };
        let der = coqui_peer::pem::decode(&pem).expect("PEM").bytes;
        let host = IdentityId::of(host_spki);
        let (prover, verifier) = match client {
            Identity::WebCrypto(_) => {
                let prover = unprotected::ecdsa_p256_pkcs8(&der).expect("a P-256 key");
                let verifier = Verifier::builder(prover.id()).trust(host).build();
                (prover, verifier)
            }
            _ => {
                let secret = &der[der.len() - 32..];
                let prover = unprotected::x25519_pkcs8(&der, host_spki).expect("an X25519 key");
                let (spki, agree) = unprotected::x25519_agreement(secret).expect("an agreement");
                let verifier = Verifier::builder(prover.id())
                    .trust(host)
                    .agreement(&spki, agree)
                    .expect("an agreement")
                    .build();
                (prover, verifier)
            }
        };
        Self {
            prover,
            verifier,
            host,
            traffic: Vec::new(),
        }
    }

    /// The answer to the host's request `params`, as the person would give it.
    pub fn answer(&self, params: &Value, typed: &Typed) -> Option<Value> {
        let (values, forged) = match typed {
            Typed::Fills(values) => (values, false),
            Typed::Forged(values) => (values, true),
            Typed::Declines => return Some(RequestReply::Decline.to_value()),
            Typed::Leaves => return None,
        };
        let params = RequestParams::from_value(params).expect("the host's request");
        let now = unix_now();
        let collector = self
            .verifier
            .collector(
                &self.host,
                Method::Request,
                &params.context,
                &params.recipient,
                now,
            )
            .expect("a key bound to the host");
        let mut sealed: std::collections::BTreeMap<_, _> = values
            .iter()
            .filter(|(name, _)| collector.is_secret(name))
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    collector.seal(name, value).expect("sealed"),
                )
            })
            .collect();
        let plain: serde_json::Map<String, Value> = values
            .iter()
            .filter(|(name, _)| !collector.is_secret(name))
            .map(|(name, value)| ((*name).to_owned(), json!(value)))
            .collect();
        let plain = Value::Object(plain).to_string();
        let auth = collector
            .authenticate(&self.prover, &plain, &sealed, now, DEFAULT_LIFETIME)
            .expect("bound");
        if forged {
            for (name, value) in &mut sealed {
                *value = collector.seal(name, "another value").expect("sealed");
            }
        }
        Some(
            RequestReply::Accept {
                plain,
                sealed,
                auth,
            }
            .to_value(),
        )
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs()
}

/// Every form `key` could be printed in.
pub fn forms(key: &str) -> Vec<(&'static str, Vec<u8>)> {
    let json = serde_json::to_string(key).expect("JSON");
    vec![
        ("raw", key.as_bytes().to_vec()),
        ("json", json.as_bytes()[1..json.len() - 1].to_vec()),
        (
            "base64",
            data_encoding::BASE64.encode(key.as_bytes()).into_bytes(),
        ),
        (
            "base64url",
            data_encoding::BASE64URL_NOPAD
                .encode(key.as_bytes())
                .into_bytes(),
        ),
        (
            "hex",
            data_encoding::HEXLOWER.encode(key.as_bytes()).into_bytes(),
        ),
        (
            "HEX",
            data_encoding::HEXUPPER.encode(key.as_bytes()).into_bytes(),
        ),
        (
            "percent",
            percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
                .into_bytes(),
        ),
    ]
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Where `key` shows, in any form, among `places` (a name and its bytes).
pub fn found_in(key: &str, places: &[(String, Vec<u8>)]) -> Vec<String> {
    let mut hits = Vec::new();
    for (place, bytes) in places {
        for (form, needle) in forms(key) {
            if contains(bytes, &needle) {
                hits.push(format!("{place} holds the key as {form}"));
            }
        }
    }
    hits
}
