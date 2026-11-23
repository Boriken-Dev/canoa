//! A WebView2 control inside the pane's window, started from the runtime Windows ships (the Evergreen runtime), with no loader
//! library of ours and nothing downloaded.
//!
//! The runtime's own `EmbeddedBrowserWebView.dll`, in the folder the runtime registered, makes the environment; the controller
//! and the page come from COM calls on what it returns. Only the interfaces this uses are declared, with the slots between them
//! left as placeholders, since a vtable is positions. Every call is on the thread that made the window (Office's own), and the
//! results arrive as messages that thread's loop delivers.

#![allow(unsafe_code, non_snake_case, missing_docs)]

use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::ptr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::Win32::Foundation::{HMODULE, HWND, RECT, S_OK};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{
    HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, implement, interface, s,
};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

/// The runtime's registration: its version and the folder that holds the versions.
const RUNTIME_KEY: &str =
    r"Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

#[interface("B96D755E-0319-4E92-A296-23436F46A1FC")]
pub unsafe trait ICoreWebView2Environment: IUnknown {
    fn CreateCoreWebView2Controller(&self, parent: HWND, handler: *mut c_void) -> HRESULT;
}

#[interface("4D00C0D1-9434-4EB6-8078-8697A560334F")]
pub unsafe trait ICoreWebView2Controller: IUnknown {
    fn get_IsVisible(&self, visible: *mut i32) -> HRESULT;
    fn put_IsVisible(&self, visible: i32) -> HRESULT;
    fn get_Bounds(&self, bounds: *mut RECT) -> HRESULT;
    fn put_Bounds(&self, bounds: RECT) -> HRESULT;
    fn get_ZoomFactor(&self, zoom: *mut f64) -> HRESULT;
    fn put_ZoomFactor(&self, zoom: f64) -> HRESULT;
    fn add_ZoomFactorChanged(&self) -> HRESULT;
    fn remove_ZoomFactorChanged(&self) -> HRESULT;
    fn SetBoundsAndZoomFactor(&self) -> HRESULT;
    fn MoveFocus(&self, reason: i32) -> HRESULT;
    fn add_MoveFocusRequested(&self) -> HRESULT;
    fn remove_MoveFocusRequested(&self) -> HRESULT;
    fn add_GotFocus(&self) -> HRESULT;
    fn remove_GotFocus(&self) -> HRESULT;
    fn add_LostFocus(&self) -> HRESULT;
    fn remove_LostFocus(&self) -> HRESULT;
    fn add_AcceleratorKeyPressed(&self) -> HRESULT;
    fn remove_AcceleratorKeyPressed(&self) -> HRESULT;
    fn get_ParentWindow(&self, parent: *mut HWND) -> HRESULT;
    fn put_ParentWindow(&self, parent: HWND) -> HRESULT;
    fn NotifyParentWindowPositionChanged(&self) -> HRESULT;
    fn Close(&self) -> HRESULT;
    fn get_CoreWebView2(&self, view: *mut *mut c_void) -> HRESULT;
}

#[interface("76ECEACB-0462-4D94-AC83-423A6793775E")]
pub unsafe trait ICoreWebView2: IUnknown {
    fn get_Settings(&self, settings: *mut *mut c_void) -> HRESULT;
    fn get_Source(&self, uri: *mut *mut u16) -> HRESULT;
    fn Navigate(&self, uri: *const u16) -> HRESULT;
}

#[interface("4E8A3389-C9D8-4BD2-B6B5-124FEE6CC14D")]
pub unsafe trait IEnvironmentCompleted: IUnknown {
    fn Invoke(&self, result: HRESULT, environment: *mut c_void) -> HRESULT;
}

#[interface("6C4819F3-C9B7-4260-8127-C9F5BDE7F68C")]
pub unsafe trait IControllerCompleted: IUnknown {
    fn Invoke(&self, result: HRESULT, controller: *mut c_void) -> HRESULT;
}

type Done = Box<dyn FnOnce(HRESULT, *mut c_void)>;

#[implement(IEnvironmentCompleted)]
struct EnvironmentDone(RefCell<Option<Done>>);

impl IEnvironmentCompleted_Impl for EnvironmentDone_Impl {
    unsafe fn Invoke(&self, result: HRESULT, environment: *mut c_void) -> HRESULT {
        let done = self.0.borrow_mut().take();
        if let Some(done) = done {
            // No panic may cross into the runtime.
            let _ = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                done(result, environment)
            }));
        }
        S_OK
    }
}

#[implement(IControllerCompleted)]
struct ControllerDone(RefCell<Option<Done>>);

