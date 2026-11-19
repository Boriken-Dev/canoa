//! WinHTTP: the current user's Internet Options and WinHTTP's own proxy resolver.
//!
//! The settings are read with `WinHttpGetIEProxyConfigForCurrentUser` rather than from the
//! registry's `DefaultConnectionSettings` blob, whose layout is undocumented and differs
//! between Windows releases. `WinHttpGetProxyForUrl` does the WPAD discovery (DHCP and DNS),
//! fetches the PAC with the user's NTLM or Kerberos credentials when challenged, and runs it, so
//! no JavaScript engine is needed.

use alloc::string::String;
use alloc::vec::Vec;
use core::ptr;

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, TRUE};
use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP, WINHTTP_AUTO_DETECT_TYPE_DNS_A,
    WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS,
    WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
    WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen, WinHttpSetTimeouts,
};

use super::SystemSettings;
use super::wininet::{InternetOptions, routes_from_winhttp_list, settings_from};
use crate::proxy::{Route, Target};

const ERROR_WINHTTP_LOGIN_FAILURE: u32 = 12015;
/// Milliseconds for name resolution, connect, send and receive on the session WinHTTP fetches a
/// PAC file with; its own resolve default is unbounded.
const SESSION_TIMEOUT_MS: i32 = 10_000;

/// Reads a NUL-terminated wide string WinHTTP allocated, then frees it.
///
/// # Safety
///
/// `pointer` is null or a `GlobalAlloc` block holding a NUL-terminated UTF-16 string.
unsafe fn take_wide(pointer: *mut u16) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let mut length = 0;
    // SAFETY: the caller promises a NUL-terminated string, so every offset up to the NUL is readable.
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `length` units were just read from the same allocation.
    let units = unsafe { core::slice::from_raw_parts(pointer, length) };
    let text = String::from_utf16_lossy(units);
    // SAFETY: WinHTTP allocates these strings with GlobalAlloc and hands ownership to the caller.
    unsafe { GlobalFree(pointer.cast()) };
    Some(text)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

fn internet_options() -> Option<InternetOptions> {
    // SAFETY: a zeroed struct of plain integers and pointers is a valid output buffer.
    let mut config: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = unsafe { core::mem::zeroed() };
    // SAFETY: `config` is a valid, writable struct for the call.
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) } == 0 {
        return None;
    }
    // SAFETY: on success the three strings are null or GlobalAlloc blocks owned by the caller.
    let (url, proxy, bypass) = unsafe {
        (
            take_wide(config.lpszAutoConfigUrl),
            take_wide(config.lpszProxy),
            take_wide(config.lpszProxyBypass),
        )
    };
    Some(InternetOptions {
        auto_detect: config.fAutoDetect != 0,
        auto_config_url: url,
        proxy,
        proxy_bypass: bypass,
    })
}

/// The current user's Internet Options.
pub(crate) fn read_settings() -> Option<SystemSettings> {
    internet_options().map(|options| settings_from(&options))
}

