//! `fetch`: one request, one reply, through a client made for the call or one the caller keeps.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use ureq::http::Method;
use url::Url;

use super::auth::AuthProvider;
use super::body::Body;
use super::client::{Client, ClientBuilder};
use super::error::Error;
use super::response::Response;
use super::settings::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_READ_TIMEOUT};
use super::transport::Transport;

/// The statuses after which `reauth` runs and the request is sent again.
const REAUTH_STATUSES: [u16; 2] = [401, 403];

/// Starts a [`FetchBuilder`] for `url`.
pub fn fetch(url: impl AsRef<str>) -> FetchBuilder {
    FetchBuilder::new(url)
}

fn form_encode(pairs: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// One request and its reply, with the reply's body read whole (the reply limit bounds it) so the
/// caller takes what it needs from it. A status is never an error: [`Response::error_for_status`]
/// is the caller's choice.
#[must_use]
pub struct FetchBuilder {
    url: String,
    method: Method,
    headers: Vec<(String, Option<String>)>,
    body: Body,
    form: Option<Vec<(String, String)>>,
    json_body: Option<Result<Vec<u8>, String>>,
    timeout: (Option<Duration>, Option<Duration>),
    client: Option<Client>,
    transport: Option<Arc<dyn Transport>>,
    options: Option<ClientBuilder>,
    reauth: Option<Arc<dyn AuthProvider>>,
}

impl core::fmt::Debug for FetchBuilder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FetchBuilder")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

impl FetchBuilder {
    /// A `GET` of `url`.
    pub fn new(url: impl AsRef<str>) -> Self {
        Self {
            url: url.as_ref().to_string(),
            method: Method::GET,
            headers: Vec::new(),
            body: Body::Empty,
            form: None,
            json_body: None,
            timeout: (Some(DEFAULT_CONNECT_TIMEOUT), Some(DEFAULT_READ_TIMEOUT)),
            client: None,
            transport: None,
            options: None,
            reauth: None,
        }
    }

    /// The method (default `GET`).
    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// A header; a `None` value removes the name.
    pub fn header(mut self, name: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        self.headers.push((name.into(), value.map(Into::into)));
        self
    }

    /// The body: bytes, text or a reader.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// A form body, URL-encoded, with `Content-Type: application/x-www-form-urlencoded` unless a
    /// header names one.
    pub fn form<K: Into<String>, V: Into<String>>(
        mut self,
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.form = Some(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        );
        self
    }

    /// A JSON body, with `Content-Type: application/json` unless a header names one. Giving it
    /// beside a body or a form is [`Error::Configuration`].
    #[cfg(feature = "json")]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        self.json_body = Some(serde_json::to_vec(value).map_err(|error| error.to_string()));
        self
    }

    /// The waits: to connect and at each step of the reply (10 s and 60 s).
    pub fn timeout(mut self, connect: Option<Duration>, read: Option<Duration>) -> Self {
        self.timeout = (connect, read);
        self
    }

    /// Sends through a client the caller keeps open. Beside `transport` or `client_options` it is
    /// [`Error::Configuration`]: a given client has them already.
    pub fn client(mut self, client: &Client) -> Self {
        self.client = Some(client.clone());
        self
    }

    /// Sends through a client made for the call over this transport.
    pub fn transport(mut self, transport: impl Transport + 'static) -> Self {
        self.transport = Some(Arc::new(transport));
        self
    }

    /// The settings of the client made for the call.
    pub fn client_options(mut self, options: ClientBuilder) -> Self {
        self.options = Some(options);
        self
    }

    /// Runs `provider` once after a 401 or 403 and sends the request one more time; its errors
    /// are the caller's. The body must be one that can be sent twice.
    pub fn reauth(mut self, provider: Arc<dyn AuthProvider>) -> Self {
        self.reauth = Some(provider);
        self
    }

    /// Sends the request.
    ///
    /// # Errors
    ///
    /// [`Error::Configuration`] for options that contradict, and what the client reports.
    pub fn send(self) -> Result<Response, Error> {
        let contradiction = |name: &str, reason: &str| Error::config(name, reason);
        if self.client.is_some() && (self.transport.is_some() || self.options.is_some()) {
            return Err(contradiction(
                "client",
                "has its transport and settings already: pass them to fetch alone, or set them on the client",
            ));
        }
        let given = [
            !matches!(self.body, Body::Empty),
            self.form.is_some(),
            self.json_body.is_some(),
        ]
        .iter()
        .filter(|given| **given)
        .count();
        if given > 1 {
            return Err(contradiction(
                "json",
                "body, form and json are three bodies: give one",
            ));
        }
        let mut headers = self.headers;
        let has_type = |headers: &[(String, Option<String>)]| {
            headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type") && value.is_some())
        };
        let body = if let Some(json) = self.json_body {
            let bytes = json.map_err(|_| contradiction("json", "cannot be serialized"))?;
            if !has_type(&headers) {
                headers.push(("Content-Type".into(), Some("application/json".into())));
            }
            Body::Bytes(bytes)
        } else if let Some(pairs) = &self.form {
            if !has_type(&headers) {
                headers.push((
                    "Content-Type".into(),
                    Some("application/x-www-form-urlencoded".into()),
                ));
            }
            Body::Text(form_encode(pairs))
        } else {
            self.body
        };
        if self.reauth.is_some() && !body.is_replayable() {
            return Err(contradiction(
                "reauth",
                "sends the request twice: the body must be bytes, text, a form or json",
            ));
        }
        let (client, own) = match self.client {
            Some(client) => (client, false),
            None => {
                let mut builder = self.options.unwrap_or_default();
                if let Some(transport) = self.transport {
                    builder = builder.transport(ArcTransport(transport));
                }
                (builder.build()?, true)
            }
        };
        let result = (|| {
            let send = |body: Body| {
                let mut request = client
                    .request(self.method.clone(), &self.url)
                    .body(body)
                    .timeout(self.timeout.0, self.timeout.1);
                for (name, value) in &headers {
                    request = request.header(name.clone(), value.clone());
                }
                request.send()
            };
            let response = send(body.replay().unwrap_or_default())?;
            if let Some(provider) = &self.reauth
                && REAUTH_STATUSES.contains(&response.status())
            {
                let url = Url::parse(&self.url).map_err(|_| {
                    Error::InvalidRequest("the request URL cannot be parsed".to_string())
                })?;
                response.discard(Some(64 * 1024));
                provider
                    .ensure(&client, &url)
                    .map_err(|source| Error::Hook {
                        key: "REAUTH".to_string(),
                        request: self.url.clone(),
                        source,
                    })?;
                return send(body);
            }
            Ok(response)
        })();
        if own {
            client.close();
        }
        result
    }
}

/// A transport shared by a call.
#[derive(Debug)]
struct ArcTransport(Arc<dyn Transport>);

impl Transport for ArcTransport {
    fn send(
        &self,
        request: super::request::PreparedRequest,
        stream: bool,
    ) -> Result<Response, Error> {
        self.0.send(request, stream)
    }

    fn close(&self) {
        self.0.close();
    }
}
