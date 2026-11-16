//! The indexes the knowledge tools search: one per overlay version, made once in the background and shared by every connection
//! that applies that version.

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use coqui_aos::{self as aos, OverlayTree};

use crate::kb::{
    self, Corpus, Embedder, HttpEmbedder, Index, Options, Settings, index_dir, is_default_path,
    previous_version, remove_other_versions, variant_dir,
};

/// An installed overlay as one connection applies it.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    /// The name canoa.cfg gives it.
    pub(crate) identity: String,
    /// The tree digest its install record names, which its files were checked against.
    pub(crate) digest: String,
    /// Its files.
    pub(crate) tree: OverlayTree,
    /// Whether the person asked for its rules and context to be served as an index and not put in the context.
    pub(crate) indexed: bool,
}

/// How indexes are made: where they live, with what settings and by which embeddings endpoint.
pub(crate) struct Builder {
    pub(crate) state: PathBuf,
    pub(crate) settings: Settings,
    pub(crate) embedder: Option<HttpEmbedder>,
}

/// An overlay version's index as the tools read it.
pub(crate) struct Ready {
    pub(crate) index: Index,
    /// The files that were indexed, whole, for reading by line range.
    pub(crate) corpus: Corpus,
    /// What the passages are scored by: the model's id, or `None` for words.
    pub(crate) model: Option<String>,
    /// Why the index is not what was configured, when it is not.
    pub(crate) note: Option<String>,
}

enum Built {
    Building,
    Ready(Arc<Ready>),
    Failed(String),
}

/// One overlay version's index while it is made and after.
pub(crate) struct Slot {
    built: Mutex<Built>,
    done: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Slot {
    /// The index, once made: waits up to `limit` for the background work, and says so when it is not done by then.
    pub(crate) fn wait(&self, limit: Duration) -> Result<Arc<Ready>, String> {
        let until = Instant::now() + limit;
        let mut built = lock(&self.built);
        loop {
            match &*built {
                Built::Ready(ready) => return Ok(Arc::clone(ready)),
                Built::Failed(why) => return Err(why.clone()),
                Built::Building => {}
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(
                    "the knowledge is still being indexed; ask again in a moment".to_owned(),
                );
            }
            built = self
                .done
                .wait_timeout(built, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn finish(&self, result: Result<Ready, String>) {
        *lock(&self.built) = match result {
            Ok(ready) => Built::Ready(Arc::new(ready)),
            Err(why) => Built::Failed(why),
        };
        self.done.notify_all();
    }
}

/// The slots of every overlay version this host has been asked about.
#[derive(Default)]
pub(crate) struct Library(Mutex<HashMap<(String, String), Arc<Slot>>>);

impl Library {
    /// The slot of `source`'s version, started the first time it is asked for.
    pub(crate) fn slot(&self, builder: &Arc<Builder>, source: &Source) -> Arc<Slot> {
        let key = (source.identity.clone(), source.digest.clone());
        let mut slots = lock(&self.0);
        if let Some(slot) = slots.get(&key)
            && !matches!(*lock(&slot.built), Built::Failed(_))
        {
            return Arc::clone(slot);
        }
        let slot = Arc::new(Slot {
            built: Mutex::new(Built::Building),
            done: Condvar::new(),
        });
        slots.insert(key, Arc::clone(&slot));
        let (made, builder, source) = (Arc::clone(&slot), Arc::clone(builder), source.clone());
        let started = std::thread::Builder::new()
            .name("canoa-kb-index".to_owned())
            .spawn(move || made.finish(build(&builder, &source)));
        if let Err(error) = started {
            slot.finish(Err(format!("the index could not be started: {error}")));
        }
        slot
    }
}

/// The files of `tree` that make up its knowledge: what its manifest declares as rules and context, and what lies where an
/// overlay keeps knowledge by default.
pub(crate) fn corpus_of(tree: &OverlayTree) -> Corpus {
    let mut declared = BTreeSet::new();
    if let Ok(read) = aos::read_overlay(tree, &aos::Settings::default()) {
        declared.extend(aos::rules_of(&read.manifest, tree));
        declared.extend(aos::context_of(&read.manifest, tree));
    }
    let mut corpus = Corpus::new();
    for path in tree.paths() {
        if (declared.contains(path) || is_default_path(path))
            && let Some(bytes) = tree.bytes(path)
        {
            // A path the overlay's own tree holds is relative; one the index would refuse is left out.
            let _ = corpus.insert(path, bytes.to_vec());
        }
    }
    corpus
}

fn build(builder: &Builder, source: &Source) -> Result<Ready, String> {
    let corpus = corpus_of(&source.tree);
    let version = index_dir(&builder.state, &source.identity, &source.digest);
    let mut settings = builder.settings.clone();
    // The corpus is already what the overlay's knowledge is; the index keeps all of it.
    settings.include = vec!["**/*".to_owned()];
    let options = Options {
        source: Some(source.digest.clone()),
        ..Options::default()
    };
    let mut note = None;
    let mut made = None;
    if let Some(embedder) = &builder.embedder {
        let dir = variant_dir(&version, Some(embedder.model()));
        match refresh(
            &dir,
            &settings,
            &corpus,
            Some(embedder),
            &options,
            &builder.state,
            source,
        ) {
            Ok(()) => made = Some((dir, Some(embedder.model().to_owned()))),
            Err(error) => {
                note = Some(format!(
                    "embeddings are not available ({error}); searching by words"
                ));
            }
        }
    }
    let (dir, model) = match made {
        Some(made) => made,
        None => {
            let dir = variant_dir(&version, None);
            refresh(
                &dir,
                &settings,
                &corpus,
                None,
                &options,
                &builder.state,
                source,
            )
            .map_err(|error| error.to_string())?;
            (dir, None)
        }
    };
    let index = Index::open(&dir).map_err(|error| error.to_string())?;
    // Earlier versions are of no use once this one is whole; a failure to remove one is left for the next version.
    let _ = remove_other_versions(&builder.state, &source.identity, &source.digest);
    Ok(Ready {
        index,
        corpus,
        model,
        note,
    })
}

fn refresh(
    dir: &std::path::Path,
    settings: &Settings,
    corpus: &Corpus,
    embedder: Option<&dyn Embedder>,
    options: &Options,
    state: &std::path::Path,
    source: &Source,
) -> Result<(), kb::Error> {
    let variant = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Ok(existing) = Index::open(dir) {
        let mut wanted = settings.clone();
        wanted.model = embedder.map(|embedder| embedder.model().to_owned());
        wanted.dimensions = existing.settings().dimensions;
        if wanted != *existing.settings() {
            // The configuration changed since this version was indexed: its passages are made again under it.
            return Index::rebuild(dir, settings.clone(), corpus, embedder, options).map(drop);
        }
    }
    let seed = previous_version(state, &source.identity, &source.digest, &variant);
    Index::refresh(
        dir,
        settings.clone(),
        corpus,
        embedder,
        options,
        seed.as_deref(),
    )
    .map(drop)
}
