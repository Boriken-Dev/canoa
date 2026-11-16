//! `kb_search` for the host: coqui-aos's index search by words and meaning, and the `llm` mode over it.

use coqui_aos::index::{Index, IndexSearch};
use coqui_aos::{
    KnowledgeOverlay, Passage, SearchAnswer, SearchBackend, SearchFailure, SearchMode,
    SearchRequest,
};

use super::Access;
use super::tools::WAIT;

/// The backend of one connection's tools.
pub(super) struct Backend<'a>(pub(super) &'a Access);

impl Backend<'_> {
    /// The index search of `overlay`: its committed index, and the embeddings client when the host has one.
    fn index_search(
        &self,
        overlay: &KnowledgeOverlay,
    ) -> Result<(IndexSearch, Option<String>), SearchFailure> {
        let source = self
            .0
            .sources
            .iter()
            .find(|source| source.identity == overlay.identity)
            .ok_or_else(|| {
                SearchFailure::unavailable("the overlay is not one of this session's")
            })?;
        let ready = self
            .0
            .slot(source)
            .wait(WAIT)
            .map_err(SearchFailure::unavailable)?;
        let index = Index::open(&ready.dir)
            .map_err(|error| SearchFailure::unavailable(error.to_string()))?;
        let mut search = IndexSearch::new().index(&overlay.identity, index);
        if let Some(embedder) = &self.0.builder.embedder {
            search = search.embedder(Box::new(embedder.clone()));
        }
        Ok((search, ready.note.clone()))
    }

    /// The passages another mode found, put in the order the person's model gives. When it cannot, a search that asked for
    /// `llm` by name is refused (a named mode is never answered by another), and an `auto` one is answered as the other mode
    /// found them, with the reason in a note.
    fn by_llm(
        &self,
        overlay: &KnowledgeOverlay,
        request: &SearchRequest,
    ) -> Result<SearchAnswer, SearchFailure> {
        let ranker = self.0.builder.ranker.as_ref().ok_or_else(|| {
            SearchFailure::mode_unavailable(
                "the host names no model to order passages: [host.kb] llm_endpoint",
            )
        })?;
        let (search, note) = self.index_search(overlay)?;
        let mut candidates = request.clone();
        candidates.mode = None;
        candidates.top = request.top.max(ranker.candidates);
        let found = search.search(overlay, &candidates)?;
        let first = found.mode;
        let mut passages = found.passages;
        // The handler orders by score, so the candidates are put in the order of their score before the model sees them.
        passages.sort_by(|a, b| b.score.total_cmp(&a.score));
        passages.truncate(ranker.candidates.max(request.top));
        let texts: Vec<String> = passages.iter().map(passage_text).collect();
        let by = first.as_str();
        match ranker.order(&request.query, &texts) {
            Ok(order) if !order.is_empty() => {
                let mut placed = vec![false; passages.len()];
                let mut ordered: Vec<Passage> = Vec::with_capacity(passages.len());
                for number in order {
                    if !core::mem::replace(&mut placed[number], true) {
                        ordered.push(passages[number].clone());
                    }
                }
                // What the model left out follows, in the order it was found.
                for (number, passage) in passages.iter().enumerate() {
                    if !placed[number] {
                        ordered.push(passage.clone());
                    }
                }
                let count = u32::try_from(ordered.len()).unwrap_or(u32::MAX).max(1);
                for (rank, passage) in ordered.iter_mut().enumerate() {
                    passage.score =
                        1.0 - f64::from(u32::try_from(rank).unwrap_or(u32::MAX)) / f64::from(count);
                }
                let mut answer = SearchAnswer::new(SearchMode::Llm, ordered).note(format!(
                    "{} passages found by {by} were ordered by {}",
                    passages.len(),
                    ranker.model()
                ));
                if let Some(note) = note {
                    answer.note = Some(format!("{}; {note}", answer.note.unwrap_or_default()));
                }
                Ok(answer)
            }
            outcome => {
                let why = outcome
                    .err()
                    .unwrap_or_else(|| "the model ordered nothing".to_owned());
                if request.mode.is_some() {
                    return Err(SearchFailure::mode_unavailable(format!(
                        "the model could not order the passages: {why}"
                    )));
                }
                let mut answer = SearchAnswer::new(first, passages).note(format!(
                    "the model could not order them ({why}); ordered by {by}"
                ));
                if let Some(note) = note {
                    answer.note = Some(format!("{}; {note}", answer.note.unwrap_or_default()));
                }
                Ok(answer)
            }
        }
    }
}

fn passage_text(passage: &Passage) -> String {
    if passage.heading.is_empty() {
        format!("{}: {}", passage.path, passage.text)
    } else {
        format!("{} > {}: {}", passage.path, passage.heading, passage.text)
    }
}

impl SearchBackend for Backend<'_> {
    fn modes(&self) -> Vec<SearchMode> {
        let mut modes = vec![SearchMode::Words];
        if self.0.builder.embedder.is_some() {
            modes.push(SearchMode::Meaning);
        }
        if self.0.builder.ranker.is_some() {
            modes.push(SearchMode::Llm);
        }
        modes
    }

    fn search(
        &self,
        overlay: &KnowledgeOverlay,
        request: &SearchRequest,
    ) -> Result<SearchAnswer, SearchFailure> {
        // `auto` is answered by the person's model when they named one, and by words or meaning when it cannot answer.
        if request.mode == Some(SearchMode::Llm)
            || (request.mode.is_none() && self.0.builder.ranker.is_some())
        {
            return self.by_llm(overlay, request);
        }
        let (search, note) = self.index_search(overlay)?;
        let mut answer = search.search(overlay, request)?;
        if let Some(note) = note
            && answer.note.is_none()
        {
            answer.note = Some(note);
        }
        Ok(answer)
    }
}
