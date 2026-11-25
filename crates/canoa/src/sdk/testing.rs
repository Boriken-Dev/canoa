//! A stand-in for the host's lent HTTP: canned answers by method and path, and a record of what was asked, so a provider is tested
//! against recorded answers and no network.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::sync::{Mutex, PoisonError};

use coqui_acp_api::guatiao::library::ProviderError;

use super::{HttpReply, HttpRequest, LentHttp};

/// One canned answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// `GET` or `POST`.
    pub method: String,
    /// The path the request names.
    pub path: String,
    /// The status answered.
    pub status: u16,
    /// The body answered.
    pub body: String,
}

impl Route {
    /// A `method` request of `path` answered `status` with `body`.
    #[must_use]
    pub fn new(method: &str, path: &str, status: u16, body: impl Into<String>) -> Self {
        Self {
            method: method.to_owned(),
            path: path.to_owned(),
            status,
            body: body.into(),
        }
    }
}

/// What a [`Recorded`] was asked, kept after the stand-in is handed over.
#[derive(Debug, Clone, Default)]
pub struct Asked(Arc<Mutex<Vec<HttpRequest>>>);

impl Asked {
    /// The requests so far, oldest first.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A [`LentHttp`] that answers from its routes, and `404` to any other request.
#[derive(Debug, Clone)]
pub struct Recorded {
    routes: Vec<Route>,
    asked: Asked,
}

impl Recorded {
    /// A stand-in answering `routes`, and what it will be asked.
    #[must_use]
    pub fn new(routes: Vec<Route>) -> (Self, Asked) {
        let asked = Asked::default();
        (
            Self {
                routes,
                asked: asked.clone(),
            },
            asked,
        )
    }
}

impl LentHttp for Recorded {
    fn request(&self, request: HttpRequest) -> Result<HttpReply, ProviderError> {
        let found = self
            .routes
            .iter()
            .find(|route| route.method == request.method && route.path == request.path)
            .map(|route| HttpReply::new(route.status, route.body.clone()));
        self.asked
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(found.unwrap_or_else(|| HttpReply::new(404, "{\"error\":\"no such route\"}")))
    }
}
