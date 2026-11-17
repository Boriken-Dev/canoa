//! `speech/voices`, `speech/say`, `speech/audio` and `speech/stop` on the control channel: text streamed through a provider canoa.cfg
//! defines comes back as audio in order, its voices are listed with where the text goes, and an utterance belongs to one link only.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use canoa_host::Link;
use data_encoding::BASE64;
use linked::{Hosted, next, send};
use serde_json::{Value, json};
use support::{example, quoted};

fn host() -> Hosted {
    let agent = example("stdio_agent", false);
    let standin = example("host_say_standin", false);
    Hosted::start_with_setup(
        |_| {
            format!(
                "[host.agents.Probe]\nprogram = {}\nargs = [\"--scripted\"]\n\n[say.standin]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"lines\"]\nvoices = [\"ana\", \"luis\"]\nmodels = [\"small\", \"big\"]\n",
                quoted(&agent.to_string_lossy()),
                quoted(&standin.to_string_lossy()),
            )
        },
        &[],
    )
}

fn call(link: &Link, id: u64, method: &str, params: &Value) -> Result<Value, Value> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    );
    loop {
        let message = next(link).expect("an answer");
        if message["id"] == id && message.get("method").is_none() {
            return match message.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(message["result"].clone()),
            };
        }
    }
}

#[test]
fn text_spoken_through_the_providers_in_canoa_cfg_comes_back_as_audio_in_order() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let listed = call(&link, 1, "speech/voices", &json!({"provider": "standin"})).expect("listed");
    let standin = &listed["providers"][0];
    assert_eq!(standin["name"], "standin", "{listed}");
    assert_eq!(standin["voices"][1]["id"], "luis", "{listed}");
    assert_eq!(standin["models"][0]["id"], "small", "{listed}");
    assert_eq!(standin["leaves"], "maybe", "{listed}");
    assert!(standin.get("problem").is_none(), "{listed}");
    // The speech-to-text providers are not this list, and these tables are not theirs.
    let input = call(&link, 2, "speech/providers", &json!({})).expect("listed");
    assert!(
        input["providers"].as_array().expect("providers").is_empty(),
        "{input}"
    );

    let started = call(
        &link,
        3,
        "speech/say",
        &json!({"provider": "standin", "text": "Good morning. Good night.", "voice": "luis"}),
    )
    .expect("started");
    assert_eq!(started["encoding"], "pcm_s16le", "{started}");
    let utterance = started["utterance"]
        .as_str()
        .expect("an utterance")
        .to_owned();
    let mut audio = Vec::new();
    for n in 0..100_u64 {
        let piece = call(
            &link,
            10 + n,
            "speech/audio",
            &json!({"utterance": utterance, "wait_ms": 1000}),
        )
        .expect("audio");
        audio.extend(
            BASE64
                .decode(piece["audio"].as_str().expect("audio").as_bytes())
                .expect("base64"),
        );
        if piece["done"] == true {
            break;
        }
    }
    assert_eq!(
        String::from_utf8(audio).expect("text"),
        "[Good morning.][Good night.]"
    );
    let again =
        call(&link, 200, "speech/audio", &json!({"utterance": utterance})).expect_err("gone");
    assert_eq!(again["code"], -32003, "{again}");
    host.stop();
}

#[test]
fn an_utterance_is_stopped_and_belongs_to_the_link_that_started_it() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let started = call(
        &link,
        1,
        "speech/say",
        &json!({"provider": "standin", "text": "One. Two."}),
    )
    .expect("started");
    let utterance = started["utterance"]
        .as_str()
        .expect("an utterance")
        .to_owned();
    call(&link, 2, "speech/stop", &json!({"utterance": utterance})).expect("stopped");
    let after = call(&link, 3, "speech/audio", &json!({"utterance": utterance})).expect_err("gone");
    assert_eq!(after["code"], -32003, "{after}");
    let refused = call(
        &link,
        4,
        "speech/say",
        &json!({"provider": "standin", "text": "Hi.", "voice": "zed"}),
    )
    .expect_err("an unlisted voice");
    assert!(
        refused["message"]
            .as_str()
            .expect("why")
            .contains("ana, luis"),
        "{refused}"
    );
    host.stop();
}
