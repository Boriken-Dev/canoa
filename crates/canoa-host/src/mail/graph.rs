//! Exchange Online through Microsoft Graph: the backend for a mailbox in the cloud, where Exchange's web services are going away.
//! Messages are found with `$search` or `$filter` on `/me/messages`, read with a `GET` that expands the attachments, saved as
//! drafts with `POST /me/messages` or `createReply` (the app asks for `Mail.ReadWrite` only, so no request here can send), and
//! moved, flagged, marked and categorized with `move` and `PATCH`. Every request carries a bearer token a [`TokenSource`] gave, by
//! default the person's own Windows sign-in taken silently for an app registration the account names.

mod token;
mod wire;

#[cfg(test)]
mod standin;
#[cfg(test)]
mod tests;

use core::time::Duration;
use std::sync::Arc;

use canoa::network::{Network, Secret};
use serde_json::{Value, json};

use self::token::Tokens;
pub(crate) use self::token::{TokenSource, Wam};
use self::wire::{FOLDER_FIELDS, LIST_FIELDS, Plan, encode, folder, message, plan, quote, summary};
use super::model::{
    Backend, Change, Draft, FlagState, Folder, MailError, Message, Page, Query, Summary,
    find_folder, well_known,
};

/// Graph's address, which an account may replace.
pub(crate) const DEFAULT_SERVER: &str = "https://graph.microsoft.com/v1.0";

/// The most bytes read from one answer: a page of a message list or one message with its body.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

/// The most messages asked of Graph in one page.
const PAGE: usize = 100;

/// What every request asks for: ids that stay the same when a message moves, and a body as plain text.
const PREFER: &str = "IdType=\"ImmutableId\", outlook.body-content-type=\"text\"";

/// What an account names.
#[derive(Clone, Debug)]
pub(crate) struct GraphSettings {
    /// Graph's address (`https://graph.microsoft.com/v1.0`).
    pub(crate) server: String,
    pub(crate) timeout: Duration,
}

/// One Exchange Online mailbox.
#[derive(Debug)]
pub(crate) struct Graph {
    settings: GraphSettings,
    tokens: Tokens,
    network: Arc<Network>,
}

/// What Graph answered.
struct Reply {
    status: u16,
    retry_after: Option<String>,
    body: String,
}

/// A request that has a body, or has none.
#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Patch,
}

impl Graph {
    pub(crate) fn new(
        settings: GraphSettings,
        tokens: Arc<dyn TokenSource>,
        network: Arc<Network>,
    ) -> Self {
        Self {
            settings: GraphSettings {
                server: settings.server.trim_end_matches('/').to_owned(),
                ..settings
            },
            tokens: Tokens::new(tokens),
            network,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.settings.server)
    }

    /// One request, signed in with the token held; Graph's refusal of it is answered once with a new token.
    fn call(&self, method: Method, url: &str, body: Option<&Value>) -> Result<Value, MailError> {
        let mut reply = self.send(method, url, body, &self.tokens.get()?)?;
        if reply.status == 401 {
            self.tokens.forget();
            reply = self.send(method, url, body, &self.tokens.get()?)?;
        }
        Self::finish(&reply)
    }

