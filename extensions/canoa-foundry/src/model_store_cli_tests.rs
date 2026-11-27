//! The models page against a stand-in for Foundry Local's `foundry` command line (`examples/foundry_cli_standin.rs`), which answers with
//! recorded outputs and exit codes: what it lists, what installing and removing run, and what a refusal says. Nothing is downloaded.

use canoa::store::models::{ModelAct, ModelQuery, ModelStore};
use coqui_acp_harness::LocalSpawner;

use super::*;

fn standin() -> String {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps").parent().expect("debug");
    let name = if cfg!(windows) {
        "foundry_cli_standin.exe"
    } else {
        "foundry_cli_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-foundry --examples ({})",
        path.display()
    );
    path.to_string_lossy().into_owned()
}

fn spawner() -> Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}

fn list() -> ModelList {
    let mut query = ModelQuery::default();
    query.program = Some(standin());
    FoundryModels.list(query, spawner()).expect("a listing")
}

fn act(model: &str, action: &str) -> Result<ModelActed, ProviderError> {
    let mut request = ModelAct::new(model, action);
    request.program = Some(standin());
    FoundryModels.act(request, spawner())
}

#[test]
fn the_catalog_is_one_row_for_each_alias_with_the_cache_listing_saying_which_build_is_installed() {
    let listed = list();
    assert_eq!(listed.unavailable, None);
    assert_eq!(
        listed.models.len(),
        1,
        "two builds of one alias are one row"
    );
    let row = &listed.models[0];
    assert_eq!(row.id, "deepseek-r1-14b");
    let builds: Vec<_> = row
        .builds
        .iter()
        .map(|variant| (variant.id.as_str(), variant.installed))
        .collect();
    assert_eq!(
        builds,
        [
            ("deepseek-r1-distill-qwen-14b-generic-gpu:4", false),
            ("deepseek-r1-distill-qwen-14b-qnn-npu:2", true)
        ],
        "the recorded catalog says the NPU build is downloaded, and the cache listing agrees"
    );
    assert!(row.on);
    assert!(
        row.files.is_empty(),
        "Foundry Local states no digest, so there is nothing to check"
    );
    assert!(
        row.actions[0].removes,
        "an installed model offers its removal"
    );
}

#[test]
fn a_program_that_is_not_installed_says_why_the_page_is_empty() {
    let listed = FoundryModels
        .list(ModelQuery::default(), spawner())
        .expect("an answer");
    assert!(listed.models.is_empty());
    let why = listed.unavailable.expect("a reason");
    assert!(
        why.contains("not installed") && why.contains("Programs list"),
        "{why}"
    );
}

#[test]
fn installing_runs_the_programs_download_and_shows_what_it_printed_last() {
    for model in [
        "deepseek-r1-14b",
        "deepseek-r1-distill-qwen-14b-generic-gpu:4",
    ] {
        let note = act(model, "download").expect("downloaded").note;
        assert_eq!(
            note.as_deref(),
            Some(format!("Downloaded '{model}' to the local cache.").as_str()),
            "the closing sentence of a download whose percentage lines are rewritten"
        );
    }
}

#[test]
fn removing_runs_the_programs_cache_removal_and_shows_its_answer() {
    let note = act("deepseek-r1-14b", "remove").expect("removed").note;
    assert_eq!(
        note.as_deref(),
        Some("Removed 'deepseek-r1-14b' from the cache.")
    );
}

#[test]
fn a_refusal_is_the_programs_own_error_with_its_exit_code() {
    let why = act("unknown-model", "download").expect_err("refused");
    let words = why.message();
    assert!(words.starts_with("download unknown-model: "), "{words}");
    assert!(words.contains("was not found in the catalog"), "{words}");
    assert!(
        words.contains("Run 'foundry model list'"),
        "its hint is kept: {words}"
    );
    assert!(words.ends_with("(exit 3, MODEL_NOT_FOUND)"), "{words}");
    let why = act("not-cached", "remove").expect_err("refused");
    let words = why.message();
    assert!(words.contains("is not in the local cache"), "{words}");
    assert!(words.ends_with("(exit 2, INVALID_ARGUMENT)"), "{words}");
    assert_eq!(
        act("unknown-model", "remove")
            .expect_err("refused")
            .message(),
        "remove unknown-model: Model 'no-such-model' was not found in the catalog. Run 'foundry model list' to see the models you can use. (exit 3, MODEL_NOT_FOUND)"
    );
}

#[test]
fn an_action_the_program_has_no_word_for_and_a_name_that_could_be_an_option_are_refused_before_it_runs()
 {
    assert!(act("deepseek-r1-14b", "format").is_err());
    let why = act("--help", "download").expect_err("refused");
    assert!(
        why.message().contains("letters, digits"),
        "{}",
        why.message()
    );
    let mut request = ModelAct::new("deepseek-r1-14b", "download");
    request.program = None;
    let why = FoundryModels
        .act(request, spawner())
        .expect_err("no program");
    assert!(why.message().contains("not installed"), "{}", why.message());
}
