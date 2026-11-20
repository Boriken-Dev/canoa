//! Exchange Server through its web services (EWS, SOAP over HTTPS): the backend for an on-premises mailbox. Messages are found with
//! `FindItem` and Exchange's own query language (AQS), read with `GetItem`, saved as drafts with `CreateItem` (always
//! `SaveOnly`; no request here sends), and moved, flagged, marked and categorized with `MoveItem` and `UpdateItem`. The server is
//! named by the account or found by Autodiscover from the account's address.

mod hosts;
mod parse;
mod request;
mod transport;

use core::time::Duration;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::network::Network;

use self::hosts::Hosts;
use self::request::{FolderRef, well_known};
pub(crate) use self::transport::Auth;
use self::transport::Transport;
use super::model::{
    Backend, Change, Draft, Folder, MailError, Message, Page, Query, Summary, find_folder,
};
use super::xml::Element;

/// How many Autodiscover redirects are followed.
const MAX_REDIRECTS: usize = 3;

/// What an account names.
#[derive(Clone, Debug)]
pub(crate) struct EwsSettings {
    /// The mailbox's address, which Autodiscover asks about.
    pub(crate) email: Option<String>,
    /// The EWS address (`https://mail.example.com/EWS/Exchange.asmx`); Autodiscover finds it when this is not given.
    pub(crate) server: Option<String>,
    /// The Autodiscover address to ask instead of the two the address's domain suggests.
    pub(crate) autodiscover: Option<String>,
    pub(crate) auth: Auth,
    pub(crate) timeout: Duration,
}

/// One Exchange mailbox.
#[derive(Debug)]
pub(crate) struct Ews {
    settings: EwsSettings,
    transport: Transport,
    found: Mutex<Option<String>>,
}

impl Ews {
    pub(crate) fn new(settings: EwsSettings, network: Arc<Network>) -> Self {
        let transport = Transport::new(network, settings.auth.clone(), settings.timeout);
        Self {
            settings,
            transport,
            found: Mutex::new(None),
        }
    }

    /// The EWS address: the account's, else the one Autodiscover found (asked once).
    fn url(&self) -> Result<String, MailError> {
        if let Some(server) = &self.settings.server {
            return Ok(server.clone());
        }
        if let Some(found) = self
            .found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Ok(found);
        }
        let url = self.discover()?;
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.clone());
        Ok(url)
    }

    fn discover(&self) -> Result<String, MailError> {
        let email = self.settings.email.as_deref().ok_or_else(|| {
            MailError::Bad(
                "the account names neither a server nor an address, so there is nothing to find the server by".to_owned(),
            )
        })?;
        let candidates: Vec<String> = match &self.settings.autodiscover {
            Some(url) => vec![url.clone()],
            None => {
                let domain = email.rsplit('@').next().unwrap_or(email);
                vec![
                    format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"),
                    format!("https://{domain}/autodiscover/autodiscover.xml"),
                ]
            }
        };
        let body = request::autodiscover(email);
        let trusted = Hosts::of(email, self.settings.autodiscover.as_deref());
        let mut last = None;
        for candidate in candidates {
            let mut url = candidate;
            for _ in 0..=MAX_REDIRECTS {
                let reply = match self.transport.post(&url, &body) {
                    Ok(reply) if reply.status == 200 => reply,
                    Ok(reply) => {
                        last = Some(MailError::Backend(format!(
                            "{url} answered {}",
                            reply.status
                        )));
                        break;
                    }
                    Err(error) => {
                        last = Some(error);
                        break;
                    }
                };
                match parse::ews_url(&reply.body) {
                    Ok(Some(found)) => {
                        let (next, redirect) = match found.strip_prefix("redirect:") {
                            Some(next) => (next.to_owned(), true),
                            None => (found, false),
                        };
                        // The reply is the server's word: the sign-in goes only where the account's own names say.
                        if let Err(error) = trusted.admit(&next) {
                            last = Some(error);
                            break;
                        }
                        if !redirect {
                            return Ok(next);
                        }
                        url = next;
                    }
                    Ok(None) => {
                        last = Some(MailError::Backend(format!("{url} names no EWS address")));
                        break;
                    }
                    Err(error) => {
                        last = Some(error);
                        break;
                    }
                }
            }
        }
        Err(MailError::Unreachable(format!(
            "Autodiscover found no server for {email} ({}): name it in the account's `server`",
            last.map_or_else(|| "nothing was asked".to_owned(), |error| error.to_string())
        )))
    }

    /// One operation, answered with the `Body` of the reply.
    fn call(&self, operation: &str) -> Result<Element, MailError> {
        let url = self.url()?;
        let reply = self.transport.post(&url, &request::envelope(operation))?;
        // A SOAP fault comes with a 500; anything else that is not a 200 is not Exchange's web services.
        if reply.status != 200 && reply.status != 500 {
            return Err(MailError::Backend(format!(
                "{url} answered {}: that is not an EWS address",
                reply.status
            )));
        }
        parse::body_of(&reply.body)
    }

    /// The folder a caller means, as a request names it.
    fn resolve(&self, name: &str) -> Result<FolderRef, MailError> {
        match well_known(name) {
            Some(well) => Ok(FolderRef::Well(well)),
            None => self.lookup(name),
        }
    }

    /// The folder `name` is the id, path or name of, among the mailbox's own folders.
    fn lookup(&self, name: &str) -> Result<FolderRef, MailError> {
        let folders = self.folders()?;
        find_folder(&folders, name).map(|folder| FolderRef::Id(folder.id.clone()))
    }
}

