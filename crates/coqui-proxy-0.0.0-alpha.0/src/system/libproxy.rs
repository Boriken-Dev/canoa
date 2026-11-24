//! libproxy (the library desktops and many programs ask "which proxy for this URL"), loaded at
//! run time so a program links nothing and works where the library is missing.

use alloc::ffi::CString;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::sync::Mutex;

use libloading::Library;

use crate::error::{Error, Reason};
use crate::proxy::{Proxy, Route};
use crate::source::ProxySource;

type NewFactory = unsafe extern "C" fn() -> *mut c_void;
type GetProxies = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut *mut c_char;
type FreeProxies = unsafe extern "C" fn(*mut *mut c_char);
type FreeFactory = unsafe extern "C" fn(*mut c_void);

/// A libproxy proxy factory.
struct Factory(*mut c_void);

// SAFETY: the factory is only used behind a `Mutex`, one call at a time, as libproxy requires of
// a factory shared between threads.
unsafe impl Send for Factory {}

/// libproxy as a [`ProxySource`]: whatever the desktop, the environment and its PAC support say.
pub struct LibProxySource {
    get_proxies: GetProxies,
    free_proxies: FreeProxies,
    free_factory: FreeFactory,
    factory: Mutex<Factory>,
    // Last, so the library outlives the factory it made (`Drop` frees the factory first).
    _library: Library,
}

impl core::fmt::Debug for LibProxySource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LibProxySource")
            .finish_non_exhaustive()
    }
}

fn settings_error(text: impl Into<String>) -> Error {
    Error::Settings(Reason::new(text))
}

impl LibProxySource {
    /// The name the library is looked up by on the linker path.
    pub const DEFAULT_LIBRARY: &'static str = "libproxy.so.1";

    /// Loads libproxy from `path`, or [`LibProxySource::DEFAULT_LIBRARY`] for `None`, and opens a
    /// factory.
    ///
    /// # Errors
    ///
    /// [`Error::Settings`] when the library or one of its functions is missing or the factory
    /// cannot be made.
    pub fn load(path: Option<impl AsRef<Path>>) -> Result<Self, Error> {
        let name = path.as_ref().map_or_else(
            || std::ffi::OsString::from(Self::DEFAULT_LIBRARY),
            |path| path.as_ref().as_os_str().to_os_string(),
        );
        // SAFETY: loading libproxy runs its initialisers, which only set up the library's own state.
        let library = unsafe { Library::new(&name) }
            .map_err(|_| settings_error("libproxy could not be loaded"))?;
        // SAFETY: the symbol names and signatures are those of libproxy's C API (`proxy.h`); the
        // copied pointers stay valid because `_library` is kept for the life of `self`.
        let (new_factory, get_proxies, free_proxies, free_factory) = unsafe {
            let new_factory = *library
                .get::<NewFactory>(b"px_proxy_factory_new\0")
                .map_err(|_| settings_error("libproxy lacks px_proxy_factory_new"))?;
            let get_proxies = *library
                .get::<GetProxies>(b"px_proxy_factory_get_proxies\0")
                .map_err(|_| settings_error("libproxy lacks px_proxy_factory_get_proxies"))?;
            let free_proxies = *library
                .get::<FreeProxies>(b"px_proxy_factory_free_proxies\0")
                .map_err(|_| settings_error("libproxy lacks px_proxy_factory_free_proxies"))?;
            let free_factory = *library
                .get::<FreeFactory>(b"px_proxy_factory_free\0")
                .map_err(|_| settings_error("libproxy lacks px_proxy_factory_free"))?;
            (new_factory, get_proxies, free_proxies, free_factory)
        };
        // SAFETY: `px_proxy_factory_new` takes no argument and returns a factory or null.
        let factory = unsafe { new_factory() };
        if factory.is_null() {
            return Err(settings_error("libproxy could not make a factory"));
        }
        Ok(Self {
            get_proxies,
            free_proxies,
            free_factory,
            factory: Mutex::new(Factory(factory)),
            _library: library,
        })
    }

    fn proxies_for(&self, url: &str) -> Result<Vec<String>, Error> {
        let url = CString::new(url).map_err(|_| Error::Parse("url"))?;
        let factory = self
            .factory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the factory is live and exclusively held; `url` is NUL-terminated.
        let list = unsafe { (self.get_proxies)(factory.0, url.as_ptr()) };
        if list.is_null() {
            return Err(settings_error("libproxy gave no answer"));
        }
        let mut found = Vec::new();
        let mut at = list;
        // SAFETY: libproxy returns a null-terminated array of NUL-terminated strings; each is
        // copied before the array is freed with libproxy's own function.
        unsafe {
            while !(*at).is_null() {
                found.push(CStr::from_ptr(*at).to_string_lossy().into_owned());
                at = at.add(1);
            }
            (self.free_proxies)(list);
        }
        Ok(found)
    }
}

impl Drop for LibProxySource {
    fn drop(&mut self) {
        let factory = self
            .factory
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the factory came from this library's `px_proxy_factory_new` and is freed once.
        unsafe { (self.free_factory)(factory.0) };
    }
}

impl ProxySource for LibProxySource {
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error> {
        let mut routes = Vec::new();
        for entry in self.proxies_for(url)? {
            if entry.starts_with("direct://") {
                routes.push(Route::Direct);
            } else if let Ok(proxy) = Proxy::parse(&entry) {
                routes.push(Route::Proxy(proxy));
            } else {
                tracing::warn!("ignoring an answer of libproxy that is not a proxy");
            }
        }
        if routes.is_empty() {
            routes.push(Route::Direct);
        }
        Ok(Some(routes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_that_is_not_there_is_a_settings_error() {
        let error = LibProxySource::load(Some("/nonexistent/libproxy.so.1")).unwrap_err();
        assert!(matches!(error, Error::Settings(_)), "{error}");
    }
}