impl IControllerCompleted_Impl for ControllerDone_Impl {
    unsafe fn Invoke(&self, result: HRESULT, controller: *mut c_void) -> HRESULT {
        let done = self.0.borrow_mut().take();
        if let Some(done) = done {
            // No panic may cross into the runtime.
            let _ = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                done(result, controller)
            }));
        }
        S_OK
    }
}

/// The folder of the installed runtime's current version, from its registration: per machine (either registry view), else per user.
#[must_use]
pub fn runtime_dir() -> Option<PathBuf> {
    let machine_32 =
        r"Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
            .to_owned();
    let keys = [
        LOCAL_MACHINE.open(&machine_32),
        LOCAL_MACHINE.open(RUNTIME_KEY),
        CURRENT_USER.open(RUNTIME_KEY),
    ];
    keys.into_iter().flatten().find_map(|key| {
        let version = key.get_string("pv").ok()?;
        let location = key.get_string("location").ok()?;
        let dir = Path::new(&location).join(version);
        dir.is_dir().then_some(dir)
    })
}

/// The runtime's folder for the processor this DLL runs as.
const fn arch_dir() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86"
    }
}

type CreateEnvironment =
    unsafe extern "system" fn(i32, i32, PCWSTR, *mut c_void, *mut c_void) -> HRESULT;

/// Starts making the environment, whose user data lives in `user_data`; `done` is called with the result when the runtime has it.
fn create_environment(user_data: &Path, done: Done) -> Result<(), String> {
    let dir = runtime_dir().ok_or("the WebView2 runtime is not installed")?;
    let dll = dir
        .join("EBWebView")
        .join(arch_dir())
        .join("EmbeddedBrowserWebView.dll");
    // SAFETY: a library of the runtime Windows installed, kept loaded for the life of the process.
    let module: HMODULE = unsafe { LoadLibraryW(&HSTRING::from(dll.as_os_str())) }
        .map_err(|error| format!("the WebView2 runtime did not load ({error})"))?;
    // SAFETY: the export is the runtime's own environment constructor; its signature is the one the runtime's loader calls it with.
    let create: CreateEnvironment = unsafe {
        let found = GetProcAddress(module, s!("CreateWebViewEnvironmentWithOptionsInternal"))
            .ok_or("the WebView2 runtime has no environment entry")?;
        core::mem::transmute::<unsafe extern "system" fn() -> isize, CreateEnvironment>(found)
    };
    std::fs::create_dir_all(user_data)
        .map_err(|error| format!("the pane's data folder could not be made ({error})"))?;
    let handler: IEnvironmentCompleted = EnvironmentDone(RefCell::new(Some(done))).into();
    let folder = HSTRING::from(user_data.as_os_str());
    // SAFETY: valid arguments for the runtime's constructor; it takes its own reference on the handler.
    let hr = unsafe {
        create(
            1,
            0,
            PCWSTR(folder.as_ptr()),
            ptr::null_mut(),
            handler.as_raw(),
        )
    };
    if hr.is_err() {
        return Err(format!(
            "the WebView2 environment could not be started (hr={:#010x})",
            hr.0
        ));
    }
    Ok(())
}

struct Live {
    controller: ICoreWebView2Controller,
    view: ICoreWebView2,
}

struct Inner {
    live: RefCell<Option<Live>>,
    closed: Cell<bool>,
    bounds: Cell<RECT>,
    url: RefCell<String>,
}

/// A WebView2 control and the page it shows, made asynchronously; every method is for the thread that made the window.
#[derive(Clone)]
pub struct Browser {
    inner: Rc<Inner>,
}

