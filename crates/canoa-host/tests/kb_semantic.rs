//! A search by meaning against a real embeddings endpoint, such as Foundry Local's daemon with an embedding model loaded. It
//! runs only when asked, because it needs a model the person chose and loaded:
//!
//! `CANOA_TEST_EMBED_URL=http://127.0.0.1:60570 CANOA_TEST_EMBED_MODEL=<exact id> cargo test -p canoa-host --test kb_semantic -- --ignored --nocapture`
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use canoa_host::kb::{
    Corpus, Embedder, HttpEmbedder, Index, Mode, Options, SearchOptions, Settings,
};

const NOTES: [(&str, &str); 5] = [
    (
        "kb/cars.md",
        "# Cars\n\n## Engine\n\nWhen the motor will not turn over, check the battery terminals, then the starter relay.\n\n## Tyres\n\nKeep the pressure at the figure printed inside the driver's door.\n",
    ),
    (
        "kb/cooking.md",
        "# Cooking\n\nSimmer the tomatoes with garlic for twenty minutes, then fold in the basil and a spoon of olive oil.\n",
    ),
    (
        "kb/garden.md",
        "# Garden\n\nWater the seedlings at dawn; roots take up moisture best before the heat of the day.\n",
    ),
    (
        "rules/review.md",
        "Reviewers read the diff before the description, and ask for tests that fail without the change.\n",
    ),
    (
        "kb/space.md",
        "# Space\n\nA rocket reaches orbit when its sideways speed is high enough that the ground curves away as fast as it falls.\n",
    ),
];

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-kb-semantic-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
#[ignore = "needs an embeddings endpoint with a model loaded (CANOA_TEST_EMBED_URL, CANOA_TEST_EMBED_MODEL)"]
fn a_query_finds_a_passage_that_shares_no_word_with_it() {
    let url = std::env::var("CANOA_TEST_EMBED_URL").expect("CANOA_TEST_EMBED_URL");
    let model = std::env::var("CANOA_TEST_EMBED_MODEL").expect("CANOA_TEST_EMBED_MODEL");
    let embedder = HttpEmbedder::new(&url, model.clone());

    let mut corpus = Corpus::new();
    for (path, text) in NOTES {
        corpus.insert(path, text.as_bytes().to_vec()).unwrap();
    }
    let dir = scratch("index");
    let started = Instant::now();
    let report = Index::create(
        &dir,
        Settings::default(),
        &corpus,
        Some(&embedder),
        &Options::default(),
    )
    .unwrap();
    println!(
        "model {} embedded {} passages in {:?}",
        embedder.model(),
        report.passages_new,
        started.elapsed()
    );
    let index = Index::open(&dir).unwrap();
    println!(
        "saved model {:?}, dimensions {:?}",
        index.settings().model,
        index.settings().dimensions
    );
    assert_eq!(index.settings().model.as_deref(), Some(model.as_str()));
    assert!(index.settings().dimensions.is_some_and(|size| size > 8));

    // None of these words is in the passage that answers it.
    let cases = [
        ("my vehicle refuses to start", "kb/cars.md"),
        ("how to cook a pasta sauce", "kb/cooking.md"),
        ("when should I irrigate plants", "kb/garden.md"),
        ("how fast must a satellite travel", "kb/space.md"),
    ];
    let mut by_meaning = SearchOptions::default();
    by_meaning.mode = Mode::Semantic;
    by_meaning.top = 3;
    let mut words = SearchOptions::default();
    words.mode = Mode::Lexical;
    let mut right = 0;
    for (query, want) in cases {
        let hits = index.search(query, &by_meaning, Some(&embedder)).unwrap();
        println!("{query:?}");
        for hit in &hits {
            println!(
                "    {:.3} {}:{}-{} {}",
                hit.score, hit.path, hit.from_line, hit.to_line, hit.heading
            );
        }
        if hits[0].path == want {
            right += 1;
        }
        assert!(hits.windows(2).all(|pair| pair[0].score >= pair[1].score));
        assert!(hits.iter().all(|hit| hit.score <= 1.0001));
    }
    assert!(
        right >= 3,
        "{right} of {} queries found their passage first",
        cases.len()
    );

    let engine = index
        .search("my vehicle refuses to start", &words, None)
        .unwrap();
    println!("by words, the same query finds {} passages", engine.len());

    // An update with nothing changed embeds nothing.
    let before = Instant::now();
    let again = Index::update(&dir, &corpus, Some(&embedder), &Options::default()).unwrap();
    println!("an update with nothing changed took {:?}", before.elapsed());
    assert_eq!(again.passages_new, 0);
    fs::remove_dir_all(&dir).unwrap();
}
