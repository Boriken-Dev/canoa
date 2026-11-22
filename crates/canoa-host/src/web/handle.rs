//! HTTP request routing and response building for the web app server, add-in local endpoints, and launcher page.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "JSON answers and peer addresses are std's"
)]

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::net::IpAddr;

use crate::serve::NotAdmitted;

use super::assets::{
    Body, Lookup, cache_control, content_type, host_allowed, is_app_route, lookup, loopback_host,
    not_modified, segments, validators,
};
use super::wire::{count_headers, header, tools_send};
use super::{
    ADMIT_PATH, HOST_INDEX, ICON_FILE, LAUNCH_PATH, Launcher, Office, Route, Shared, local, office,
};

/// One answer: its status, headers of its own, body, and whether the connection may be kept after it.
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) headers: Vec<(&'static str, String)>,
    pub(super) body: Body,
    pub(super) keep: bool,
}

impl Answer {
    /// An answer of `status` with a line of text saying what it means, after which the connection closes.
    pub(super) fn refusal(status: u16) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "text/plain; charset=utf-8".to_owned())],
            body: Body::Owned(format!("{}\n", reason(status)).into_bytes()),
            keep: false,
        }
    }
}

pub(super) const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Content Too Large",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// The answer to one request, and whether it was `HEAD`, which is answered without a body.
pub(super) fn answer(
    shared: &Shared,
    request: &httparse::Request<'_, '_>,
    from: Option<IpAddr>,
    body: &[u8],
) -> (Answer, bool) {
    let head = request.method == Some("HEAD");
    let asked_path = request
        .path
        .and_then(|target| target.split(['?', '#']).next());
    let admitting = request.method == Some("POST")
        && shared.admission.is_some()
        && asked_path == Some(ADMIT_PATH);
    let launching = request.method == Some("POST")
        && shared.launcher.is_some()
        && asked_path == Some(LAUNCH_PATH);
    let tools_route = shared.tools.is_some()
        && asked_path.is_some_and(|path| {
            path.strip_prefix(local::TOOLS_PATH)
                .is_some_and(|rest| rest.starts_with('/'))
        });
    let posting = admitting || launching || (tools_route && request.method == Some("POST"));
    if !head && !posting && request.method != Some("GET") {
        let mut refused = Answer::refusal(405);
        refused.headers.push(("Allow", "GET, HEAD".to_owned()));
        return (refused, false);
    }
    let body_follows = !(shared.tools.is_some() && tools_send(request))
        && (count_headers(request, "transfer-encoding") > 0
            || count_headers(request, "content-length") > 1
            || header(request, "content-length").is_some_and(|length| length.trim() != "0"));
    // One Host in every request, HTTP/1.0 included, so none goes unchecked.
    if body_follows || count_headers(request, "host") != 1 {
        return (Answer::refusal(400), head);
    }
    if !header(request, "host").is_some_and(|host| host_allowed(host, &shared.hosts)) {
        return (Answer::refusal(421), head);
    }
    let Some(target) = request.path else {
        return (Answer::refusal(400), head);
    };
    if !target.starts_with('/') {
        return (Answer::refusal(400), head);
    }
    if admitting {
        return (admit(shared, request, target, from), false);
    }
    if launching {
        return (launch(shared, request, from), false);
    }
    if tools_route {
        return (tools(shared, request, target, from, body), false);
    }
    if shared.launcher.is_some() {
        return launcher_page(shared, target, head);
    }
    // The add-in's app page is at `/office/app/`, and the app asks for `canoa.json` beside the page it was loaded from.
    let asked = target.split(['?', '#']).next();
    if asked == Some(HOST_INDEX) || (shared.office.is_some() && asked == Some(office::APP_INDEX)) {
        let found = Answer {
            status: 200,
            headers: vec![
                ("Content-Type", "application/json".to_owned()),
                ("Cache-Control", "no-store".to_owned()),
            ],
            body: Body::Owned(
                shared.index[usize::from(!header(request, "host").is_some_and(loopback_host))]
                    .clone(),
            ),
            keep: true,
        };
        return (found, head);
    }
    let Some(segments) = segments(target) else {
        tracing::debug!("web: a path that is not plain was refused");
        return (Answer::refusal(404), head);
    };
    // The add-in's routes, when asked for, are the only ones answered with a policy of their own.
    let office = shared
        .office
        .as_ref()
        .and_then(|office| Office::route(&segments).map(|route| (office, route)));
    if let Some((office, route @ Route::Page(page))) = &office {
        let mut headers = vec![
            ("Content-Type", content_type(page.name()).to_owned()),
            ("Cache-Control", page.cache().to_owned()),
        ];
        headers.extend(office.headers(route));
        let body = Body::Owned(page.body());
        return (
            Answer {
                status: 200,
                headers,
                body,
                keep: true,
            },
            head,
        );
    }
    // The app's page and the icon are the site's own files, under another path.
    let segments = match &office {
        Some((_, Route::App)) => vec!["index.html".to_owned()],
        Some((_, Route::Icon)) => ICON_FILE.iter().map(|part| (*part).to_owned()).collect(),
        _ => segments,
    };
    let own = office
        .as_ref()
        .map(|(office, route)| office.headers(route))
        .unwrap_or_default();
    let found = match lookup(&shared.site, &segments) {
        Lookup::Found(file) => Some((file, segments.last().map(String::as_str))),
        Lookup::Refused => None,
        Lookup::Missing if is_app_route(&segments) => {
            match lookup(&shared.site, &["index.html".to_owned()]) {
                Lookup::Found(file) => Some((file, Some("index.html"))),
                Lookup::Missing | Lookup::Refused => None,
            }
        }
        Lookup::Missing => None,
    };
    let Some((file, name)) = found else {
        return (Answer::refusal(404), head);
    };
    let name = name.unwrap_or("index.html");
    let cache = cache_control(&segments, name, shared.limits.immutable_age, false);
    let mut headers = validators(&file, cache, name);
    headers.extend(own);
    if not_modified(
        header(request, "if-none-match"),
        header(request, "if-modified-since"),
        &file,
    ) {
        headers.retain(|(name, _)| *name != "Content-Type");
        return (
            Answer {
                status: 304,
                headers,
                body: Body::None,
                keep: true,
            },
            head,
        );
    }
    (
        Answer {
            status: 200,
            headers,
            body: file.body,
            keep: true,
        },
        head,
    )
}