impl core::fmt::Debug for Browser {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Browser")
            .field("open", &self.is_open())
            .finish()
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    /// A browser with nothing shown.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                live: RefCell::new(None),
                closed: Cell::new(false),
                bounds: Cell::new(RECT::default()),
                url: RefCell::new(String::new()),
            }),
        }
    }

    /// Whether the control exists.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner.live.borrow().is_some()
    }

    /// Starts the control inside `parent`, filling `bounds`, to show `url`. `failed` is told why when it cannot be made, and
    /// `ready` once the page has been asked for.
    pub fn open(
        &self,
        parent: HWND,
        user_data: &Path,
        bounds: RECT,
        url: &str,
        failed: impl Fn(String) + 'static,
        ready: impl Fn() + 'static,
    ) {
        let inner = Rc::clone(&self.inner);
        inner.closed.set(false);
        inner.bounds.set(bounds);
        *inner.url.borrow_mut() = url.to_owned();
        let failed = Rc::new(failed);
        let ready = Rc::new(ready);
        let on_failed = Rc::clone(&failed);
        let environment_done: Done = Box::new(move |result, environment| {
            if inner.closed.get() {
                return;
            }
            if result.is_err() {
                failed(format!(
                    "the WebView2 environment could not be made (hr={:#010x})",
                    result.0
                ));
                return;
            }
            // SAFETY: the runtime passes its environment, borrowed for this call; `from_raw_borrowed` takes no ownership.
            let Some(environment) =
                (unsafe { ICoreWebView2Environment::from_raw_borrowed(&environment) }).cloned()
            else {
                failed("the WebView2 environment is missing".to_owned());
                return;
            };
            let inner_for_controller = Rc::clone(&inner);
            let failed_here = Rc::clone(&failed);
            let ready_here = Rc::clone(&ready);
            let controller_done: Done = Box::new(move |result, controller| {
                let inner = inner_for_controller;
                // SAFETY: the runtime passes its controller, borrowed for this call.
                let Some(controller) =
                    (unsafe { ICoreWebView2Controller::from_raw_borrowed(&controller) }).cloned()
                else {
                    failed_here(format!(
                        "the WebView2 control could not be made (hr={:#010x})",
                        result.0
                    ));
                    return;
                };
                if inner.closed.get() {
                    // SAFETY: a call on the controller this closure owns.
                    unsafe {
                        let _ = controller.Close();
                    }
                    return;
                }
                let mut raw: *mut c_void = ptr::null_mut();
                // SAFETY: a writable slot; on success it holds an owned `ICoreWebView2` that `from_raw` takes over.
                let got = unsafe { controller.get_CoreWebView2(&mut raw) };
                if got.is_err() || raw.is_null() {
                    failed_here(format!(
                        "the WebView2 page could not be reached (hr={:#010x})",
                        got.0
                    ));
                    return;
                }
                // SAFETY: as above.
                let view = unsafe { ICoreWebView2::from_raw(raw) };
                // SAFETY: plain value arguments on the live controller.
                unsafe {
                    let _ = controller.put_Bounds(inner.bounds.get());
                    let _ = controller.put_IsVisible(1);
                }
                *inner.live.borrow_mut() = Some(Live { controller, view });
                let url = inner.url.borrow().clone();
                Browser {
                    inner: Rc::clone(&inner),
                }
                .navigate(&url);
                ready_here();
            });
            let handler: IControllerCompleted =
                ControllerDone(RefCell::new(Some(controller_done))).into();
            // SAFETY: a live environment and handler; the environment takes its own reference on the handler.
            let hr = unsafe { environment.CreateCoreWebView2Controller(parent, handler.as_raw()) };
            if hr.is_err() {
                failed(format!(
                    "the WebView2 control could not be started (hr={:#010x})",
                    hr.0
                ));
            }
        });
        if let Err(why) = create_environment(user_data, environment_done) {
            on_failed(why);
        }
    }

    /// Shows `url`.
    pub fn navigate(&self, url: &str) {
        *self.inner.url.borrow_mut() = url.to_owned();
        if let Some(live) = self.inner.live.borrow().as_ref() {
            let address = HSTRING::from(url);
            // SAFETY: a live page and a NUL-terminated string that outlives the call.
            unsafe {
                let _ = live.view.Navigate(address.as_ptr());
            }
        }
    }

    /// The address the page is at now, once the control exists and has started to load.
    #[must_use]
    pub fn source(&self) -> Option<String> {
        let live = self.inner.live.borrow();
        let live = live.as_ref()?;
        let mut address: *mut u16 = ptr::null_mut();
        // SAFETY: a live page and a writable slot.
        let hr = unsafe { live.view.get_Source(&mut address) };
        if hr.is_err() || address.is_null() {
            return None;
        }
        // SAFETY: the runtime returns a NUL-terminated string it allocated for the caller, freed once read.
        unsafe {
            let text = PCWSTR(address).to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(address as *const c_void));
            text
        }
    }

    /// The page being shown, or the one it will show once made.
    #[must_use]
    pub fn url(&self) -> String {
        self.inner.url.borrow().clone()
    }

    /// Moves the control to fill `bounds`.
    pub fn resize(&self, bounds: RECT) {
        self.inner.bounds.set(bounds);
        if let Some(live) = self.inner.live.borrow().as_ref() {
            // SAFETY: a plain value argument on the live controller.
            unsafe {
                let _ = live.controller.put_Bounds(bounds);
            }
        }
    }

    /// Shows or hides the control.
    pub fn set_visible(&self, visible: bool) {
        if let Some(live) = self.inner.live.borrow().as_ref() {
            // SAFETY: a plain value argument on the live controller.
            unsafe {
                let _ = live.controller.put_IsVisible(i32::from(visible));
            }
        }
    }

    /// Ends the control. A control still being made is closed as soon as it exists.
    pub fn close(&self) {
        self.inner.closed.set(true);
        if let Some(live) = self.inner.live.borrow_mut().take() {
            // SAFETY: a call on the live controller, which is dropped after.
            unsafe {
                let _ = live.controller.Close();
            }
        }
    }
}
