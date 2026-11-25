//! The `sdk_provider` kind: the contract outside providers are built against, and the built-in `server` held to it.
//! Regenerate the description files with `CANOA_SDK_WRITE_KINDS=1 cargo test -p canoa --features sdk --test sdk`.
#![cfg(feature = "sdk")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::sdk::contract::{self, Fixtures};
use canoa::sdk::testing::Route;
use canoa::sdk::{
    Auth, Capability, ChatMessage, ChatReply, ChatRequest, EmbedReply, EmbedRequest, Header,
    HttpReply, HttpRequest, LentHttpVtable, OpenRequest, Ranked, RerankReply, RerankRequest,
    SdkInfo, SdkProviderVtable, SdkVtable, Server,
};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_serde::Serializable;
use serde_json::Value as Json;

fn file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn pretty(value: &Json) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("written");
    text.push('\n');
    text
}

fn committed(path: &str, fresh: &str) {
    let path = file(path);
    if std::env::var_os("CANOA_SDK_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, fresh).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    // Compared as documents: another crate's `preserve_order` in the build changes the order a JSON object is written in.
    let (committed, fresh): (Json, Json) = (
        serde_json::from_str(&committed).expect("the committed file is JSON"),
        serde_json::from_str(fresh).expect("the fresh text is JSON"),
    );
    assert!(committed == fresh, "regenerate {}", path.display());
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document = format!(
        "{{\"kinds\":[{},{},{}]}}",
        SdkProviderVtable::description(),
        SdkVtable::description(),
        LentHttpVtable::description()
    );
    committed(
        "include/canoa.sdk_provider.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: serde_json::Map<String, Json> = [
        ("Capability", schema_of::<Capability>()),
        ("Auth", schema_of::<Auth>()),
        ("SdkInfo", schema_of::<SdkInfo>()),
        ("OpenRequest", schema_of::<OpenRequest>()),
        ("Header", schema_of::<Header>()),
        ("HttpRequest", schema_of::<HttpRequest>()),
        ("HttpReply", schema_of::<HttpReply>()),
        ("ChatMessage", schema_of::<ChatMessage>()),
        ("ChatRequest", schema_of::<ChatRequest>()),
        ("ChatReply", schema_of::<ChatReply>()),
        ("EmbedRequest", schema_of::<EmbedRequest>()),
        ("EmbedReply", schema_of::<EmbedReply>()),
        ("RerankRequest", schema_of::<RerankRequest>()),
        ("Ranked", schema_of::<Ranked>()),
        ("RerankReply", schema_of::<RerankReply>()),
    ]
    .into_iter()
    .map(|(name, schema)| (name.to_owned(), schema))
    .collect();
    committed(
        "include/canoa.sdk_provider.schemas.json",
        &pretty(&Json::Object(schemas)),
    );
}

#[test]
fn a_description_survives_the_trip_through_a_value() {
    let mut info = SdkInfo::new("acme", "Acme");
    info.standards = vec!["acme-api".to_owned()];
    info.credential = "api_key".to_owned();
    info.auth = Auth::header("x-api-key");
    info.capabilities = vec![Capability::new("chat")];
    let value = info.to_value().expect("a value");
    assert_eq!(SdkInfo::from_value(&value).expect("read back"), info);
}

/// The answers a server that speaks the OpenAI shape gives, recorded as the shape documents them.
pub fn server_routes() -> Vec<Route> {
    vec![
        Route::new(
            "GET",
            "/v1/models",
            200,
            r#"{"object":"list","data":[{"id":"emb-1","object":"model"},{"id":"chat-1","object":"model"}]}"#,
        ),
        Route::new(
            "POST",
            "/v1/chat/completions",
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}],"usage":{"prompt_tokens":3,"completion_tokens":1}}"#,
        ),
        Route::new(
            "POST",
            "/v1/embeddings",
            200,
            r#"{"data":[{"index":1,"embedding":[0.5,0.5]},{"index":0,"embedding":[1,0]}],"usage":{"prompt_tokens":2}}"#,
        ),
        Route::new(
            "POST",
            "/v1/rerank",
            200,
            r#"{"results":[{"index":1,"relevance_score":0.9},{"index":0,"relevance_score":0.2}]}"#,
        ),
    ]
}

