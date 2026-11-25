//! The indexes the knowledge tools search: one per overlay version, made once in the background and shared by every connection
//! that applies that version. The index and its embeddings client are `coqui_aos::index`'s.

use alloc::sync::Arc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use coqui_aos::index::{
    self, Corpus, Embedder, Index, Options, index_dir, previous_version, remove_other_versions,
    variant_dir,
};
use coqui_aos::{
    self as aos, CallerFacts, Document, KnowledgeOverlay, KnowledgeSettings, KnowledgeTree,
    OverlayTree, Renderers, TemplateFacts,
};

use super::rank::Ranker;

/// An installed overlay as one connection applies it.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    /// The name canoa.cfg gives it.
    pub(crate) identity: String,
    /// The tree digest its install record names, which its files were checked against.
    pub(crate) digest: String,
    /// Its files.
    pub(crate) tree: OverlayTree,
    /// Whether its rules and context are served through the tools and not put in the context.
    pub(crate) served: bool,
    /// The values the person gave its declared options, which its templates read.
    pub(crate) options: Document,
}

impl Source {
    /// The overlay as the knowledge tools take it.
    pub(crate) fn overlay(&self) -> KnowledgeOverlay {
        // A tree that does not read as an overlay has no knowledge to declare; what lies where overlays keep it is still served.
        let manifest = aos::read_overlay(&self.tree, &aos::Settings::default())
            .map(|read| read.manifest)
            .unwrap_or_default();
        let mut tree = KnowledgeTree::new();
        for path in self.tree.paths() {
            if let Some(bytes) = self.tree.bytes(path) {
                tree.insert(path, aos::KnowledgeEntry::File(bytes.to_vec()));
            }
        }
        let mut facts = TemplateFacts::new();
        facts.options.clone_from(&self.options);
        KnowledgeOverlay::new(&self.identity, manifest, tree)
            .served(self.served)
            .facts(facts)
    }
}

/// How indexes are made: where they live, with what settings, by which embeddings endpoint, and what orders passages.
pub(crate) struct Builder {
    pub(crate) state: PathBuf,
    pub(crate) settings: index::Settings,
    pub(crate) embedder: Option<super::embedder::AssistEmbedder>,
    pub(crate) knowledge: KnowledgeSettings,
    pub(crate) ranker: Option<Ranker>,
    /// What the answers say when `[host.kb.search]` names a provider that is not loaded.
    pub(crate) search_note: Option<String>,
}

/// An overlay version's index as the tools read it.
pub(crate) struct Ready {
    /// Where the committed index is: opened for each search, since the backend owns what it searches.
    pub(crate) dir: PathBuf,
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

fn build(builder: &Builder, source: &Source) -> Result<Ready, String> {
    // What a search finds is what `kb_read` serves with no caller: the served text of each knowledge file.
    let corpus = Corpus::from_overlay(
        &source.overlay(),
        &builder.knowledge,
        &CallerFacts::new(),
        &Renderers::all(),
    );
    let version = index_dir(&builder.state, &source.identity, &source.digest);
    let mut options = Options::default();
    options.source = Some(source.digest.clone());
    let mut note = None;
    let mut made = None;
    if let Some(embedder) = &builder.embedder {
        let dir = variant_dir(&version, Some(embedder.model()));
        match refresh(
            &dir,
            &builder.settings,
            &corpus,
            Some(embedder),
            &options,
            &builder.state,
            source,
        ) {
            Ok(()) => made = Some(dir),
            Err(error) => {
                note = Some(format!(
                    "embeddings are not available ({error}); searching by words"
                ));
            }
        }
    }
    let dir = match made {
        Some(made) => made,
        None => {
            let dir = variant_dir(&version, None);
            refresh(
                &dir,
                &builder.settings,
                &corpus,
                None,
                &options,
                &builder.state,
                source,
            )
            .map_err(|error| error.to_string())?;
            dir
        }
    };
    // The index is read once to know it is whole.
    Index::open(&dir).map_err(|error| error.to_string())?;
    // Earlier versions are of no use once this one is whole; a failure to remove one is left for the next version.
    let _ = remove_other_versions(&builder.state, &source.identity, &source.digest);
    Ok(Ready { dir, note })
}

fn refresh(
    dir: &Path,
    settings: &index::Settings,
    corpus: &Corpus,
    embedder: Option<&dyn Embedder>,
    options: &Options,
    state: &Path,
    source: &Source,
) -> Result<(), index::Error> {
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