/// The answer to `POST /office/admit?peer=<id>`: the page this server serves asks for its own key to be accepted, once it has
/// connected and been turned away.
///
/// It is refused unless the request is from this computer, names this computer by a loopback name, and comes from a page of this
/// very origin (`Origin` is the scheme and `Host` of the request, and `Sec-Fetch-Site`, when the browser sends it, says
/// `same-origin`). A page of another origin cannot send that `Origin`; a program on this computer can, and so can already do
/// whatever the person's own account can.
fn admit(
    shared: &Shared,
    request: &httparse::Request<'_, '_>,
    target: &str,
    from: Option<IpAddr>,
) -> Answer {
    let reply = |status: u16, said: &str| Answer {
        status,
        headers: vec![
            ("Content-Type", "application/json".to_owned()),
            ("Cache-Control", "no-store".to_owned()),
        ],
        body: Body::Owned(
            serde_json::json!({ "admitted": status == 200, "said": said })
                .to_string()
                .into_bytes(),
        ),
        keep: false,
    };
    let Some(admission) = &shared.admission else {
        return reply(403, "not here");
    };
    if let Err((status, said)) = own_page(shared, request, from) {
        return reply(status, said);
    }
    let peer = target
        .split_once('?')
        .map(|(_, query)| query.split('#').next().unwrap_or(query))
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("peer=")))
        .and_then(|value| {
            percent_encoding::percent_decode_str(value)
                .decode_utf8()
                .ok()
        })
        .and_then(|value| value.parse::<coqui_peer_webcrypto::PeerId>().ok());
    let Some(peer) = peer else {
        return reply(400, "no key id");
    };
    match admission.admit(&peer) {
        Ok(()) => reply(200, "accepted"),
        Err(NotAdmitted::NeverConnected) => reply(
            404,
            "this key never connected, so there is nothing to accept",
        ),
        Err(NotAdmitted::NotLocal) => reply(403, "this key did not connect from this computer"),
        Err(NotAdmitted::Store) => reply(500, "the trust store could not be used"),
    }
}

/// Whether the request is from the page this server serves, on this computer: from a loopback address, naming this computer by a
/// loopback name, from a page of this very origin (`Origin` is the scheme and `Host` of the request, and `Sec-Fetch-Site`, when
/// the browser sends it, says `same-origin`) and, when the home keeps a secret for the add-in, carrying it. The refusal is a status
/// and what it says.
fn own_page(
    shared: &Shared,
    request: &httparse::Request<'_, '_>,
    from: Option<IpAddr>,
) -> Result<(), (u16, &'static str)> {
    let Some(host) = header(request, "host").filter(|_| shared.local) else {
        return Err((403, "not here"));
    };
    let scheme = if shared.tls.is_some() {
        "https"
    } else {
        "http"
    };
    let own = format!("{scheme}://{host}");
    let same_origin = header(request, "origin")
        .is_some_and(|origin| origin.eq_ignore_ascii_case(&own))
        && header(request, "sec-fetch-site").is_none_or(|site| site == "same-origin");
    if !loopback_host(host) || !from.is_some_and(|address| address.is_loopback()) || !same_origin {
        return Err((
            403,
            "only the page this host serves, on this computer, may ask",
        ));
    }
    if let Some(secret) = &shared.secret
        && !secret.permits(header(request, local::KEY_HEADER))
    {
        return Err((
            403,
            "this add-in's secret is needed: render its manifest again for this home",
        ));
    }
    Ok(())
}

