//! A PAC function as a [`ProxySource`]: anything that answers `FindProxyForURL`.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use super::{PacScript, browser_url};
use crate::error::Error;
use crate::proxy::{Route, Target};
use crate::source::ProxySource;

/// Something that answers `FindProxyForURL(url, host)` with a PAC result such as
/// `PROXY a:1; DIRECT`: a [`PacScript`], or any closure of that shape.
pub trait FindProxy: Send + Sync {
    /// The PAC result for `url` whose host is `host`.
    ///
    /// # Errors
    ///
    /// Whatever the function fails with; [`PacSource`] treats it as a failed lookup.
    fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error>;
}

impl FindProxy for PacScript {
    fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error> {
        PacScript::find_proxy_for_url(self, url, host)
    }
}

impl<F> FindProxy for F
where
    F: Fn(&str, &str) -> Result<String, Error> + Send + Sync,
{
    fn find_proxy_for_url(&self, url: &str, host: &str) -> Result<String, Error> {
        self(url, host)
    }
}

/// A [`FindProxy`] as a [`ProxySource`]: the function answers, `PREFIX` and `DIRECT` included.
#[derive(Clone)]
pub struct PacSource {
    script: Arc<dyn FindProxy>,
    browser_compatibility: bool,
}

impl fmt::Debug for PacSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PacSource")
            .field("browser_compatibility", &self.browser_compatibility)
            .finish_non_exhaustive()
    }
}

impl PacSource {
    /// A source over `script`.
    pub fn new(script: impl FindProxy + 'static) -> Self {
        Self {
            script: Arc::new(script),
            browser_compatibility: false,
        }
    }

    /// Whether an `https:` URL is reduced to `https://host/` before the function sees it, as
    /// browsers do (default `false`).
    #[must_use]
    pub fn with_browser_compatibility(mut self, enabled: bool) -> Self {
        self.browser_compatibility = enabled;
        self
    }

    /// Whether `https:` URLs are reduced.
    #[must_use]
    pub fn browser_compatibility(&self) -> bool {
        self.browser_compatibility
    }

    /// The function.
    #[must_use]
    pub fn script(&self) -> &dyn FindProxy {
        &*self.script
    }
}

impl ProxySource for PacSource {
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error> {
        let target = Target::parse(url)?;
        let shown = if self.browser_compatibility {
            browser_url(url)
        } else {
            url.to_string()
        };
        let text = self.script.find_proxy_for_url(&shown, &target.host)?;
        Ok(Some(Route::parse_pac_result(&text)))
    }
}
