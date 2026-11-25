//! A stand-in backend: answers deterministically with no network, and can be told to fail, so what is built on `assist` is tested
//! without an account or a server.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::Mutex;

use canoa::network::Network;
use canoa::sdk::{ChatReply, EmbedReply, Ranked, RerankReply};
use coqui_acp_api::{ModelInfo, ModelListing};

use super::{Answer, Backend, Call, Failure, Operation};

/// The topics the stand-in's vectors are made of: a text that mentions one of a group's words is of that topic, so two texts that
/// share no word are close when they share a topic, which is what a model that matches by meaning does.
const TOPICS: [&[&str]; 4] = [
    &["water", "boat", "canoe", "river", "paddle"],
    &["food", "bread", "soup", "kitchen", "cook"],
    &["code", "rust", "compiler", "function", "bug"],
    &["music", "song", "guitar", "drum", "chord"],
];

/// A backend with a few operations and a switch that makes it fail.
pub(crate) struct StandIn {
    id: String,
    can: Vec<Operation>,
    /// Whether the next requests fail.
    pub(crate) failing: AtomicBool,
    /// The text a chat answers; empty answers an order that reverses the passages.
    pub(crate) says: Mutex<String>,
    /// How many requests it served or refused.
    pub(crate) calls: AtomicUsize,
    /// The requests it was asked, as text.
    pub(crate) asked: Mutex<Vec<String>>,
}

impl StandIn {
    /// A backend called `id` that can do `can`.
    pub(crate) fn new(id: &str, can: &[Operation]) -> Arc<Self> {
        Arc::new(Self {
            id: id.to_owned(),
            can: can.to_vec(),
            failing: AtomicBool::new(false),
            says: Mutex::new(String::new()),
            calls: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
        })
    }

    /// The vector of `text`: how much it says of each topic.
    pub(crate) fn vector(text: &str) -> Vec<f32> {
        let lower = text.to_lowercase();
        TOPICS
            .iter()
            .map(|words| {
                #[allow(clippy::cast_precision_loss)]
                let count = words.iter().filter(|word| lower.contains(*word)).count() as f32;
                count
            })
            .collect()
    }
}

impl Backend for StandIn {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        "a stand-in"
    }

    fn location(&self) -> &str {
        "nowhere: it answers in the test"
    }

    fn serves(&self, operation: Operation) -> bool {
        self.can.contains(&operation)
    }

    fn serve(
        &self,
        _network: &Arc<Network>,
        call: Call,
        _timeout: Duration,
    ) -> Result<Answer, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(Failure::new("the stand-in is told to fail"));
        }
        let mut asked = self
            .asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match call {
            Call::Complete(request) => {
                asked.push(
                    request
                        .messages
                        .last()
                        .map(|m| m.content.clone())
                        .unwrap_or_default(),
                );
                let said = self
                    .says
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                Ok(Answer::Complete(ChatReply::new(said)))
            }
            Call::Embed(request) => {
                asked.extend(request.inputs.iter().cloned());
                Ok(Answer::Embed(EmbedReply::new(
                    request
                        .inputs
                        .iter()
                        .map(|text| Self::vector(text))
                        .collect(),
                )))
            }
            Call::Rerank(request) => {
                asked.push(request.query.clone());
                let query = Self::vector(&request.query);
                let mut results: Vec<Ranked> = request
                    .passages
                    .iter()
                    .enumerate()
                    .map(|(at, text)| {
                        let score: f32 = Self::vector(text)
                            .iter()
                            .zip(&query)
                            .map(|(a, b)| a * b)
                            .sum();
                        Ranked::new(u32::try_from(at).unwrap_or(u32::MAX), f64::from(score))
                    })
                    .collect();
                results.sort_by(|a, b| b.score.total_cmp(&a.score));
                Ok(Answer::Rerank(RerankReply::new(results)))
            }
        }
    }

    fn models(&self, _network: &Arc<Network>) -> Result<ModelListing, Failure> {
        Ok(ModelListing::of(vec![ModelInfo::new("stand-in")]))
    }
}