/// A JSON answer that no cache keeps.
fn json_answer(status: u16, body: &serde_json::Value, keep: bool) -> Answer {
    Answer {
        status,
        headers: vec![
            ("Content-Type", "application/json".to_owned()),
            ("Cache-Control", "no-store".to_owned()),
        ],
        body: Body::Owned(body.to_string().into_bytes()),
        keep,
    }
}

/// The answer to `POST /office/launch` on a server that only offers to start a host.
fn launch(shared: &Shared, request: &httparse::Request<'_, '_>, from: Option<IpAddr>) -> Answer {
    if let Err((status, said)) = own_page(shared, request, from) {
        return json_answer(status, &serde_json::json!({ "said": said }), false);
    }
    let started = shared.launcher.as_ref().is_some_and(Launcher::ask);
    if started {
        json_answer(200, &serde_json::json!({ "starting": true }), false)
    } else {
        json_answer(
            503,
            &serde_json::json!({ "said": "nothing is waiting to start a host" }),
            false,
        )
    }
}

/// The answer to a request under `/office/tools/`: the pane's tools. `open` registers an Office application, `poll` waits for what
/// the host has for it (and, with `after`, names the last message the pane handled), `send` carries one message of the pane's and `close` ends it. Only the page that this server serves, on this
/// computer, may ask.
fn tools(
    shared: &Shared,
    request: &httparse::Request<'_, '_>,
    target: &str,
    from: Option<IpAddr>,
    body: &[u8],
) -> Answer {
    if let Err((status, said)) = own_page(shared, request, from) {
        return json_answer(status, &serde_json::json!({ "said": said }), false);
    }
    let Some(tools) = &shared.tools else {
        return json_answer(403, &serde_json::json!({ "said": "not here" }), false);
    };
    let (path, query) = target
        .split_once('?')
        .map_or((target, ""), |(path, query)| {
            (path, query.split('#').next().unwrap_or(query))
        });
    let value = |name: &str| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_default()
    };
    let session = value("session");
    let action = path
        .strip_prefix(local::TOOLS_PATH)
        .unwrap_or_default()
        .trim_start_matches('/');
    let post = request.method == Some("POST");
    let (status, answer) = match (action, post) {
        ("open", true) => tools.open(value("app")),
        ("poll", false) => match value("after") {
            "" => tools.poll(session, 0),
            after => after.parse().map_or_else(
                |_| {
                    (
                        400,
                        serde_json::json!({ "said": "after is the last number the pane handled" }),
                    )
                },
                |after| tools.poll(session, after),
            ),
        },
        ("send", true) => tools.send(session, body),
        ("close", true) => tools.close(session),
        _ => (
            405,
            serde_json::json!({ "said": "not a request of the tools" }),
        ),
    };
    // A connection of its own for each: a poll that holds messages is never cut by the lifetime of a kept connection.
    json_answer(status, &answer, false)
}

/// What a server that only offers to start a host answers a `GET`: the add-in's page as the offer, its script and the icon, and
/// `/canoa.json` saying where to ask. Anything else is not here.
fn launcher_page(shared: &Shared, target: &str, head: bool) -> (Answer, bool) {
    let asked = target.split(['?', '#']).next();
    if asked == Some(HOST_INDEX) || asked == Some(office::APP_INDEX) {
        let body = serde_json::json!({ "launcher": LAUNCH_PATH });
        return (json_answer(200, &body, true), head);
    }
    let Some(office) = &shared.office else {
        return (Answer::refusal(404), head);
    };
    let Some(segments) = segments(target) else {
        return (Answer::refusal(404), head);
    };
    let route = match Office::route(&segments) {
        Some(Route::Page(page)) if page.name() == "taskpane.html" => Office::launch_page(),
        Some(route @ Route::Page(_)) => route,
        _ => return (Answer::refusal(404), head),
    };
    let Route::Page(page) = &route else {
        return (Answer::refusal(404), head);
    };
    let mut headers = vec![
        ("Content-Type", content_type(page.name()).to_owned()),
        ("Cache-Control", "no-store".to_owned()),
    ];
    headers.extend(office.headers(&route));
    (
        Answer {
            status: 200,
            headers,
            body: Body::Owned(page.body()),
            keep: true,
        },
        head,
    )
}