    fn get(&self, url: &str) -> Result<Value, MailError> {
        self.call(Method::Get, url, None)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, MailError> {
        self.call(Method::Post, &self.url(path), Some(body))
    }

    fn patch(&self, path: &str, body: &Value) -> Result<Value, MailError> {
        self.call(Method::Patch, &self.url(path), Some(body))
    }

    fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        token: &Secret,
    ) -> Result<Reply, MailError> {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(self.settings.timeout))
            .build();
        let agent = self.network.agent(config).map_err(|error| {
            MailError::Unreachable(format!("the network is not usable: {error}"))
        })?;
        let authorization = format!("Bearer {}", token.expose());
        let unreachable = |error: ureq::Error| {
            // The address holds no secret: the token is in a header, never in the URL.
            MailError::Unreachable(format!("{url}: {error}"))
        };
        let response = match method {
            Method::Get => agent
                .get(url)
                .header("authorization", &authorization)
                .header("prefer", PREFER)
                .header("accept", "application/json")
                .call(),
            Method::Post | Method::Patch => {
                let request = if matches!(method, Method::Post) {
                    agent.post(url)
                } else {
                    agent.patch(url)
                };
                request
                    .header("authorization", &authorization)
                    .header("prefer", PREFER)
                    .header("accept", "application/json")
                    .header("content-type", "application/json")
                    .send(body.map_or_else(|| "{}".to_owned(), Value::to_string))
            }
        }
        .map_err(unreachable)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .into_body()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
            .map_err(|error| MailError::Unreachable(format!("{url}: {error}")))?;
        Ok(Reply {
            status,
            retry_after,
            body,
        })
    }

    /// What a reply means: its JSON, or why the call failed in the words Graph used.
    fn finish(reply: &Reply) -> Result<Value, MailError> {
        let json: Value = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
        if (200..300).contains(&reply.status) {
            return Ok(json);
        }
        let code = json["error"]["code"].as_str().unwrap_or("an error");
        let said = json["error"]["message"].as_str().unwrap_or_default();
        Err(match reply.status {
            401 => MailError::Auth(format!(
                "Graph refused the token ({code}): the app registration may be for another tenant, or the sign-in has gone stale"
            )),
            403 => MailError::Auth(format!(
                "Graph refused the call ({code}): the app registration needs the delegated permission Mail.ReadWrite, \
                 allowed for the person or by an administrator"
            )),
            429 | 503 => MailError::Backend(format!(
                "Graph is busy ({}): try again{}",
                reply.status,
                reply
                    .retry_after
                    .as_deref()
                    .map_or_else(String::new, |after| format!(" in {after} seconds"))
            )),
            _ => MailError::Backend(format!("Graph said {code}: {said}")),
        })
    }

    /// The address Graph named for the next page, when it is Graph's own: the token goes to nowhere else.
    fn next(&self, body: &Value) -> Result<Option<String>, MailError> {
        let Some(next) = body["@odata.nextLink"].as_str() else {
            return Ok(None);
        };
        let origin = |address: &str| {
            url::Url::parse(address).ok().map(|u| {
                (
                    u.scheme().to_owned(),
                    u.host_str().map(str::to_owned),
                    u.port_or_known_default(),
                )
            })
        };
        if origin(next) == origin(&self.settings.server) && origin(next).is_some() {
            Ok(Some(next.to_owned()))
        } else {
            Err(MailError::Backend(
                "Graph named a next page at another address, which the token is not sent to"
                    .to_owned(),
            ))
        }
    }

    /// Every item of a collection, page after page.
    fn collect(&self, first: &str) -> Result<Vec<Value>, MailError> {
        let mut url = first.to_owned();
        let mut items = Vec::new();
        loop {
            let body = self.get(&url)?;
            if let Some(page) = body["value"].as_array() {
                items.extend(page.iter().cloned());
            }
            match self.next(&body)? {
                Some(next) => url = next,
                None => return Ok(items),
            }
        }
    }

    /// The messages path a search runs on: the whole mailbox, or one folder.
    fn scope(&self, name: Option<&str>) -> Result<String, MailError> {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok("/me/messages".to_owned());
        };
        Ok(match well_known(name) {
            Some("msgfolderroot") => "/me/messages".to_owned(),
            Some(well) => format!("/me/mailFolders/{well}/messages"),
            None => {
                let folders = self.folders()?;
                format!(
                    "/me/mailFolders/{}/messages",
                    encode(&find_folder(&folders, name)?.id)
                )
            }
        })
    }

    /// The folder id or well-known name a move goes to.
    fn destination(&self, name: &str) -> Result<String, MailError> {
        match well_known(name) {
            Some("msgfolderroot") | None => {
                let folders = self.folders()?;
                Ok(find_folder(&folders, name)?.id.clone())
            }
            Some(well) => Ok(well.to_owned()),
        }
    }

    fn move_one(&self, id: &str, destination: &str) -> Result<(), MailError> {
        self.post(
            &format!("/me/messages/{}/move", encode(id)),
            &json!({"destinationId": destination}),
        )
        .map(drop)
    }

    /// Applies `each` to every id, and says how far it got when one fails.
    fn each(
        ids: &[String],
        mut each: impl FnMut(&str) -> Result<(), MailError>,
    ) -> Result<(), MailError> {
        for (done, id) in ids.iter().enumerate() {
            each(id).map_err(|error| {
                if done == 0 {
                    error
                } else {
                    match error {
                        MailError::Backend(why) => MailError::Backend(format!(
                            "{why} (after {done} of {} messages)",
                            ids.len()
                        )),
                        other => other,
                    }
                }
            })?;
        }
        Ok(())
    }
}