#[test]
fn the_server_provider_passes_the_contract_at_an_address_with_and_without_v1() {
    for address in ["http://127.0.0.1:60570", "http://127.0.0.1:60570/v1/"] {
        let mut routes = server_routes();
        if address.ends_with("/v1/") {
            for route in &mut routes {
                route.path = route.path.trim_start_matches("/v1").to_owned();
            }
        }
        let fixtures = Fixtures {
            address: address.to_owned(),
            settings: r#"{"capabilities":["embed","chat"]}"#.to_owned(),
            routes,
            model: "chat-1".to_owned(),
        };
        contract::run(&Server, &fixtures).unwrap_or_else(|why| panic!("{address}: {why}"));
    }
}

#[test]
fn the_server_asks_for_paths_only_and_sends_no_key() {
    use canoa::sdk::testing::Recorded;
    use canoa::sdk::{LentHttp as _, SdkProvider as _};
    let (http, asked) = Recorded::new(server_routes());
    let sdk = Server
        .open(
            OpenRequest::new("http://127.0.0.1:1", "{}"),
            http.into_object(),
        )
        .expect("opens");
    sdk.models().expect("listed");
    let requests = asked.requests();
    assert_eq!(requests[0].path, "/v1/models");
    assert!(requests.iter().all(|request| {
        request
            .headers
            .iter()
            .all(|header| !header.name.eq_ignore_ascii_case("authorization"))
    }));
}

/// A provider whose description the host cannot offer, as the control for the contract.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(canoa::sdk::SdkProvider, id = "bad", crate = ::coqui_acp_api::guatiao)]
struct Bad;

impl canoa::sdk::SdkProvider for Bad {
    fn describe(&self) -> Result<SdkInfo, coqui_acp_api::guatiao::library::ProviderError> {
        Ok(SdkInfo::new("Not-A-Word", "Bad"))
    }

    fn open(
        &self,
        _request: OpenRequest,
        _http: coqui_acp_api::guatiao::library::Object<dyn canoa::sdk::LentHttp>,
    ) -> Result<
        coqui_acp_api::guatiao::library::Object<dyn canoa::sdk::Sdk>,
        coqui_acp_api::guatiao::library::ProviderError,
    > {
        unreachable!("the contract stops at the description")
    }
}

#[test]
fn a_provider_the_host_cannot_offer_fails_the_contract() {
    let fixtures = Fixtures {
        address: "http://h".to_owned(),
        settings: "{}".to_owned(),
        routes: server_routes(),
        model: "m".to_owned(),
    };
    let why = contract::run(&Bad, &fixtures).expect_err("refused");
    assert!(why.contains("cannot be offered"), "{why}");
}

#[test]
fn a_server_stamps_the_capabilities_the_person_declared_on_its_models() {
    use canoa::sdk::testing::Recorded;
    use canoa::sdk::{LentHttp as _, SdkProvider as _, models_for};
    let (http, _) = Recorded::new(server_routes());
    let sdk = Server
        .open(
            OpenRequest::new("http://h", r#"{"capabilities":["embed"]}"#),
            http.into_object(),
        )
        .expect("opens");
    let models = sdk.models().expect("listed").models;
    assert_eq!(models_for(&models, "embed").offered.len(), 2);
    assert!(models_for(&models, "chat").offered.is_empty());
    let (http, _) = Recorded::new(server_routes());
    let sdk = Server
        .open(OpenRequest::new("http://h", "{}"), http.into_object())
        .expect("opens");
    let models = sdk.models().expect("listed").models;
    assert_eq!(
        models_for(&models, "embed").unreported.len(),
        2,
        "a server that declares nothing reports nothing, and its models are kept apart"
    );
}