impl Backend for Ews {
    fn folders(&self) -> Result<Vec<Folder>, MailError> {
        Ok(parse::folders(&self.call(&request::find_folders())?))
    }

    fn search(&self, query: &Query) -> Result<Page, MailError> {
        let (scope, deep) = match query.folder.as_deref().map(str::trim) {
            None | Some("") => (FolderRef::Well("msgfolderroot"), true),
            Some(name) => {
                let scope = self.resolve(name)?;
                let deep = scope == FolderRef::Well("msgfolderroot");
                (scope, deep)
            }
        };
        let body = self.call(&request::find_items(&scope, deep, query))?;
        let (mut items, total, more) = parse::items(&body);
        let mut total = total;
        if let Some(flagged) = query.flagged {
            // Exchange's query language has no follow-up flag, so the page is filtered here and its count is not known.
            items.retain(|item| item.flagged == flagged);
            total = None;
        }
        Ok(Page { items, total, more })
    }

    fn read(&self, id: &str) -> Result<Message, MailError> {
        let body = self.call(&request::get_items(&[id.to_owned()], true))?;
        parse::got(&body)
            .first()
            .and_then(parse::message)
            .ok_or_else(|| MailError::Bad(format!("no message has the id {id}")))
    }

    fn find_by_message_id(&self, message_id: &str) -> Result<Option<String>, MailError> {
        let body = self.call(&request::find_by_message_id(message_id))?;
        let (items, _, _) = parse::items(&body);
        Ok(items.into_iter().next().map(|item| item.id))
    }

    fn summaries(&self, ids: &[String]) -> Result<Vec<Summary>, MailError> {
        let body = self.call(&request::get_items(ids, false))?;
        let found: Vec<Summary> = parse::got(&body)
            .iter()
            .filter_map(parse::summary)
            .collect();
        if found.len() == ids.len() {
            Ok(found)
        } else {
            Err(MailError::Bad(
                "Exchange did not find every message named".to_owned(),
            ))
        }
    }

    fn draft(&self, draft: &Draft) -> Result<Summary, MailError> {
        let body = self.call(&request::create_draft(draft))?;
        let mut saved = parse::created(&body).ok_or_else(|| {
            MailError::Backend("Exchange saved the draft but gave no id".to_owned())
        })?;
        match draft {
            Draft::New { subject, .. } => saved.subject.clone_from(subject),
            Draft::Reply { .. } => {
                if let Ok(read) = self.read(&saved.id) {
                    saved = read.summary;
                }
            }
        }
        Ok(saved)
    }

    fn change(&self, ids: &[String], change: &Change) -> Result<(), MailError> {
        if ids.is_empty() {
            return Ok(());
        }
        match change {
            Change::Move { folder } => {
                let to = self.resolve(folder)?;
                self.call(&request::move_items(ids, &to)).map(drop)
            }
            Change::Archive => {
                match self.call(&request::move_items(ids, &FolderRef::Well("archive"))) {
                    Err(MailError::Backend(why)) if why.contains("FolderNotFound") => {
                        let archive = self.lookup("Archive").map_err(|_| {
                            MailError::Bad(
                                "the mailbox has no Archive folder: move the messages to a folder named by mail_list".to_owned(),
                            )
                        })?;
                        self.call(&request::move_items(ids, &archive)).map(drop)
                    }
                    other => other.map(drop),
                }
            }
            Change::Read(_) | Change::Flag(_) => self
                .call(&request::update_items(ids, change, &[]))
                .map(drop),
            Change::Categories { add, remove } => {
                let current = parse::categories(&self.call(&request::get_items(ids, false))?);
                if current.len() != ids.len() {
                    return Err(MailError::Backend(
                        "Exchange did not give the categories of every message".to_owned(),
                    ));
                }
                let after: Vec<Vec<String>> = current
                    .into_iter()
                    .map(|mut kept| {
                        kept.retain(|name| {
                            !remove.iter().any(|gone| gone.eq_ignore_ascii_case(name))
                        });
                        for name in add {
                            if !kept.iter().any(|have| have.eq_ignore_ascii_case(name)) {
                                kept.push(name.clone());
                            }
                        }
                        kept
                    })
                    .collect();
                self.call(&request::update_items(ids, change, &after))
                    .map(drop)
            }
        }
    }
}
