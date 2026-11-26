//! A client above [`ureq`] that owns redirects, cookies, tokens, hooks, retries and a
//! deadline, with every setting read through prefixed environment variables.
//!
//! It is a port of the Python library `syshttp`: the same settings, the same environment variable
//! names and the same file formats, so a Python and a Rust program in one environment share them.

mod auth;
mod body;
mod client;
pub mod cookies;
mod deadline;
mod decode;
pub mod env;
mod error;
mod fetch;
mod fileio;
mod headers;
pub mod hooks;
mod httpdate;
pub mod jar;
mod redirect;
mod request;
mod response;
mod retry;
mod settings;
mod state;
mod transport;

pub use auth::{Auth, AuthProvider};
pub use body::Body;
pub use client::{Client, ClientBuilder, HookRequest, RequestBuilder};
pub use env::{HttpEnv, HttpEnvBuilder, ProxyOverrides};
pub use error::{BoxError, Error};
pub use fetch::{FetchBuilder, fetch};
pub use headers::{Headers, peer_text};
pub use jar::{CookieJar, TokenJar};
pub use redirect::split_userinfo;
pub use request::{PreparedRequest, Timeouts};
pub use response::{Response, ResponseBuilder, StatusError, TextErrors};
pub use retry::retry_after;
pub use settings::{
    CookieSettings, CookieStore, DEFAULT_CONNECT_TIMEOUT, DEFAULT_READ_TIMEOUT, PoolSettings,
    RetrySettings, SessionSettings, TokenSettings, TokenStore,
};
pub use transport::{Transport, UreqTransport, UreqTransportBuilder};
