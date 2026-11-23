//! How Codex is given what the host adds to a session: the skills of a profile, in a directory its adapter hands to Codex.

use canoa::vendor::{
    DeliveredFile, GatewayRoute, HookSupport, HookSupportRequest, SkillsDelivery, SkillsRequest,
};

/// The directory below a session's folders where Codex looks for skills.
pub(crate) const SKILLS_FOLDER: &str = ".agents/skills";

/// Skills in a directory the session request names, for a Codex adapter that says it takes more directories.
pub(crate) fn skills_delivery(request: &SkillsRequest) -> Option<SkillsDelivery> {
    request.additional_directories.then(|| {
        let mut delivery = SkillsDelivery::new(
            SKILLS_FOLDER,
            "a directory of skills Codex reads, through ACP additionalDirectories",
        );
        delivery.directory = true;
        delivery
    })
}

/// Codex takes the address and bearer of a gateway as an OpenAI API key whose base URL is moved, the gateway's `/v1` below its address.
pub(crate) fn gateway_route() -> GatewayRoute {
    let mut route = GatewayRoute::new("codex-api-key", "api_key");
    route.address = Some("openai_base_url".to_owned());
    route.suffix = "/v1".to_owned();
    route
}

/// The `config.toml` Codex is run with in its account's folder when a gateway serves it: the tools and features a local model's
/// gateway cannot serve are off (its hosted web search, and the multi-agent and goal tools whose request shapes only OpenAI's models
/// take).
pub(crate) fn gateway_files() -> Vec<DeliveredFile> {
    vec![DeliveredFile::new(
        "config.toml",
        "web_search = \"disabled\"\n\n[features]\nmulti_agent = false\ngoals = false\n",
    )]
}

/// Codex has no hook that reports what ACP does not show.
pub(crate) fn hook_support(request: &HookSupportRequest) -> Option<HookSupport> {
    match (request.event.as_str(), request.reason.as_deref()) {
        ("cd", _) => Some(HookSupport::cannot(
            "Codex has no event for a change of working directory",
        )),
        ("pre-compact", _) => Some(HookSupport::cannot(
            "the Codex adapter has no way to compact over ACP, and Codex's own hook for it has no trigger there",
        )),
        ("session-start", Some("compact")) => Some(HookSupport::cannot(
            "the Codex adapter has no way to compact over ACP",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_go_in_a_directory_only_where_the_adapter_takes_directories() {
        assert_eq!(skills_delivery(&SkillsRequest::new(false)), None);
        let delivery = skills_delivery(&SkillsRequest::new(true)).expect("a delivery");
        assert!(delivery.directory);
        assert_eq!(delivery.folder, ".agents/skills");
        assert_eq!(delivery.plugin, None);
    }

    #[test]
    fn a_gateway_is_an_api_key_with_its_address_moved_and_codex_runs_without_what_it_cannot_serve()
    {
        let route = gateway_route();
        assert_eq!(
            (route.kind.as_str(), route.suffix.as_str()),
            ("codex-api-key", "/v1")
        );
        assert_eq!(route.address.as_deref(), Some("openai_base_url"));
        let files = gateway_files();
        assert_eq!(files[0].path, "config.toml");
        assert!(files[0].text.contains("web_search = \"disabled\""));
        assert!(
            files[0].text.contains("multi_agent = false")
                && files[0].text.contains("goals = false")
        );
    }

    #[test]
    fn no_event_is_given_by_its_hooks() {
        for event in ["cd", "pre-compact"] {
            let said = hook_support(&HookSupportRequest::new(event, None)).expect("a word");
            assert!(!said.fires);
        }
    }
}
