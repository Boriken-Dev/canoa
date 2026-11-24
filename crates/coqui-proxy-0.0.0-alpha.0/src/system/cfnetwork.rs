//! macOS CFNetwork as a [`ProxySource`]: `CFNetworkCopyProxiesForURL` decides from System
//! Configuration, and a PAC script it names is run by `CFNetworkExecuteProxyAutoConfigurationURL`
//! on a private run-loop thread that is abandoned after a deadline.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_void};
use core::ptr;
use core::time::Duration;
use std::sync::Mutex;
use std::time::Instant;

use crate::error::{Error, Reason};
use crate::proxy::{Proxy, ProxyScheme, Route};
use crate::settings::ReaderSettings;
use crate::source::ProxySource;

type CFTypeRef = *const c_void;
type CFIndex = isize;

const UTF8: u32 = 0x0800_0100;
const NUMBER_SINT32: CFIndex = 3;

#[repr(C)]
struct CFStreamClientContext {
    version: CFIndex,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    description: *const c_void,
}

type ResultCallback = extern "C" fn(*mut c_void, CFTypeRef, CFTypeRef);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFRelease(value: CFTypeRef);
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFStringCreateWithBytes(
        allocator: CFTypeRef,
        bytes: *const u8,
        length: CFIndex,
        encoding: u32,
        external: u8,
    ) -> CFTypeRef;
    fn CFStringGetCString(
        string: CFTypeRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: u32,
    ) -> u8;
    fn CFURLCreateWithString(allocator: CFTypeRef, string: CFTypeRef, base: CFTypeRef)
    -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: CFIndex, out: *mut c_void) -> u8;
    fn CFRunLoopGetCurrent() -> CFTypeRef;
    fn CFRunLoopAddSource(run_loop: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(run_loop: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source: u8) -> i32;
}

#[link(name = "CFNetwork", kind = "framework")]
unsafe extern "C" {
    static kCFProxyTypeKey: CFTypeRef;
    static kCFProxyHostNameKey: CFTypeRef;
    static kCFProxyPortNumberKey: CFTypeRef;
    static kCFProxyAutoConfigurationURLKey: CFTypeRef;
    static kCFProxyTypeNone: CFTypeRef;
    static kCFProxyTypeHTTP: CFTypeRef;
    static kCFProxyTypeHTTPS: CFTypeRef;
    static kCFProxyTypeSOCKS: CFTypeRef;
    static kCFProxyTypeAutoConfigurationURL: CFTypeRef;
    fn CFNetworkCopySystemProxySettings() -> CFTypeRef;
    fn CFNetworkCopyProxiesForURL(url: CFTypeRef, settings: CFTypeRef) -> CFTypeRef;
    fn CFNetworkExecuteProxyAutoConfigurationURL(
        script_url: CFTypeRef,
        target_url: CFTypeRef,
        callback: ResultCallback,
        context: *mut CFStreamClientContext,
    ) -> CFTypeRef;
}

/// An owned Core Foundation object, released on drop.
struct Owned(CFTypeRef);