impl Backend for Graph {
    fn folders(&self) -> Result<Vec<Folder>, MailError> {
        let mut out: Vec<Folder> = Vec::new();
        // Each folder's children follow it, so a path is built from its parent's.
        let mut pending: Vec<(String, String)> = vec![(
            self.url(&format!(
                "/me/mailFolders?$top={PAGE}&$select={FOLDER_FIELDS}"
            )),
            String::new(),
        )];
        while let Some((url, parent)) = pending.pop() {
            let mut level = Vec::new();
            for value in self.collect(&url)? {
                let made = folder(&value, &parent);
                if value["childFolderCount"].as_u64().unwrap_or(0) > 0 {
                    level.push((
                        self.url(&format!(
                            "/me/mailFolders/{}/childFolders?$top={PAGE}&$select={FOLDER_FIELDS}",
                            encode(&made.id)
                        )),
                        made.path.clone(),
                    ));
                }
                out.push(made);
            }
            level.reverse();
            pending.extend(level);
        }
        Ok(out)
    }

    fn search(&self, query: &Query) -> Result<Page, MailError> {
        let plan: Plan = plan(query)?;
        let scope = self.scope(query.folder.as_deref())?;
        let limit = query.limit.max(1);
        let wanted = query.offset + limit + 1;
        let mut params = vec![
            format!("$select={LIST_FIELDS}"),
            format!("$top={}", wanted.min(PAGE)),
        ];
        if let Some(search) = &plan.search {
            params.push(format!("$search={}", encode(&format!("\"{search}\""))));
        } else {
            params.push("$orderby=receivedDateTime%20desc".to_owned());
            params.push("$count=true".to_owned());
            if let Some(filter) = &plan.filter {
                params.push(format!("$filter={}", encode(filter)));
            }
        }
        let mut url = self.url(&format!("{scope}?{}", params.join("&")));
        let mut skipped = 0_usize;
        let mut items: Vec<Summary> = Vec::new();
        let mut more = false;
        let mut total = None;
        let mut first = true;
        'pages: loop {
            let body = self.get(&url)?;
            if first {
                first = false;
                total = body["@odata.count"]
                    .as_u64()
                    .and_then(|count| usize::try_from(count).ok())
                    .filter(|_| !plan.filters_here());
            }
            for value in body["value"].as_array().into_iter().flatten() {
                let message = summary(value);
                if !plan.keeps(&message) {
                    continue;
                }
                if skipped < query.offset {
                    skipped += 1;
                } else if items.len() < limit {
                    items.push(message);
                } else {
                    more = true;
                    break 'pages;
                }
            }
            match self.next(&body)? {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(Page { items, total, more })
    }

    fn read(&self, id: &str) -> Result<Message, MailError> {
        let value = self.get(&self.url(&format!(
            "/me/messages/{}?$select={LIST_FIELDS},body,internetMessageId&$expand=attachments($select=name,size,contentType)",
            encode(id)
        )))?;
        Ok(message(&value))
    }

    fn summaries(&self, ids: &[String]) -> Result<Vec<Summary>, MailError> {
        ids.iter()
            .map(|id| {
                self.get(&self.url(&format!(
                    "/me/messages/{}?$select={LIST_FIELDS}",
                    encode(id)
                )))
                .map(|value| summary(&value))
            })
            .collect()
    }

    fn thread(&self, summary_of: &Summary) -> Result<Vec<Summary>, MailError> {
        let Some(conversation) = &summary_of.conversation else {
            return Ok(vec![summary_of.clone()]);
        };
        let filter = format!("conversationId eq {}", quote(conversation));
        let found = self.collect(&self.url(&format!(
            "/me/messages?$select={LIST_FIELDS}&$top={PAGE}&$filter={}",
            encode(&filter)
        )))?;
        let mut items: Vec<Summary> = found.iter().map(summary).collect();
        items.sort_by(|a, b| a.received.cmp(&b.received));
        Ok(items)
    }

    fn draft(&self, draft: &Draft) -> Result<Summary, MailError> {
        let recipients = |addresses: &[String]| -> Vec<Value> {
            addresses
                .iter()
                .map(|address| json!({"emailAddress": {"address": address}}))
                .collect()
        };
        let saved = match draft {
            Draft::New {
                to,
                cc,
                bcc,
                subject,
                body,
            } => self.post(
                "/me/messages",
                &json!({
                    "subject": subject,
                    "body": {"contentType": "text", "content": body},
                    "toRecipients": recipients(to),
                    "ccRecipients": recipients(cc),
                    "bccRecipients": recipients(bcc),
                }),
            )?,
            Draft::Reply { to, all, body } => self.post(
                &format!(
                    "/me/messages/{}/{}",
                    encode(to),
                    if *all {
                        "createReplyAll"
                    } else {
                        "createReply"
                    }
                ),
                &json!({"comment": body}),
            )?,
        };
        let saved = summary(&saved);
        if saved.id.is_empty() {
            return Err(MailError::Backend(
                "Graph saved the draft but gave no id".to_owned(),
            ));
        }
        Ok(saved)
    }

    fn change(&self, ids: &[String], change: &Change) -> Result<(), MailError> {
        if ids.is_empty() {
            return Ok(());
        }
        match change {
            Change::Move { folder } => {
                let to = self.destination(folder)?;
                Self::each(ids, |id| self.move_one(id, &to))
            }
            Change::Archive => {
                let first = self.move_one(&ids[0], "archive");
                let to = match first {
                    Ok(()) => "archive".to_owned(),
                    Err(MailError::Backend(why)) if why.contains("ErrorFolderNotFound") => {
                        let folders = self.folders()?;
                        let archive = find_folder(&folders, "Archive").map_err(|_| {
                            MailError::Bad(
                                "the mailbox has no Archive folder: move the messages to a folder named by mail_list".to_owned(),
                            )
                        })?;
                        self.move_one(&ids[0], &archive.id)?;
                        archive.id.clone()
                    }
                    Err(other) => return Err(other),
                };
                Self::each(&ids[1..], |id| self.move_one(id, &to))
            }
            Change::Read(read) => Self::each(ids, |id| {
                self.patch(
                    &format!("/me/messages/{}", encode(id)),
                    &json!({"isRead": read}),
                )
                .map(drop)
            }),
            Change::Flag(state) => {
                let status = match state {
                    FlagState::Flagged => "flagged",
                    FlagState::Complete => "complete",
                    FlagState::None => "notFlagged",
                };
                Self::each(ids, |id| {
                    self.patch(
                        &format!("/me/messages/{}", encode(id)),
                        &json!({"flag": {"flagStatus": status}}),
                    )
                    .map(drop)
                })
            }
            Change::Categories { add, remove } => Self::each(ids, |id| {
                let path = format!("/me/messages/{}", encode(id));
                let current = self.get(&self.url(&format!("{path}?$select=categories")))?;
                let mut kept: Vec<String> = current["categories"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|name| name.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                kept.retain(|name| !remove.iter().any(|gone| gone.eq_ignore_ascii_case(name)));
                for name in add {
                    if !kept.iter().any(|have| have.eq_ignore_ascii_case(name)) {
                        kept.push(name.clone());
                    }
                }
                self.patch(&path, &json!({"categories": kept})).map(drop)
            }),
        }
    }
}
