//! What a pane's window shows: first a line saying what the add-in is doing, then, once the host is found, the chat in a WebView2
//! control that fills the window.
//!
//! Finding or starting the host can take seconds, so it runs on a thread of its own and posts its answer to the window as a
//! message; everything else, and every call into the control, happens on the thread that made the window (Office's).

#![allow(unsafe_code)]

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, DT_CENTER, DT_WORDBREAK, DrawTextW, EndPaint, FillRect,
    GetSysColorBrush, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetClientRect, PostMessageW, WM_APP, WM_DESTROY, WM_PAINT, WM_SIZE,
};

use crate::context::Context;
use crate::host::{self, Failure, Located};
use crate::log::log;
use crate::relation::chat_url;
use crate::webview::Browser;

/// The message a finished search for the host is posted with: `lparam` is a `Box<Result<Located, Failure>>`.
const WM_LOCATED: u32 = WM_APP + 1;

/// What looking for the host is told: the thing that finds it, replaceable by a test.
type Locator = std::sync::Arc<dyn Fn() -> Result<Located, Failure> + Send + Sync>;

thread_local! {
    /// The state of each pane's window, by window.
    static BY_WINDOW: RefCell<HashMap<isize, Weak<PaneState>>> = RefCell::new(HashMap::new());
    /// What the next control made on this thread is told it is for, and the number the add-in knows it by.
    static NEXT: RefCell<Option<(u64, Context)>> = const { RefCell::new(None) };
    /// Every pane's state, by the number the add-in gave it.
    static BY_ID: RefCell<HashMap<u64, Weak<PaneState>>> = RefCell::new(HashMap::new());
    /// What replaces looking for the host, for the next control (a test's).
    static LOCATOR: RefCell<Option<Locator>> = const { RefCell::new(None) };
}

/// Tells the next pane control made on this thread what it is for. The add-in does this just before it asks Office for a pane.
pub fn set_next_context(id: u64, context: Context) {
    NEXT.with(|next| *next.borrow_mut() = Some((id, context)));
}

/// The state of the pane the add-in numbered `id`, while its control exists.
#[must_use]
pub fn pane_state(id: u64) -> Option<Rc<PaneState>> {
    BY_ID.with(|map| map.borrow().get(&id).and_then(Weak::upgrade))
}

/// Replaces the search for the host, for the next pane control made on this thread.
pub fn set_next_locator(locator: impl Fn() -> Result<Located, Failure> + Send + Sync + 'static) {
    LOCATOR.with(|slot| *slot.borrow_mut() = Some(std::sync::Arc::new(locator)));
}

/// Where the pane's browser keeps its data: the variable `brand::DATA`, else `webview2` in the brand's `office` folder of the user's local
/// application data. It is the pane's own: the app's key, once admitted, lives in it between runs.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(crate::brand::DATA).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join(crate::brand::FOLDER)
        .join("office")
        .join("webview2")
}

/// One pane: what it is for, how far it has got, and its browser.
pub struct PaneState {
    context: RefCell<Context>,
    window: Cell<HWND>,
    status: RefCell<String>,
    located: RefCell<Option<Located>>,
    locating: Cell<bool>,
    locator: Locator,
    browser: Browser,
}

impl core::fmt::Debug for PaneState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PaneState")
            .field("status", &*self.status.borrow())
            .finish_non_exhaustive()
    }
}

impl PaneState {
    /// The state for a control being made now: what the add-in said it is for, else the application with no relation.
    #[must_use]
    pub fn for_next_control() -> Rc<Self> {
        let (id, context) = NEXT
            .with(|next| next.borrow_mut().take())
            .unwrap_or_else(|| {
                let client = crate::context::client_of_exe(&crate::context::current_exe_name());
                (
                    0,
                    Context {
                        client,
                        relation: None,
                    },
                )
            });
        let locator = LOCATOR
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_else(|| std::sync::Arc::new(host::locate));
        let state = Rc::new(Self {
            context: RefCell::new(context),
            window: Cell::new(HWND::default()),
            status: RefCell::new(crate::brand::NAME.to_owned()),
            located: RefCell::new(None),
            locating: Cell::new(false),
            locator,
            browser: Browser::new(),
        });
        if id != 0 {
            BY_ID.with(|map| map.borrow_mut().insert(id, Rc::downgrade(&state)));
        }
        state
    }

    /// What the pane shows while the chat is not there.
    #[must_use]
    pub fn status(&self) -> String {
        self.status.borrow().clone()
    }

    /// The address the chat is at now, once the browser exists.
    #[must_use]
    pub fn chat_address(&self) -> Option<String> {
        self.browser.is_open().then(|| self.browser.url())
    }

    fn set_status(&self, text: &str) {
        *self.status.borrow_mut() = text.to_owned();
        let window = self.window.get();
        if !window.is_invalid() {
            // SAFETY: the live window this pane made.
            unsafe {
                let _ = InvalidateRect(Some(window), None, true);
            }
        }
    }

    /// The pane's window now exists: look for the host, and show the chat when it is found.
    pub fn attach(self: &Rc<Self>, window: HWND) {
        self.window.set(window);
        BY_WINDOW.with(|map| {
            map.borrow_mut()
                .insert(window.0 as isize, Rc::downgrade(self))
        });
        self.search();
    }

    /// The window is going: end the browser and forget the window.
    pub fn detach(&self) {
        self.browser.close();
        let window = self.window.replace(HWND::default());
        if !window.is_invalid() {
            BY_WINDOW.with(|map| map.borrow_mut().remove(&(window.0 as isize)));
        }
    }

