//! A stand-in for Microsoft Graph on loopback, for the mail tests: the sample mailbox of the Exchange stand-in behind the routes the
//! backend uses, with the bearer token a real Graph asks for. It follows the limits the backend works around (`$search` cannot be
//! combined with `$filter` or `$orderby`, a sorted property must open the `$filter`), answers the queries the backend writes and
//! refuses what it does not know, so a query the backend gets wrong fails here instead of matching everything. It pages with
//! `@odata.nextLink`, keeps only the fields `$select` names, and records every request and anything that would send mail.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use crate::mail::standin::{Mail, Mailbox};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One request the stand-in served.
#[derive(Clone, Debug)]
pub(crate) struct Logged {
    pub(crate) method: String,
    /// The path after the version, as sent (still percent-encoded).
    pub(crate) path: String,
    /// The query, as sent.
    pub(crate) query: String,
    pub(crate) authorization: Option<String>,
    pub(crate) prefer: Option<String>,
    pub(crate) body: String,
}

impl Logged {
    /// The value of query parameter `name`, decoded.
    pub(crate) fn param(&self, name: &str) -> Option<String> {
        url::form_urlencoded::parse(self.query.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    log: Mutex<Vec<Logged>>,
    sent: Mutex<Vec<String>>,
    token: Mutex<String>,
    page_size: AtomicUsize,
    next: AtomicU32,
    /// The address `@odata.nextLink` names instead of the stand-in's own.
    foreign: Mutex<Option<String>>,
    stop: AtomicBool,
    addr: SocketAddr,
}

/// The running stand-in; it stops when dropped.
pub(crate) struct Standin {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Standin {
    /// A Graph that accepts `token` as the bearer token.
    pub(crate) fn start(mailbox: Mailbox, token: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(mailbox),
            log: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
            token: Mutex::new(token.to_owned()),
            page_size: AtomicUsize::new(100),
            next: AtomicU32::new(100),
            foreign: Mutex::new(None),
            stop: AtomicBool::new(false),
            addr,
        });
        let serving = Arc::clone(&shared);
        let thread = thread::spawn(move || {
            while !serving.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = Arc::clone(&serving);
                        thread::spawn(move || {
                            let _ = serve(&shared, stream);
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Graph's address, as an account names it.
    pub(crate) fn server(&self) -> String {
        format!("http://{}/v1.0", self.shared.addr)
    }

    pub(crate) fn mailbox(&self) -> Mailbox {
        lock(&self.shared.mailbox).clone()
    }

    pub(crate) fn log(&self) -> Vec<Logged> {
        lock(&self.shared.log).clone()
    }

    /// Everything that would have sent a message.
    pub(crate) fn sent(&self) -> Vec<String> {
        lock(&self.shared.sent).clone()
    }

    /// How many items a page holds at most, so that a search reaches `@odata.nextLink`.
    pub(crate) fn page_size(&self, size: usize) {
        self.shared.page_size.store(size, Ordering::SeqCst);
    }

    /// Makes every next page name `address` instead of the stand-in.
    pub(crate) fn name_next_at(&self, address: &str) {
        *lock(&self.shared.foreign) = Some(address.to_owned());
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(shared: &Shared, mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut data = Vec::new();
    let mut chunk = [0_u8; 4096];
    let (head_len, content_length, method, target, authorization, prefer) = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        data.extend_from_slice(&chunk[..read]);
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut request = httparse::Request::new(&mut headers);
        if let Ok(httparse::Status::Complete(len)) = request.parse(&data) {
            let header = |name: &str| {
                request
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case(name))
                    .map(|h| String::from_utf8_lossy(h.value).into_owned())
            };
            break (
                len,
                header("content-length")
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(0),
                request.method.unwrap_or("GET").to_owned(),
                request.path.unwrap_or("/").to_owned(),
                header("authorization"),
                header("prefer"),
            );
        }
    };
    while data.len() < head_len + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(&data[head_len..]).into_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let path = path.strip_prefix("/v1.0").unwrap_or(path).to_owned();
    let logged = Logged {
        method,
        path,
        query: query.to_owned(),
        authorization,
        prefer,
        body,
    };
    lock(&shared.log).push(logged.clone());
    let (status, answer) = respond(shared, &logged);
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        answer.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(answer.as_bytes())
}

type Answer = (u16, Value);

fn failure(status: u16, code: &str, message: &str) -> Answer {
    (status, json!({"error": {"code": code, "message": message}}))
}

fn respond(shared: &Shared, request: &Logged) -> (u16, String) {
    let valid = format!("Bearer {}", lock(&shared.token));
    let (status, value) = if request.authorization.as_deref() == Some(valid.as_str()) {
        route(shared, request)
    } else {
        failure(
            401,
            "InvalidAuthenticationToken",
            "Access token is empty or invalid",
        )
    };
    (status, value.to_string())
}

fn segments(path: &str) -> Vec<String> {
    path.trim_matches('/')
        .split('/')
        .map(|segment| percent_decode_str(segment).decode_utf8_lossy().into_owned())
        .collect()
}

fn route(shared: &Shared, request: &Logged) -> Answer {
    let parts = segments(&request.path);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let mut mailbox = lock(&shared.mailbox);
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["me", "mailFolders"]) => folders(&mailbox, None, request),
        ("GET", ["me", "mailFolders", id, "childFolders"]) => match folder_id(&mailbox, id) {
            Some(parent) => folders(&mailbox, Some(&parent), request),
            None => no_folder(),
        },
        ("GET", ["me", "mailFolders", id, "messages"]) => match folder_id(&mailbox, id) {
            Some(folder) => list(shared, &mailbox, Some(&folder), request),
            None => no_folder(),
        },
        ("GET", ["me", "messages"]) => list(shared, &mailbox, None, request),
        ("GET", ["me", "messages", id]) => match mailbox.mail.iter().find(|m| m.id == *id) {
            Some(mail) => (200, one(mail, request)),
            None => no_item(),
        },
        ("POST", ["me", "messages"]) => create(shared, &mut mailbox, request),
        ("PATCH", ["me", "messages", id]) => update(&mut mailbox, id, request),
        ("POST", ["me", "messages", id, "move"]) => relocate(&mut mailbox, id, request),
        (
            "POST",
            [
                "me",
                "messages",
                id,
                action @ ("createReply" | "createReplyAll"),
            ],
        ) => reply(
            shared,
            &mut mailbox,
            id,
            *action == "createReplyAll",
            request,
        ),
        ("POST", [.., "send" | "sendMail" | "reply" | "replyAll" | "forward"]) => {
            lock(&shared.sent).push(request.path.clone());
            (202, json!({}))
        }
        _ => failure(404, "ErrorInvalidRequest", "no such route"),
    }
}

fn no_folder() -> Answer {
    failure(
        404,
        "ErrorFolderNotFound",
        "The specified folder could not be found in the store.",
    )
}

fn no_item() -> Answer {
    failure(
        404,
        "ErrorItemNotFound",
        "The specified object was not found in the store.",
    )
}

/// The id of the folder a path names, by id or by well-known name.
fn folder_id(mailbox: &Mailbox, name: &str) -> Option<String> {
    mailbox
        .folders
        .iter()
        .find(|folder| folder.id == name || folder.well == Some(name))
        .map(|folder| folder.id.clone())
}

fn folders(mailbox: &Mailbox, parent: Option<&str>, request: &Logged) -> Answer {
    if request.param("$select").is_none() {
        return failure(
            400,
            "BadRequest",
            "the backend asks for the fields it needs",
        );
    }
    let items: Vec<Value> = mailbox
        .folders
        .iter()
        .filter(|folder| folder.parent.as_deref() == parent)
        .map(|folder| {
            let here: Vec<&Mail> = mailbox
                .mail
                .iter()
                .filter(|mail| mail.folder == folder.id)
                .collect();
            let children = mailbox
                .folders
                .iter()
                .filter(|child| child.parent.as_deref() == Some(folder.id.as_str()))
                .count();
            json!({
                "id": folder.id,
                "displayName": folder.name,
                "parentFolderId": folder.parent,
                "childFolderCount": children,
                "totalItemCount": here.len(),
                "unreadItemCount": here.iter().filter(|mail| !mail.read).count(),
            })
        })
        .collect();
    (200, json!({"value": items}))
}

fn person((name, address): &(String, String)) -> Value {
    json!({"emailAddress": {"name": name, "address": address}})
}

/// A message as Graph shows it, with the fields `$select` names (all of them without one).
fn one(mail: &Mail, request: &Logged) -> Value {
    let text = request
        .prefer
        .as_deref()
        .is_some_and(|prefer| prefer.contains("outlook.body-content-type=\"text\""));
    let content = if text {
        mail.body.clone()
    } else {
        format!("<html><body><p>{}</p></body></html>", mail.body)
    };
    let flag = match mail.flag.as_str() {
        "Flagged" => "flagged",
        "Complete" => "complete",
        _ => "notFlagged",
    };
    let all = json!({
        "id": mail.id,
        "conversationId": mail.conversation,
        "parentFolderId": mail.folder,
        "subject": mail.subject,
        "from": person(&mail.from),
        "toRecipients": mail.to.iter().map(person).collect::<Vec<_>>(),
        "ccRecipients": [],
        "receivedDateTime": mail.received,
        "isRead": mail.read,
        "hasAttachments": mail.attachments,
        "flag": {"flagStatus": flag},
        "categories": mail.categories,
        "bodyPreview": mail.body.chars().take(40).collect::<String>(),
        "body": {"contentType": if text { "text" } else { "html" }, "content": content},
        "internetMessageId": format!("<{}@example.test>", mail.id),
    });
    let mut kept = serde_json::Map::new();
    match request.param("$select") {
        Some(select) => {
            let wanted: Vec<&str> = select.split(',').collect();
            for (key, value) in all.as_object().expect("an object") {
                if key == "id" || wanted.contains(&key.as_str()) {
                    kept.insert(key.clone(), value.clone());
                }
            }
        }
        None => kept = all.as_object().expect("an object").clone(),
    }
    if request
        .param("$expand")
        .is_some_and(|expand| expand.starts_with("attachments"))
    {
        let files = if mail.attachments {
            vec![json!({"name": "plan.pdf", "size": 2048, "contentType": "application/pdf"})]
        } else {
            Vec::new()
        };
        kept.insert("attachments".to_owned(), json!(files));
    }
    Value::Object(kept)
}

fn list(shared: &Shared, mailbox: &Mailbox, folder: Option<&str>, request: &Logged) -> Answer {
    let search = request.param("$search");
    let filter = request.param("$filter");
    let order = request.param("$orderby");
    if request.param("$select").is_none() {
        return failure(
            400,
            "BadRequest",
            "the backend asks for the fields it needs",
        );
    }
    if search.is_some() && (filter.is_some() || order.is_some() || request.param("$skip").is_some())
    {
        return failure(
            400,
            "BadRequest",
            "$search cannot be combined with $filter, $orderby or $skip",
        );
    }
    if filter.is_some()
        && order.is_some()
        && !order_follows_filter(filter.as_deref(), order.as_deref())
    {
        return failure(
            400,
            "InefficientFilter",
            "The restriction or sort order is too complex for this operation.",
        );
    }
    let mut found: Vec<&Mail> = Vec::new();
    for mail in &mailbox.mail {
        if folder.is_some_and(|folder| mail.folder != folder) {
            continue;
        }
        if let Some(search) = &search {
            match kql(mail, search) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(why) => return failure(400, "BadRequest", &why),
            }
        }
        if let Some(filter) = &filter {
            match odata(mail, filter) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(why) => return failure(400, "BadRequest", &why),
            }
        }
        found.push(mail);
    }
    found.sort_by(|a, b| b.received.cmp(&a.received));
    let total = found.len();
    let size = request
        .param("$top")
        .and_then(|top| top.parse::<usize>().ok())
        .unwrap_or(10)
        .min(shared.page_size.load(Ordering::SeqCst));
    let start: usize = request
        .param("$skiptoken")
        .and_then(|token| token.parse().ok())
        .unwrap_or(0);
    let page: Vec<Value> = found
        .iter()
        .skip(start)
        .take(size)
        .map(|mail| one(mail, request))
        .collect();
    let mut answer = json!({"value": page});
    if request.param("$count").as_deref() == Some("true") && search.is_none() {
        answer["@odata.count"] = json!(total);
    }
    if start + size < total {
        let base = lock(&shared.foreign)
            .clone()
            .unwrap_or_else(|| format!("http://{}/v1.0", shared.addr));
        let path = &request.path;
        let query: Vec<String> = request
            .query
            .split('&')
            .filter(|pair| !pair.starts_with("$skiptoken="))
            .map(str::to_owned)
            .collect();
        answer["@odata.nextLink"] = json!(format!(
            "{base}{path}?{}&$skiptoken={}",
            query.join("&"),
            start + size
        ));
    }
    (200, answer)
}

/// Whether a `$filter` and `$orderby` together are ones Graph answers: the sorted property opens the filter.
fn order_follows_filter(filter: Option<&str>, order: Option<&str>) -> bool {
    let property = order
        .and_then(|order| order.split_whitespace().next())
        .unwrap_or_default();
    filter.is_some_and(|filter| filter.starts_with(property))
}

fn unquote(value: &str) -> Result<String, String> {
    value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .map(|v| v.replace("''", "'"))
        .ok_or_else(|| format!("the value {value} is not quoted"))
}

/// Evaluates a `$filter` of clauses joined by `and`; a clause it does not know is an error.
fn odata(mail: &Mail, filter: &str) -> Result<bool, String> {
    for clause in filter.split(" and ") {
        let words: Vec<&str> = clause.trim().splitn(3, ' ').collect();
        let ok = match words.as_slice() {
            ["receivedDateTime", "ge", value] => mail.received.as_str() >= *value,
            ["receivedDateTime", "le", value] => mail.received.as_str() <= *value,
            ["isRead", "eq", value] => mail.read == (*value == "true"),
            ["hasAttachments", "eq", value] => mail.attachments == (*value == "true"),
            ["flag/flagStatus", "eq", value] => {
                let status = unquote(value)?;
                match status.as_str() {
                    "flagged" => mail.flag == "Flagged",
                    "complete" => mail.flag == "Complete",
                    "notFlagged" => mail.flag == "NotFlagged",
                    _ => return Err(format!("no flag status is named {status}")),
                }
            }
            ["conversationId", "eq", value] => mail.conversation == unquote(value)?,
            ["internetMessageId", "eq", value] => {
                format!("<{}@example.test>", mail.id) == unquote(value)?
            }
            _ => return Err(format!("cannot read the filter clause {clause}")),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluates a `$search` value of KQL terms joined by `AND`; a term it does not know is an error.
fn kql(mail: &Mail, search: &str) -> Result<bool, String> {
    let query = search
        .strip_prefix('"')
        .and_then(|q| q.strip_suffix('"'))
        .ok_or_else(|| format!("$search is not in double quotes: {search}"))?;
    let quoted = |value: &str| {
        value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .map(str::to_lowercase)
            .ok_or_else(|| format!("the term {value} is not quoted"))
    };
    for part in query.split(" AND ") {
        let part = part.trim();
        let ok = if let Some(inner) = part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
            let haystack = format!("{} {} {}", mail.subject, mail.body, mail.from.1).to_lowercase();
            inner.split(" OR ").any(|alternative| {
                alternative
                    .split_whitespace()
                    .all(|word| haystack.contains(&word.to_lowercase()))
            })
        } else if let Some(value) = part.strip_prefix("from:") {
            let value = quoted(value)?;
            mail.from.0.to_lowercase().contains(&value)
                || mail.from.1.to_lowercase().contains(&value)
        } else if let Some(value) = part.strip_prefix("to:") {
            let value = quoted(value)?;
            mail.to.iter().any(|(n, a)| {
                n.to_lowercase().contains(&value) || a.to_lowercase().contains(&value)
            })
        } else if let Some(value) = part.strip_prefix("subject:") {
            mail.subject.to_lowercase().contains(&quoted(value)?)
        } else if let Some(day) = part.strip_prefix("received>=") {
            mail.received[..10] >= *date(day)?
        } else if let Some(day) = part.strip_prefix("received<=") {
            mail.received[..10] <= *date(day)?
        } else if let Some(value) = part.strip_prefix("hasattachments:") {
            mail.attachments == (value == "true")
        } else {
            return Err(format!("cannot read the search term {part}"));
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

fn date(text: &str) -> Result<&str, String> {
    if text.len() == 10 && text.as_bytes()[4] == b'-' && text.as_bytes()[7] == b'-' {
        Ok(text)
    } else {
        Err(format!("cannot read the date {text}"))
    }
}

fn new_id(shared: &Shared) -> String {
    format!("AAMk/{}+x=", shared.next.fetch_add(1, Ordering::SeqCst))
}

fn addresses(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let address = entry["emailAddress"]["address"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            (address.clone(), address)
        })
        .collect()
}

/// Saves a draft in Drafts: nothing is sent.
fn create(shared: &Shared, mailbox: &mut Mailbox, request: &Logged) -> Answer {
    let Ok(body) = serde_json::from_str::<Value>(&request.body) else {
        return failure(400, "BadRequest", "the body is not JSON");
    };
    if body["body"]["contentType"] != "text" {
        return failure(400, "BadRequest", "the stand-in takes text drafts");
    }
    let Some(drafts) = folder_id(mailbox, "drafts") else {
        return no_folder();
    };
    let mail = Mail {
        id: new_id(shared),
        folder: drafts,
        conversation: format!("D{}", shared.next.load(Ordering::SeqCst)),
        subject: body["subject"].as_str().unwrap_or_default().to_owned(),
        from: ("Me".to_owned(), "me@example.test".to_owned()),
        to: addresses(&body["toRecipients"]),
        received: "2026-11-14T00:00:00Z".to_owned(),
        read: true,
        attachments: false,
        categories: Vec::new(),
        flag: "NotFlagged".to_owned(),
        body: body["body"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    };
    let shown = one(&mail, &selecting_all(request));
    mailbox.mail.push(mail);
    (201, shown)
}

/// A request that selects everything, for the answer to a write.
fn selecting_all(request: &Logged) -> Logged {
    Logged {
        query: String::new(),
        ..request.clone()
    }
}

/// A draft of a reply, quoting the message.
fn reply(shared: &Shared, mailbox: &mut Mailbox, id: &str, all: bool, request: &Logged) -> Answer {
    let Some(original) = mailbox.mail.iter().find(|mail| mail.id == id).cloned() else {
        return no_item();
    };
    let Ok(body) = serde_json::from_str::<Value>(&request.body) else {
        return failure(400, "BadRequest", "the body is not JSON");
    };
    let Some(drafts) = folder_id(mailbox, "drafts") else {
        return no_folder();
    };
    let mut to = vec![original.from.clone()];
    if all {
        to.extend(
            original
                .to
                .iter()
                .filter(|p| p.1 != "me@example.test")
                .cloned(),
        );
    }
    let mail = Mail {
        id: new_id(shared),
        folder: drafts,
        conversation: original.conversation.clone(),
        subject: format!("RE: {}", original.subject.trim_start_matches("Re: ")),
        from: ("Me".to_owned(), "me@example.test".to_owned()),
        to,
        received: "2026-11-14T00:00:00Z".to_owned(),
        read: true,
        attachments: false,
        categories: Vec::new(),
        flag: "NotFlagged".to_owned(),
        body: format!(
            "{}\n\nFrom: {}\n{}",
            body["comment"].as_str().unwrap_or_default(),
            original.from.1,
            original.body
        ),
    };
    let shown = one(&mail, &selecting_all(request));
    mailbox.mail.push(mail);
    (201, shown)
}

/// Changes the fields a PATCH names; a field the stand-in does not know is an error.
fn update(mailbox: &mut Mailbox, id: &str, request: &Logged) -> Answer {
    let Ok(body) = serde_json::from_str::<Value>(&request.body) else {
        return failure(400, "BadRequest", "the body is not JSON");
    };
    let Some(mail) = mailbox.mail.iter_mut().find(|mail| mail.id == id) else {
        return no_item();
    };
    for (key, value) in body.as_object().into_iter().flatten() {
        match key.as_str() {
            "isRead" => mail.read = value.as_bool().unwrap_or(false),
            "flag" => {
                mail.flag = match value["flagStatus"].as_str() {
                    Some("flagged") => "Flagged",
                    Some("complete") => "Complete",
                    Some("notFlagged") => "NotFlagged",
                    _ => return failure(400, "BadRequest", "no such flag status"),
                }
                .to_owned();
            }
            "categories" => {
                mail.categories = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect();
            }
            other => return failure(400, "BadRequest", &format!("cannot change {other}")),
        }
    }
    let mail = mail.clone();
    let shown = one(&mail, &selecting_all(request));
    (200, shown)
}

fn relocate(mailbox: &mut Mailbox, id: &str, request: &Logged) -> Answer {
    let Ok(body) = serde_json::from_str::<Value>(&request.body) else {
        return failure(400, "BadRequest", "the body is not JSON");
    };
    let Some(destination) = body["destinationId"]
        .as_str()
        .and_then(|name| folder_id(mailbox, name))
    else {
        return no_folder();
    };
    let Some(mail) = mailbox.mail.iter_mut().find(|mail| mail.id == id) else {
        return no_item();
    };
    mail.folder = destination;
    let mail = mail.clone();
    let shown = one(&mail, &selecting_all(request));
    (201, shown)
}