impl Owned {
    fn new(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null object this value owns one reference to.
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(text: &str) -> Option<Owned> {
    // SAFETY: the bytes outlive the call, which copies them.
    Owned::new(unsafe {
        CFStringCreateWithBytes(
            ptr::null(),
            text.as_ptr(),
            CFIndex::try_from(text.len()).ok()?,
            UTF8,
            0,
        )
    })
}

fn url_of(text: &str) -> Option<Owned> {
    let string = cf_string(text)?;
    // SAFETY: `string` is a live CFString; a null base is allowed.
    Owned::new(unsafe { CFURLCreateWithString(ptr::null(), string.0, ptr::null()) })
}

fn rust_string(string: CFTypeRef) -> Option<String> {
    if string.is_null() {
        return None;
    }
    let mut buffer = [0 as c_char; 1024];
    // SAFETY: `string` is a CFString and the buffer length is passed.
    let copied = unsafe { CFStringGetCString(string, buffer.as_mut_ptr(), 1024, UTF8) };
    // SAFETY: on success the buffer holds a NUL-terminated string.
    (copied != 0).then(|| {
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

/// What a proxy dictionary of CFNetwork says.
enum Entry {
    Route(Route),
    Pac(String),
}

fn entries(list: CFTypeRef) -> Vec<Entry> {
    let mut found = Vec::new();
    // SAFETY: `list` is a live CFArray of CFDictionary; every key below is a CFNetwork constant,
    // and values are only read while `list` is alive.
    unsafe {
        let count = CFArrayGetCount(list);
        for index in 0..count {
            let entry = CFArrayGetValueAtIndex(list, index);
            let kind = CFDictionaryGetValue(entry, kCFProxyTypeKey);
            if kind.is_null() {
                continue;
            }
            if CFEqual(kind, kCFProxyTypeNone) != 0 {
                found.push(Entry::Route(Route::Direct));
            } else if CFEqual(kind, kCFProxyTypeAutoConfigurationURL) != 0 {
                let url = rust_string(CFDictionaryGetValue(entry, kCFProxyAutoConfigurationURLKey));
                found.extend(url.map(Entry::Pac));
            } else {
                let scheme = if CFEqual(kind, kCFProxyTypeHTTP) != 0 {
                    ProxyScheme::Http
                } else if CFEqual(kind, kCFProxyTypeHTTPS) != 0 {
                    ProxyScheme::Https
                } else if CFEqual(kind, kCFProxyTypeSOCKS) != 0 {
                    ProxyScheme::Socks5
                } else {
                    continue;
                };
                let host = rust_string(CFDictionaryGetValue(entry, kCFProxyHostNameKey));
                let mut port: i32 = 0;
                let number = CFDictionaryGetValue(entry, kCFProxyPortNumberKey);
                if !number.is_null() {
                    CFNumberGetValue(number, NUMBER_SINT32, ptr::from_mut(&mut port).cast());
                }
                let port = u16::try_from(port).unwrap_or(0);
                let proxy = host.and_then(|host| {
                    let port = if port == 0 {
                        scheme.default_port()
                    } else {
                        port
                    };
                    Proxy::builder(scheme, host, port).build().ok()
                });
                found.extend(proxy.map(|proxy| Entry::Route(Route::Proxy(proxy))));
            }
        }
    }
    found
}

/// The answer a PAC run delivers, shared with the callback through the context pointer.
struct Delivery {
    routes: Mutex<Option<Vec<Route>>>,
}

extern "C" fn delivered(info: *mut c_void, list: CFTypeRef, _error: CFTypeRef) {
    // SAFETY: `info` is the `Arc<Delivery>` pointer `run_pac` leaked for this call and keeps
    // alive until the run loop has stopped.
    let delivery = unsafe { &*info.cast::<Delivery>() };
    let routes = if list.is_null() {
        Vec::new()
    } else {
        entries(list)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Route(route) => Some(route),
                Entry::Pac(_) => None,
            })
            .collect()
    };
    *delivery
        .routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(routes);
}

/// Runs the PAC script at `script` for `target` on this thread's run loop, until it answers or
/// `deadline` passes.
fn run_pac(script: &str, target: &str, deadline: Duration) -> Option<Vec<Route>> {
    let script_url = url_of(script)?;
    let target_url = url_of(target)?;
    let delivery = Arc::new(Delivery {
        routes: Mutex::new(None),
    });
    let info = Arc::into_raw(Arc::clone(&delivery))
        .cast_mut()
        .cast::<c_void>();
    let mut context = CFStreamClientContext {
        version: 0,
        info,
        retain: ptr::null(),
        release: ptr::null(),
        description: ptr::null(),
    };
    // SAFETY: both URLs are live; `delivered` matches the callback type; the context is copied by
    // CFNetwork and `info` stays valid until the `Arc::from_raw` below.
    let source = unsafe {
        CFNetworkExecuteProxyAutoConfigurationURL(
            script_url.0,
            target_url.0,
            delivered,
            &raw mut context,
        )
    };
    let Some(source) = Owned::new(source) else {
        // SAFETY: balances `into_raw` above; the callback will never run.
        drop(unsafe { Arc::from_raw(info.cast::<Delivery>()) });
        return None;
    };
    let started = Instant::now();
    // SAFETY: the run loop and the source belong to this thread, and the default mode constant
    // is a static CFString.
    unsafe {
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source.0, kCFRunLoopDefaultMode);
        while started.elapsed() < deadline
            && delivery
                .routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0);
        }
        CFRunLoopRemoveSource(run_loop, source.0, kCFRunLoopDefaultMode);
    }
    // SAFETY: balances `into_raw` above; the source is removed, so the callback cannot run again.
    drop(unsafe { Arc::from_raw(info.cast::<Delivery>()) });
    delivery
        .routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// macOS CFNetwork as a [`ProxySource`].
#[derive(Debug, Clone)]
pub struct CfNetworkSource {
    deadline: Duration,
}

impl CfNetworkSource {
    /// A source with the default deadline ([`ReaderSettings::DEFAULT_CFNETWORK_DEADLINE`]).
    #[must_use]
    pub fn new() -> Self {
        Self {
            deadline: ReaderSettings::DEFAULT_CFNETWORK_DEADLINE,
        }
    }

