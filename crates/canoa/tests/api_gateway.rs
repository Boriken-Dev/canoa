//! The account kind of an address that speaks several standards: its settings, the sign-in that makes an account (the key asked in a
//! sealed form and kept out of every answer), and the model list read from a loopback stand-in for the address.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::api_gateway::{
    self, ApiGatewayAccounts, CREDENTIAL, KIND, Settings, models_url, parse_standards,
};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::standard::{ANTHROPIC_API, OPENAI_API};
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountLoss, AccountModelsQuery, AccountProvider, AccountRenewal,
    Credentials, LoginAnswer, LoginForm, LoginPrompter, LoginRequest, OpenUrl, ProcessSpawner,
    UnknownKind,
};
use coqui_acp_harness::LocalSpawner;
use serde_json::{Map, Value, json};

/// The key of every test: nothing may show it but the request that is meant to carry it.
const CANARY: &str = "CANARY-gateway-key";

/// A prompter that answers the key form with `key` and records what it was asked.
struct Typing {
    key: String,
    forms: Arc<Mutex<Vec<String>>>,
}

impl LoginPrompter for Typing {
    fn say(&self, _text: &str) -> Result<(), ProviderError> {
        Ok(())
    }

    fn open_url(&self, _open: OpenUrl) -> Result<(), ProviderError> {
        Ok(())
    }

    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, ProviderError> {
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(form.title);
        let mut secrets = Credentials::new();
        secrets.insert(CREDENTIAL, format!("  {}\n", self.key));
        Ok(LoginAnswer::new(api_gateway::value(&json!({})), secrets))
    }

    fn cancelled(&self) -> Result<bool, ProviderError> {
        Ok(false)
    }
}

struct NoKeeper;

impl AccountKeeper for NoKeeper {
    fn renewed(&self, _renewal: AccountRenewal) -> Result<(), ProviderError> {
        Ok(())
    }

    fn lost(&self, _loss: AccountLoss) -> Result<(), ProviderError> {
        Ok(())
    }
}

fn spawner() -> Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}