/// This computer's fully qualified DNS name, if it has one.
#[cfg(feature = "pac")]
pub(crate) fn computer_fqdn() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsFullyQualified, GetComputerNameExW,
    };

    let mut length = 0u32;
    // SAFETY: a null buffer with a zero length asks for the required size.
    unsafe { GetComputerNameExW(ComputerNameDnsFullyQualified, ptr::null_mut(), &mut length) };
    if length == 0 {
        return None;
    }
    let mut buffer = alloc::vec![0u16; length as usize];
    // SAFETY: `buffer` holds `length` units, as the call was told.
    let ok = unsafe {
        GetComputerNameExW(
            ComputerNameDnsFullyQualified,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    (ok != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

/// A WinHTTP session that answers proxy lookups. WinHTTP caches PAC results per session.
#[derive(Debug)]
pub(crate) struct WinHttpResolver {
    session: ptr_wrapper::Session,
}

mod ptr_wrapper {
    /// A WinHTTP session handle.
    #[derive(Debug)]
    pub(super) struct Session(pub(super) *mut core::ffi::c_void);

    // SAFETY: WinHTTP session handles may be used from any thread; the API serialises its own state.
    unsafe impl Send for Session {}
    // SAFETY: as above, concurrent calls on one session are supported.
    unsafe impl Sync for Session {}
}

impl WinHttpResolver {
    /// Opens a session, or `None` when WinHTTP will not.
    pub(crate) fn new() -> Option<Self> {
        let agent = wide("coqui-syshttp");
        // SAFETY: `agent` is a NUL-terminated wide string that outlives the call; the proxy
        // arguments are null, as the no-proxy access type requires.
        let session = unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            )
        };
        if session.is_null() {
            return None;
        }
        // SAFETY: `session` is the valid handle just opened.
        unsafe {
            WinHttpSetTimeouts(
                session,
                SESSION_TIMEOUT_MS,
                SESSION_TIMEOUT_MS,
                SESSION_TIMEOUT_MS,
                SESSION_TIMEOUT_MS,
            );
        }
        Some(Self {
            session: ptr_wrapper::Session(session),
        })
    }

    /// One `WinHttpGetProxyForUrl` call: the proxy list, or the error code.
    fn query(
        &self,
        url: &str,
        auto_detect: bool,
        config_url: Option<&str>,
        auto_logon: bool,
    ) -> Result<Option<String>, u32> {
        let config_url_wide = config_url.map(wide);
        // SAFETY: the struct is plain data; every field is set below or valid as zero.
        let mut options: WINHTTP_AUTOPROXY_OPTIONS = unsafe { core::mem::zeroed() };
        if auto_detect {
            options.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
            options.dwAutoDetectFlags =
                WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
        if let Some(config_url_wide) = &config_url_wide {
            options.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
            options.lpszAutoConfigUrl = config_url_wide.as_ptr();
        }
        options.fAutoLogonIfChallenged = i32::from(auto_logon);
        // SAFETY: a zeroed WINHTTP_PROXY_INFO is a valid output buffer.
        let mut info: WINHTTP_PROXY_INFO = unsafe { core::mem::zeroed() };
        let url_wide = wide(url);
        // SAFETY: the session is open, and `url_wide`, `options` (and the PAC URL it points to)
        // and `info` all outlive the call.
        let ok = unsafe {
            WinHttpGetProxyForUrl(self.session.0, url_wide.as_ptr(), &mut options, &mut info)
        };
        if ok != TRUE {
            // SAFETY: reads the calling thread's last error, set by the failed call.
            return Err(unsafe { GetLastError() });
        }
        // SAFETY: on success both strings are null or GlobalAlloc blocks owned by the caller.
        let (proxy, _bypass) =
            unsafe { (take_wide(info.lpszProxy), take_wide(info.lpszProxyBypass)) };
        Ok(proxy)
    }

    /// What WinHTTP answers for `url`; `None` when it has no opinion.
    ///
    /// With auto-detection or a PAC URL in Internet Options WinHTTP does the discovery and runs
    /// the script. When those name neither, or WinHTTP fails, the manual proxies answer.
    pub(crate) fn lookup(&self, url: &str, target: &Target) -> Option<Vec<Route>> {
        let options = internet_options()?;
        let auto_detect = options.auto_detect;
        let config_url = options
            .auto_config_url
            .clone()
            .filter(|url| !url.is_empty());
        if auto_detect || config_url.is_some() {
            let mut answer = self.query(url, auto_detect, config_url.as_deref(), false);
            // fAutoLogonIfChallenged only after a real challenge: setting it always turns off
            // WinHTTP's caching.
            if answer == Err(ERROR_WINHTTP_LOGIN_FAILURE) {
                answer = self.query(url, auto_detect, config_url.as_deref(), true);
            }
            match answer {
                Ok(None) => return Some(alloc::vec![Route::Direct]),
                Ok(Some(list)) => {
                    let routes = routes_from_winhttp_list(&target.scheme, &list);
                    return Some(if routes.is_empty() {
                        alloc::vec![Route::Direct]
                    } else {
                        routes
                    });
                }
                Err(code) => tracing::debug!(code, "WinHttpGetProxyForUrl failed"),
            }
        }
        Some(settings_from(&options).manual_routes(target))
    }
}

impl Drop for WinHttpResolver {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `WinHttpOpen` and is closed once, here.
        unsafe { WinHttpCloseHandle(self.session.0) };
    }
}
