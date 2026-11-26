use super::*;

/// What Hugging Face answered for `GET /api/models?author=openai&search=whisper`.
const LISTING: &str = include_str!("../tests/fixtures/hf_list_openai_whisper.json");
/// What it answered for `GET /api/models/openai/whisper-<name>/revision/main?blobs=true`, for four repositories.
const BASE: &str = include_str!("../tests/fixtures/hf_whisper-base.json");
const TINY: &str = include_str!("../tests/fixtures/hf_whisper-tiny.json");
const TINY_EN: &str = include_str!("../tests/fixtures/hf_whisper-tiny.en.json");
const TURBO: &str = include_str!("../tests/fixtures/hf_whisper-large-v3-turbo.json");

fn names(ids: &[String]) -> Vec<&str> {
    ids.iter().map(String::as_str).collect()
}

#[test]
fn the_listing_gives_every_repository_and_the_default_leaves_out_the_english_only_ones() {
    let all = repositories(LISTING, &[], &[]).expect("a listing");
    assert_eq!(all.len(), 12, "{all:?}");
    assert!(names(&all).contains(&"openai/whisper-tiny.en"));
    let exclude: Vec<String> = DEFAULT_EXCLUDE
        .iter()
        .map(|each| (*each).to_owned())
        .collect();
    let fewer = repositories(LISTING, &[], &exclude).expect("a listing");
    assert_eq!(fewer.len(), 8, "{fewer:?}");
    assert!(fewer.iter().all(|id| !id.ends_with(".en")));
    let large = repositories(LISTING, &["*large*".to_owned()], &exclude).expect("a listing");
    assert!(large.iter().all(|id| id.contains("large")), "{large:?}");
    assert!(repositories("{}", &[], &[]).is_err());
    assert!(repositories("nope", &[], &[]).is_err());
}

#[test]
fn a_repository_is_a_model_whose_three_files_are_checked_by_what_the_listing_states() {
    let base = model_of(BASE).expect("read").expect("a model");
    assert_eq!(base.id, "base");
    assert_eq!(base.repository, "openai/whisper-base");
    assert_eq!(base.revision, "e37978b90ca9030d5170a5c07aadb050351a65bb");
    assert_eq!(base.label, "Base, every language (293 MB)");
    assert_eq!(base.license.as_deref(), Some("apache-2.0"));
    assert_eq!(base.bytes(), 1_983 + 2_480_466 + 290_403_936);
    let weights = base.file(WEIGHTS).expect("weights");
    assert_eq!(
        weights.check,
        Check::Sha256("07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b".into()),
        "a large file is checked by the SHA-256 the listing states"
    );
    assert_eq!(
        base.file(CONFIG).expect("config").check,
        Check::GitBlob("708cca09e7e8a9364e444efe8b4f9c2691fc5f51".into()),
        "a small one by its git blob id"
    );
    assert_eq!(
        base.url(weights),
        "https://huggingface.co/openai/whisper-base/resolve/e37978b90ca9030d5170a5c07aadb050351a65bb/model.safetensors",
        "at the commit, so the digest and the bytes are of one moment"
    );
    let turbo = model_of(TURBO).expect("read").expect("a model");
    assert_eq!(turbo.id, "large-v3-turbo");
    assert_eq!(turbo.label, "Large v3 turbo, every language (1.6 GB)");
    assert_eq!(turbo.license.as_deref(), Some("mit"));
    let english = model_of(TINY_EN).expect("read").expect("a model");
    assert_eq!(english.label, "Tiny, English only (153 MB)");
}

#[test]
fn a_repository_without_the_three_files_is_no_model_and_a_file_with_nothing_to_check_it_is_refused()
{
    let without = BASE.replace(
        "\"rfilename\":\"tokenizer.json\"",
        "\"rfilename\":\"tokens.json\"",
    );
    assert_eq!(model_of(&without), Ok(None));
    let unlisted = BASE.replace(
        "\"blobId\":\"708cca09e7e8a9364e444efe8b4f9c2691fc5f51\",",
        "",
    );
    assert!(model_of(&unlisted).is_err_and(|why| why.contains("nothing to check")));
    let moving = BASE.replace("e37978b90ca9030d5170a5c07aadb050351a65bb", "main");
    assert!(model_of(&moving).is_err_and(|why| why.contains("no commit")));
    assert!(model_of("{}").is_err());
}

#[test]
fn the_models_of_several_descriptions_are_listed_in_order_and_one_without_the_files_is_left_out() {
    let without = BASE.replace(
        "\"rfilename\":\"model.safetensors\"",
        "\"rfilename\":\"model.bin\"",
    );
    let models = models_of([TINY, without.as_str(), BASE]).expect("models");
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["tiny", "base"]);
}

#[test]
fn a_name_matches_a_pattern_with_stars_for_any_run_of_characters() {
    assert!(wildcard("*.en", "tiny.en"));
    assert!(!wildcard("*.en", "tiny"));
    assert!(wildcard("whisper-*", "whisper-tiny"));
    assert!(wildcard("a*b*c", "aXbYc"));
    assert!(!wildcard("a*a", "a"));
    assert!(wildcard("tiny", "tiny"));
    assert!(!wildcard("tiny", "tiny.en"));
}
