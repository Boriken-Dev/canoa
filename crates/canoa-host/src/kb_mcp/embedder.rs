//! The embedder of the knowledge index (`[host.kb.embedder]`): texts to vectors through the host's `assist` service, so whatever the
//! person chose serves it (an account's API, a server by address) and the host names none of them.

use alloc::sync::Arc;
use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use canoa::network::Network;
use canoa::sdk::EmbedRequest;
use coqui_aos::index::{Embedder, Error};

use crate::assist::Assist;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// The embeddings by text, and the order they came in.
type Held = (HashMap<String, Vec<f32>>, VecDeque<String>);

/// The query embeddings the host remembers, oldest dropped first, for the repeated searches an agent makes in one turn.
#[derive(Debug, Default)]
pub(crate) struct QueryCache {
    limit: usize,
    held: Mutex<Held>,
}

impl QueryCache {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            held: Mutex::default(),
        }
    }

    fn get(&self, key: &str) -> Option<Vec<f32>> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .get(key)
            .cloned()
    }

    fn put(&self, key: String, vector: Vec<f32>) {
        if self.limit == 0 {
            return;
        }
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.0.insert(key.clone(), vector).is_none() {
            held.1.push_back(key);
            while held.1.len() > self.limit {
                if let Some(old) = held.1.pop_front() {
                    held.0.remove(&old);
                }
            }
        }
    }
}

/// An embedder served by `assist`.
#[derive(Clone)]
pub(crate) struct AssistEmbedder {
    assist: Arc<Assist>,
    choice: String,
    asked: String,
    dimensions: Option<u32>,
    /// What an index stores and names its variant by: the backend, the model and the length asked for.
    name: String,
    cache: Arc<QueryCache>,
    /// The network a session's requests go through; the process's when none.
    network: Option<Arc<Network>>,
}

impl core::fmt::Debug for AssistEmbedder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AssistEmbedder")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl AssistEmbedder {
    /// The embedder for `model` under `choice` (a backend id or `default`).
    pub(crate) fn new(
        assist: Arc<Assist>,
        choice: &str,
        model: &str,
        dimensions: Option<u32>,
        cache: Arc<QueryCache>,
    ) -> Self {
        let backend = assist
            .first(crate::assist::Operation::Embed, choice)
            .unwrap_or_else(|| choice.to_owned());
        let name = match dimensions {
            Some(length) => format!("{backend}/{model}@{length}"),
            None => format!("{backend}/{model}"),
        };
        Self {
            assist,
            choice: choice.to_owned(),
            asked: model.to_owned(),
            dimensions,
            name,
            cache,
            network: None,
        }
    }

    /// The same embedder reached through `network`, as one session's profile chose it; the process's when `None`.
    pub(crate) fn on(mut self, network: Option<Arc<Network>>) -> Self {
        self.network = network;
        self
    }
}

impl Embedder for AssistEmbedder {
    fn model(&self) -> &str {
        &self.name
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, Error> {
        let single = match inputs {
            [one] => Some(format!("{}\0{one}", self.name)),
            _ => None,
        };
        if let Some(vector) = single.as_deref().and_then(|key| self.cache.get(key)) {
            return Ok(vec![vector]);
        }
        let network = self.network.clone().unwrap_or_else(Network::current);
        let mut request = EmbedRequest::new(&self.asked, inputs.to_vec());
        request.dimensions = self.dimensions;
        let done = self
            .assist
            .embed(&network, &self.choice, request, TIMEOUT)
            .map_err(Error::Embed)?;
        let vectors = done.value.vectors;
        if vectors.len() != inputs.len() {
            return Err(Error::Embed(format!(
                "{} answered {} vectors for {} inputs",
                done.backend,
                vectors.len(),
                inputs.len()
            )));
        }
        if let (Some(key), [vector]) = (single, vectors.as_slice()) {
            self.cache.put(key, vector.clone());
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_keeps_the_newest_and_drops_the_oldest() {
        let cache = QueryCache::new(2);
        cache.put("a".to_owned(), vec![1.0]);
        cache.put("b".to_owned(), vec![2.0]);
        cache.put("c".to_owned(), vec![3.0]);
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b"), Some(vec![2.0]));
        assert_eq!(cache.get("c"), Some(vec![3.0]));
        let off = QueryCache::new(0);
        off.put("a".to_owned(), vec![1.0]);
        assert!(off.get("a").is_none());
    }
}