    /// How long a PAC script may take to answer before the lookup gives up on it.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The deadline of a PAC run.
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.deadline
    }
}

impl Default for CfNetworkSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxySource for CfNetworkSource {
    fn routes(&self, url: &str) -> Result<Option<Vec<Route>>, Error> {
        let target = url_of(url).ok_or(Error::Parse("url"))?;
        // SAFETY: both calls follow the Create rule; the results are wrapped to be released.
        let list = unsafe {
            let settings = Owned::new(CFNetworkCopySystemProxySettings());
            let settings_ref = settings.as_ref().map_or(ptr::null(), |owned| owned.0);
            Owned::new(CFNetworkCopyProxiesForURL(target.0, settings_ref))
        }
        .ok_or_else(|| Error::Settings(Reason::new("CFNetwork gave no answer")))?;
        let mut routes = Vec::new();
        for entry in entries(list.0) {
            match entry {
                Entry::Route(route) => routes.push(route),
                Entry::Pac(script) => {
                    let url = url.to_string();
                    let deadline = self.deadline;
                    // A PAC run needs a run loop of its own, and its callback may never come.
                    let answer = std::thread::scope(|scope| {
                        scope
                            .spawn(|| run_pac(&script, &url, deadline))
                            .join()
                            .ok()
                            .flatten()
                    });
                    routes.extend(answer.into_iter().flatten());
                }
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
    fn a_pac_script_is_run_and_a_zero_deadline_gives_up_on_it() {
        assert_eq!(CfNetworkSource::new().deadline(), Duration::from_secs(5));
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let script = alloc::format!(
            "http://127.0.0.1:{}/proxy.pac",
            listener.local_addr().unwrap().port()
        );
        let body = "function FindProxyForURL(url, host) { return 'PROXY pac.example:3128'; }";
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                let reply = alloc::format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        let answer = run_pac(&script, "http://a.example/", Duration::from_secs(10));
        let Some(routes) = answer else {
            panic!("CFNetwork did not run the script");
        };
        assert!(matches!(&routes[0], Route::Proxy(proxy) if proxy.host() == "pac.example"));
        assert!(run_pac(&script, "http://a.example/", Duration::ZERO).is_none());
    }

    #[test]
    fn the_system_answers_a_lookup() {
        let routes = CfNetworkSource::new()
            .routes("http://example.com/")
            .unwrap()
            .unwrap();
        assert!(!routes.is_empty());
    }
}