/// A gateway on loopback: it answers `/v1/models` with the models it was given when the request carries `Bearer CANARY`, 401 when
/// it does not, and records each request line and its `Authorization` header.
fn stand_in(models: &[&str]) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let record = Arc::clone(&seen);
    let models: Vec<String> = models.iter().map(|m| (*m).to_owned()).collect();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (record, models) = (Arc::clone(&record), models.clone());
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let line = head.lines().next().unwrap_or("").to_owned();
                // The header's name is the client's to spell.
                let auth = head
                    .lines()
                    .find_map(|each| {
                        let (name, value) = each.split_once(':')?;
                        name.eq_ignore_ascii_case("authorization")
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap_or_default();
                record
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((line.clone(), auth.clone()));
                let (status, body) = if !line.starts_with("GET /v1/models ") {
                    ("404 Not Found", json!({"error": "no such path"}))
                } else if auth != format!("Bearer {CANARY}") {
                    ("401 Unauthorized", json!({"error": "bad key"}))
                } else {
                    (
                        "200 OK",
                        json!({"object": "list", "data": models.iter().map(|id| json!({"id": id, "object": "model"})).collect::<Vec<_>>()}),
                    )
                };
                let body = body.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    (address, seen)
}

fn sign_in(inputs: &Value, key: &str) -> Result<coqui_acp_api::LoginOutcome, ProviderError> {
    let request = LoginRequest::new(KIND, "unused").inputs(api_gateway::value(inputs));
    let prompter = Typing {
        key: key.to_owned(),
        forms: Arc::default(),
    };
    ApiGatewayAccounts.login(request, spawner(), prompter.into_object())
}

fn grant(address: &str, provides: &[&str]) -> AccountGrant {
    let mut credentials = Credentials::new();
    credentials.insert(CREDENTIAL, CANARY);
    AccountGrant::new("id-one", KIND)
        .settings(api_gateway::value(&json!({"base_url": address})))
        .credentials(credentials)
        .provides(provides.iter().copied())
}

fn models(grant: AccountGrant) -> coqui_acp_api::ModelListing {
    ApiGatewayAccounts
        .models(
            AccountModelsQuery::new(grant),
            spawner(),
            NoKeeper.into_object(),
        )
        .expect("an answer")
}

#[test]
fn the_kind_names_the_property_that_holds_the_standards_and_asks_the_address_a_host_may_send_a_key_to()
 {
    let offer = api_gateway::offer();
    let kind = offer.kind(KIND).expect("the kind");
    assert!(
        kind.provides.is_empty(),
        "the account says what it provides"
    );
    assert_eq!(kind.endpoints.len(), 1);
    assert_eq!(kind.endpoints[0].key.as_deref(), Some("base_url"));
    assert_eq!(offer.logins.len(), 1);
    assert_eq!(offer.logins[0].produces, KIND);
    let schema = api_gateway::settings_schema();
    assert_eq!(schema["properties"]["standards"]["x-provides"], json!(true));
    assert_eq!(schema["properties"][CREDENTIAL]["x-sensitive"], json!(true));
    assert!(offer.usage.is_empty(), "no plan limits are reported");
}

#[test]
fn settings_name_an_address_and_standards_and_nothing_else() {
    let table =
        |value: Value| -> Map<String, Value> { value.as_object().cloned().expect("a table") };
    let ok = Settings::from_map(&table(
        json!({"base_url": "https://llm.example.com/", "standards": "openai-api, anthropic-api"}),
    ))
    .expect("reads");
    assert_eq!(ok.base_url, "https://llm.example.com");
    assert_eq!(ok.standards, [OPENAI_API, ANTHROPIC_API]);
    let listed = Settings::from_map(&table(
        json!({"base_url": "http://127.0.0.1:4000", "standards": ["openai-api", "openai-api", "x-api"]}),
    ))
    .expect("a list reads");
    assert_eq!(listed.standards, [OPENAI_API, "x-api"]);
    for bad in [
        json!({"base_url": "http://llm.example.com", "standards": "openai-api"}),
        json!({"base_url": "https://u:p@llm.example.com", "standards": "openai-api"}),
        json!({"base_url": "https://llm.example.com", "standards": ""}),
        json!({"base_url": "https://llm.example.com", "standards": "Not An Id"}),
        json!({"standards": "openai-api"}),
        json!({"base_url": "https://llm.example.com", "standards": "openai-api", "key": "x"}),
    ] {
        assert!(Settings::from_map(&table(bad.clone())).is_err(), "{bad}");
    }
    assert_eq!(
        parse_standards("a-api b-api,c-api").expect("three").len(),
        3
    );
    assert_eq!(
        models_url("https://g.example/"),
        "https://g.example/v1/models"
    );
    assert_eq!(
        models_url("https://g.example/v1"),
        "https://g.example/v1/models"
    );
}

#[test]
fn signing_in_makes_an_account_of_the_standards_named_and_keeps_the_key_out_of_its_settings() {
    let outcome = sign_in(
        &json!({"base_url": "https://llm.example.com", "standards": "openai-api anthropic-api"}),
        CANARY,
    )
    .expect("signed in");
    assert_eq!(outcome.kind, KIND);
    assert_eq!(outcome.label.as_deref(), Some("llm.example.com"));
    let settings = guatiao_serde::adapt::from_value::<Map<String, Value>>(&outcome.settings)
        .expect("settings");
    assert_eq!(
        Value::Object(settings.clone()),
        json!({"base_url": "https://llm.example.com", "standards": ["openai-api", "anthropic-api"]})
    );
    assert!(!format!("{settings:?}").contains(CANARY));
    // The key is trimmed and rests in the credentials alone.
    assert_eq!(outcome.credentials.expose().get(CREDENTIAL), Some(CANARY));
    assert_eq!(outcome.credentials.expose().len(), 1);
    assert!(
        !format!("{outcome:?}").contains(CANARY),
        "no Debug shows it"
    );
}

#[test]
fn a_sign_in_that_gives_no_key_or_a_far_plain_address_makes_nothing_and_says_which() {
    let empty = sign_in(
        &json!({"base_url": "https://llm.example.com", "standards": "openai-api"}),
        "  ",
    )
    .expect_err("no key");
    assert_eq!(empty.status, Status::GUATIAO_ERR_BAD_VALUE);
    let far = sign_in(
        &json!({"base_url": "http://llm.example.com", "standards": "openai-api"}),
        CANARY,
    )
    .expect_err("a key is not sent in the clear");
    assert!(far.message().contains("https"), "{}", far.message());
    assert!(!far.message().contains(CANARY));
}

#[test]
fn the_models_are_the_openai_list_of_the_address_read_with_the_key() {
    let (address, seen) = stand_in(&["gpt-x", "claude-y"]);
    let listing = models(grant(&address, &[OPENAI_API]));
    assert!(listing.known, "{:?}", listing.unknown_because);
    let ids: Vec<&str> = listing.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-x", "claude-y"]);
    // An address that already ends in /v1 is not given a second one.
    let again = models(grant(&format!("{address}/v1"), &[OPENAI_API]));
    assert!(again.known, "{:?}", again.unknown_because);
    let lines: Vec<String> = seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(line, _)| line.clone())
        .collect();
    assert_eq!(
        lines,
        ["GET /v1/models HTTP/1.1", "GET /v1/models HTTP/1.1"]
    );
}

#[test]
fn an_account_used_as_another_standard_gives_the_reason_and_a_refused_key_is_a_failure_that_says_so()
 {
    let (address, _) = stand_in(&["m"]);
    let anthropic = models(grant(&address, &[ANTHROPIC_API]));
    assert!(!anthropic.known);
    assert!(
        anthropic
            .unknown_because
            .as_deref()
            .is_some_and(|why| why.contains("OpenAI") && why.contains("anthropic-api")),
        "{:?}",
        anthropic.unknown_because
    );
    let mut wrong = grant(&address, &[OPENAI_API]);
    let mut credentials = Credentials::new();
    credentials.insert(CREDENTIAL, "not-the-key");
    wrong = wrong.credentials(credentials);
    let refused = models(wrong);
    assert!(!refused.known);
    assert_eq!(refused.unknown_kind, Some(UnknownKind::Failed));
    let why = refused.unknown_because.expect("a reason");
    assert!(why.contains("401"), "{why}");
    assert!(
        !why.contains("not-the-key") && !why.contains(CANARY),
        "{why}"
    );
    // Nothing answers there.
    let nobody = models(grant("http://127.0.0.1:1", &[OPENAI_API]));
    assert!(!nobody.known);
    assert!(!nobody.unknown_because.unwrap_or_default().contains(CANARY));
}

#[test]
fn usage_is_not_offered_and_that_is_the_answer() {
    let usage = ApiGatewayAccounts
        .usage(
            coqui_acp_api::AccountUsageQuery::new(grant("https://g.example", &[OPENAI_API])),
            spawner(),
            NoKeeper.into_object(),
        )
        .expect("an answer");
    assert!(!usage.supported);
    assert_eq!(usage.account.as_deref(), Some("id-one"));
}
