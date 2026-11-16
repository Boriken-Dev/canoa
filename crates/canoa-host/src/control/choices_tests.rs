use super::*;

const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn config(text: &str) -> Config {
    Config::from_str_in(text, std::path::Path::new("")).expect("parses")
}

fn key(id: &str) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: Default::default(),
    }
}

#[test]
fn a_key_asks_only_what_it_may_start_a_session_on() {
    let named = KEY.parse::<coqui_peer_webcrypto::PeerId>().expect("a key").to_string();
    let other = "b".repeat(64).parse::<coqui_peer_webcrypto::PeerId>().expect("a key").to_string();
    let config = config(&format!(
        "[host.agents.open]\nprogram = \"x\"\n[host.agents.shut]\nprogram = \"x\"\npeers = []\n[host.agents.mine]\nprogram = \"x\"\npeers = [\"{KEY}\"]\n"
    ));
    assert!(reaches(&Caller::Owner, &config, "shut", false));
    assert!(reaches(&key(&other), &config, "open", false));
    assert!(!reaches(&key(&other), &config, "shut", false));
    assert!(!reaches(&key(&named), &config, "shut", false));
    assert!(reaches(&key(&named), &config, "mine", false));
    assert!(!reaches(&key(&other), &config, "mine", false));
}

#[test]
fn a_built_in_source_with_no_entry_is_reached_by_a_key_only_on_a_portal_host() {
    let config = config("");
    assert!(reaches(&Caller::Owner, &config, "Claude", false));
    assert!(!reaches(&key("someone"), &config, "Claude", false));
    assert!(reaches(&key("someone"), &config, "Claude", true));
    assert_eq!(built_in("Claude"), Some(crate::Harness::Claude));
    assert_eq!(built_in("nothing"), None);
}