    fn search(self: &Rc<Self>) {
        let window = self.window.get();
        if window.is_invalid() || self.locating.replace(true) {
            return;
        }
        self.set_status(&format!("Looking for {}…", crate::brand::NAME));
        let locator = std::sync::Arc::clone(&self.locator);
        let target = window.0 as isize;
        let spawned = std::thread::Builder::new()
            .name("office-locate".to_owned())
            .spawn(move || {
                let result = locator();
                let boxed = Box::into_raw(Box::new(result));
                // SAFETY: a message to a window by handle; it fails when the window is gone, and the box is taken back then.
                let posted = unsafe {
                    PostMessageW(
                        Some(HWND(target as *mut core::ffi::c_void)),
                        WM_LOCATED,
                        WPARAM(0),
                        LPARAM(boxed as isize),
                    )
                };
                if posted.is_err() {
                    // SAFETY: the pointer was made just above and not delivered.
                    drop(unsafe { Box::from_raw(boxed) });
                }
            });
        if let Err(error) = spawned {
            self.locating.set(false);
            self.set_status(&format!(
                "{} could not look for its host ({error}).",
                crate::brand::NAME
            ));
        }
    }

    fn on_located(self: &Rc<Self>, result: Result<Located, Failure>) {
        self.locating.set(false);
        let window = self.window.get();
        if window.is_invalid() {
            return;
        }
        match result {
            Err(failure) => {
                log(&format!("pane: no host: {failure}"));
                self.set_status(&failure.to_string());
            }
            Ok(found) => {
                log(&format!("pane: host at {}", found.web.url));
                *self.located.borrow_mut() = Some(found);
                self.open_browser(window);
            }
        }
    }

    /// The chat's address for what the pane is for now, once the host is known.
    fn address(&self) -> Option<String> {
        let located = self.located.borrow();
        let located = located.as_ref()?;
        let context = self.context.borrow();
        chat_url(
            &located.web.url,
            context.relation.as_ref(),
            context.client,
            located.secret.as_deref(),
        )
        .ok()
    }

    fn open_browser(self: &Rc<Self>, window: HWND) {
        let Some(address) = self.address() else {
            self.set_status(&format!(
                "{}'s web address is not usable.",
                crate::brand::NAME
            ));
            return;
        };
        self.set_status("Opening the chat…");
        let weak = Rc::downgrade(self);
        let failed = move |why: String| {
            log(&format!("pane: the chat could not be shown: {why}"));
            if let Some(state) = weak.upgrade() {
                state.set_status(&format!("The chat could not be shown: {why}."));
            }
        };
        let mut bounds = RECT::default();
        // SAFETY: the live window and a live local.
        unsafe {
            let _ = GetClientRect(window, &mut bounds);
        }
        self.browser
            .open(window, &data_dir(), bounds, &address, failed, || {
                log("pane: the chat was asked for")
            });
    }

    /// The open item may have changed since the pane was last shown: take what it is for now, and show the chat for it. A pane that
    /// found no host asks again.
    pub fn refresh(self: &Rc<Self>, context: Context) {
        *self.context.borrow_mut() = context;
        if self.located.borrow().is_none() {
            self.search();
            return;
        }
        if let Some(address) = self.address()
            && self.browser.is_open()
            && self.browser.url() != address
        {
            log("pane: the open item changed, so the chat follows it");
            self.browser.navigate(&address);
        }
    }

    /// The window changed size: the chat fills it.
    pub fn resize(&self) {
        let window = self.window.get();
        if window.is_invalid() {
            return;
        }
        let mut bounds = RECT::default();
        // SAFETY: the live window and a live local.
        unsafe {
            let _ = GetClientRect(window, &mut bounds);
        }
        self.browser.resize(bounds);
    }
}

fn state_of(window: HWND) -> Option<Rc<PaneState>> {
    BY_WINDOW.with(|map| {
        map.borrow()
            .get(&(window.0 as isize))
            .and_then(Weak::upgrade)
    })
}

/// The pane's window procedure: paints what the pane says, moves the chat with the window, and takes the host search's answer.
///
/// # Safety
///
/// Windows calls it with the window's own handle and message arguments.
pub unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            let text = state_of(window)
                .map_or_else(|| crate::brand::NAME.to_owned(), |state| state.status());
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: `window` is the window being painted and `paint` is a live local `BeginPaint` fills; `EndPaint` pairs it.
            unsafe {
                let dc = BeginPaint(window, &mut paint);
                let mut area = RECT::default();
                let _ = GetClientRect(window, &mut area);
                FillRect(dc, &area, GetSysColorBrush(COLOR_WINDOW));
                let mut wide: Vec<u16> = text.encode_utf16().collect();
                area.left += 16;
                area.right -= 16;
                area.top += 24;
                DrawTextW(dc, &mut wide, &mut area, DT_CENTER | DT_WORDBREAK);
                let _ = EndPaint(window, &paint);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(state) = state_of(window) {
                state.resize();
            }
            // SAFETY: the arguments are the ones Windows passed.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_LOCATED => {
            // SAFETY: `lparam` is the box `search` posted, delivered exactly once.
            let result = *unsafe { Box::from_raw(lparam.0 as *mut Result<Located, Failure>) };
            if let Some(state) = state_of(window) {
                state.on_located(result);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state) = state_of(window) {
                state.detach();
            }
            // SAFETY: the arguments are the ones Windows passed.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        // SAFETY: the arguments are the ones Windows passed.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
