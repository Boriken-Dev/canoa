//! The `desktop` notifier: a notice is pushed, as `notify/notice`, to every window open on the host that asked for them. A host with
//! no such window answers that nobody can be told.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the links are the host's"
)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::notifier::{Delivery, Notice, Notifier, NotifierError, NotifierInfo, SendRequest};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;
use serde_json::json;

/// The most notices held for one window that does not read them.
const QUEUED: usize = 64;

/// One window's link: what it asked for, and what waits to be sent on it.
#[derive(Debug)]
pub(crate) struct LinkState {
    /// The language its fixed phrases are in.
    pub(crate) language: Mutex<String>,
    queue: Mutex<VecDeque<Vec<u8>>>,
    /// The session the person is looking at in that window, while it is focused.
    pub(crate) attended: Mutex<Option<String>>,
}

impl LinkState {
    pub(crate) fn new(language: &str) -> Arc<Self> {
        Arc::new(Self {
            language: Mutex::new(language.to_owned()),
            queue: Mutex::new(VecDeque::new()),
            attended: Mutex::new(None),
        })
    }

    /// What waits to be sent on the link, oldest first.
    pub(crate) fn take(&self) -> Vec<Vec<u8>> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    fn push(&self, line: Vec<u8>) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if queue.len() >= QUEUED {
            queue.pop_front();
        }
        queue.push_back(line);
    }
}

/// The windows of one host.
#[derive(Debug, Default)]
pub(crate) struct Links(Mutex<Vec<Arc<LinkState>>>);

impl Links {
    pub(crate) fn add(&self, link: &Arc<LinkState>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(link));
    }

    pub(crate) fn remove(&self, link: &Arc<LinkState>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|each| !Arc::ptr_eq(each, link));
    }

    pub(crate) fn all(&self) -> Vec<Arc<LinkState>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The notice as the line a window reads: no actions, which a window answers in its own way.
pub(crate) fn line_of(notice: &Notice) -> Vec<u8> {
    let session = notice
        .session
        .as_ref()
        .map(|session| json!({"id": session.id, "title": session.title, "agent": session.agent}));
    let urgency = match notice.urgency {
        canoa::notifier::Urgency::Low => "low",
        canoa::notifier::Urgency::High => "high",
        _ => "normal",
    };
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": "notify/notice",
        "params": {
            "id": notice.id, "event": notice.event, "title": notice.title, "body": notice.body,
            "session": session, "urgency": urgency, "at": notice.at,
        },
    }))
    .unwrap_or_default()
}

/// The windows of this host.
#[derive(Default, Provider)]
#[provider(Notifier, id = "desktop", crate = ::coqui_acp_api::guatiao)]
pub struct Desktop;

impl Notifier for Desktop {
    fn describe(&self) -> Result<NotifierInfo, ProviderError> {
        let mut info = NotifierInfo::new("desktop", "Desktop");
        info.help = Some("A notice in the windows open on this host.".to_owned());
        Ok(info)
    }

    /// The host that owns the windows delivers to them itself (see [`push`]); asked by anything else, there is no window to tell.
    fn send(&self, _request: SendRequest) -> Result<Delivery, ProviderError> {
        Err(NotifierError::Unavailable("no window is open".to_owned()).into())
    }
}

/// Pushes `notice` to every window of `links`, or says that none is open.
pub(crate) fn push(links: &Links, notice: &Notice) -> Result<(), String> {
    let windows = links.all();
    if windows.is_empty() {
        return Err("no window is open".to_owned());
    }
    let line = line_of(notice);
    for window in &windows {
        window.push(line.clone());
    }
    Ok(())
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-notify",
    version = env!("CARGO_PKG_VERSION"),
    providers = [Desktop]
);
