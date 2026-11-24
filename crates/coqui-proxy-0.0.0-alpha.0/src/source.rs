//! Where routes come from: any [`ProxySource`] answers for a URL, and a [`ChainSource`] asks
//! several in turn.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use crate::env::Source;
use crate::error::Error;
use crate::proxy::Route;
use crate::resolver::ProxySetting;

/// Something that answers "which proxy for this URL".
///
/// `Ok(None)` is "no opinion": a chain asks the next source. `Ok(Some(routes))` is an answer,
/// `Route::Direct` included.
pub trait ProxySource: Send + Sync + fmt::Debug {
    /// The routes for `url`, or `None` when this source has no opinion.
    ///
    /// # Errors
    ///
    /// Whatever the source fails with: [`Error::Parse`] for a URL that cannot be read, or its own
    /// failure, which stops a chain.
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error>;
}

/// The routes `source` gives for `url`; [`Route::Direct`] alone when it has no opinion.
///
/// # Errors
///
/// As [`ProxySource::routes`].
pub fn lookup(source: &dyn ProxySource, url: &str) -> Result<Vec<Route>, Error> {
    Ok(source
        .routes(url)?
        .unwrap_or_else(|| alloc::vec![Route::Direct]))
}

/// Several sources asked in turn: the first with an opinion answers.
#[derive(Debug, Clone, Default)]
pub struct ChainSource {
    sources: Vec<Arc<dyn ProxySource>>,
}

impl ChainSource {
    /// A chain of `sources`, first asked first.
    pub fn new(sources: impl IntoIterator<Item = Box<dyn ProxySource>>) -> Self {
        Self {
            sources: sources.into_iter().map(Arc::from).collect(),
        }
    }

    /// The sources, in the order they are asked.
    #[must_use]
    pub fn sources(&self) -> &[Arc<dyn ProxySource>] {
        &self.sources
    }
}

impl ProxySource for ChainSource {
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error> {
        for source in &self.sources {
            if let Some(routes) = source.routes(url)? {
                return Ok(Some(routes));
            }
        }
        Ok(None)
    }
}

impl ProxySetting {
    /// The routes a setting that needs no engine answers with: `None` for a PAC location, which
    /// only a resolver or a [`PacSource`](crate::pac::PacSource) can run.
    #[must_use]
    pub fn routes(&self) -> Option<Vec<Route>> {
        match self {
            Self::Proxy(proxy) => Some(alloc::vec![Route::Proxy(proxy.clone())]),
            Self::Routes(routes) => Some(routes.clone()),
            Self::Direct => Some(alloc::vec![Route::Direct]),
            Self::Pac(_) => None,
        }
    }
}

impl ProxySource for ProxySetting {
    fn routes(&self, _url: &str) -> Result<Option<Vec<Route>>, Error> {
        Ok(ProxySetting::routes(self))
    }
}

impl From<crate::proxy::Proxy> for ProxySetting {
    fn from(proxy: crate::proxy::Proxy) -> Self {
        Self::Proxy(proxy)
    }
}

impl FromStr for ProxySetting {
    type Err = Error;

    /// Reads a setting written as a proxy URL (`http://proxy:3128`, `socks5://h`, a gateway
    /// `prefix+https://gw/fetch/`), a PAC result (`PROXY a:1; DIRECT`), a PAC location (a `file:`
    /// URL or an `http(s)` URL with a path) or the text of a PAC script; `direct` or nothing is
    /// no proxy.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] for text that is none of those.
    fn from_str(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("direct") {
            return Ok(Self::Direct);
        }
        match Source::parse(text) {
            Some(Source::PacLocation(location)) => Ok(Self::Pac(location)),
            Some(Source::PacScript(script)) => Ok(Self::Pac(script)),
            Some(Source::Routes(routes)) => Ok(match <[Route; 1]>::try_from(routes) {
                Ok([Route::Proxy(proxy)]) => Self::Proxy(proxy),
                Ok([route]) => Self::Routes(alloc::vec![route]),
                Err(routes) => Self::Routes(routes),
            }),
            None => Err(Error::Parse("proxy setting")),
        }
    }
}
