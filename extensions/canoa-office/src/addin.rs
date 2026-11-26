//! The add-in object: Office calls it to connect, to get its ribbon and to hand it the task pane factory.

#![allow(unsafe_code)]

use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::ptr;
use windows::Win32::Foundation::{
    DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME, E_FAIL, E_NOTIMPL, E_POINTER, S_OK,
};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IDispatch, IDispatch_Impl, ITypeInfo, SAFEARRAY,
};
use windows::Win32::System::Variant::{VARIANT, VT_ERROR};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
use windows::core::{BSTR, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, Result, implement};

use crate::automation::get_object;
use crate::context::{Context, client_of_exe, current, current_exe_name};
use crate::exports::Live;
use crate::icon::mark_picture;
use crate::ids::PANE_PROG_ID;
use crate::interfaces::{
    DOCK_RIGHT, ICTPFactory, ICustomTaskPane, ICustomTaskPaneConsumer,
    ICustomTaskPaneConsumer_Impl, IDTExtensibility2, IDTExtensibility2_Impl, IRibbonExtensibility,
    IRibbonExtensibility_Impl,
};
use crate::log::log;
use crate::ribbon::{IMAGE_CALLBACK, TOGGLE_CALLBACK, ribbon_xml};
use crate::surface::{pane_state, set_next_context};

/// Says something to the person in a message box, for a click that cannot do what it was for.
/// A test sets the build's no-dialog variable (`brand::NO_DIALOG`) so none opens.
fn tell_person(text: &str) {
    log(&format!("told the person: {text}"));
    if std::env::var_os(crate::brand::NO_DIALOG).is_some() {
        return;
    }
    let text = HSTRING::from(text);
    // SAFETY: both strings outlive the call; no owner window, so the box is the desktop's.
    unsafe {
        MessageBoxW(
            None,
            &text,
            &HSTRING::from(crate::brand::NAME),
            MB_OK | MB_ICONWARNING,
        );
    }
}

/// The width a pane starts with: about 400 pixels at 96 dpi, scaled to the system's density.
pub fn pane_width() -> i32 {
    // SAFETY: a call with no arguments.
    let dpi = unsafe { GetDpiForSystem() };
    (400 * dpi.max(96) / 96) as i32
}

/// The dispatch id of the button's callback.
const TOGGLE_ID: i32 = 1;

/// The dispatch id of the button's picture callback.
const IMAGE_ID: i32 = 2;

/// `DISP_E_PARAMNOTFOUND`: how a COM caller says an optional argument is absent.
const PARAM_NOT_FOUND: i32 = 0x8002_0004_u32 as i32;

/// The Canoa add-in: what Office creates from its CLSID.
#[implement(IDTExtensibility2, IRibbonExtensibility, ICustomTaskPaneConsumer)]
#[derive(Debug)]
pub struct AddIn {
    /// Office's pane factory, kept from `CTPFactoryAvailable`: a pane is made on the first click, when a window exists.
    factory: RefCell<Option<ICTPFactory>>,
    /// The object Office gave `OnConnection`, which the open item is read from.
    application: RefCell<Option<IDispatch>>,
    /// One pane per Office window (a Word window, an Outlook Explorer or message window), made on that window's first click.
    panes: RefCell<Vec<WindowPane>>,
    next_pane: Cell<u64>,
    _live: Live,
}

impl AddIn {
    /// A new add-in with no pane yet.
    pub fn new() -> Self {
        Self {
            factory: RefCell::new(None),
            application: RefCell::new(None),
            panes: RefCell::new(Vec::new()),
            next_pane: Cell::new(1),
            _live: Live::new(),
        }
    }

    /// Asks the kept factory for a pane for `window` (or for the active window with none), numbered `id`.
    fn create_pane(
        &self,
        window: Option<&IDispatch>,
        id: u64,
    ) -> core::result::Result<ICustomTaskPane, HRESULT> {
        let Some(factory) = self.factory.borrow().clone() else {
            log("no pane factory yet: Office has not handed it over");
            return Err(E_FAIL);
        };
        let prog_id = BSTR::from(PANE_PROG_ID);
        let title = BSTR::from(crate::brand::NAME);
        let context = self.context_of(window);
        // The pane's control is made inside `CreateCTP`: it is told what it is for before that.
        let mut raw: *mut c_void = ptr::null_mut();
        let mut hr = E_FAIL;
        if let Some(window) = window {
            set_next_context(id, context.clone());
            // SAFETY: both strings outlive the call, the variant holds a reference to `window`, and `raw` is a writable slot.
            hr = unsafe {
                factory.CreateCTP(
                    prog_id.as_ptr(),
                    title.as_ptr(),
                    VARIANT::from(window.clone()),
                    &mut raw,
                )
            };
            if hr.is_err() {
                log(&format!(
                    "CreateCTP for the window failed hr={:#010x}; asking for the active window",
                    hr.0
                ));
            }
        }
        if window.is_none() || hr.is_err() {
            set_next_context(id, context);
            // SAFETY: both strings outlive the call, the variant is a plain missing argument, and `raw` is a writable slot.
            hr = unsafe {
                factory.CreateCTP(
                    prog_id.as_ptr(),
                    title.as_ptr(),
                    missing_variant(),
                    &mut raw,
                )
            };
        }
        if hr.is_err() {
            log(&format!("CreateCTP failed hr={:#010x}", hr.0));
            return Err(hr);
        }
        // SAFETY: on success `raw` is an owned interface pointer of the pane, which `from_raw` takes over.
        let pane = unsafe { ICustomTaskPane::from_raw(raw) };
        // SAFETY: plain value arguments on a live interface.
        let docked = unsafe { pane.put_DockPosition(DOCK_RIGHT) };
        // The width the pane starts with, once: the pane keeps whatever width the person drags it to afterwards.
        let width = pane_width();
        // SAFETY: a plain value argument on a live interface.
        let sized = unsafe { pane.put_Width(width) };
        log(&format!(
            "task pane created, dock right hr={:#010x}, width {width} px hr={:#010x}",
            docked.0, sized.0
        ));
        Ok(pane)
    }

    /// What the window is showing: the application it belongs to, and the open item.
    fn context_of(&self, window: Option<&IDispatch>) -> Context {
        let client = client_of_exe(&current_exe_name());
        current(client, self.application.borrow().as_ref(), window)
    }

    /// The button: makes the window's pane on its first click, then shows it when it is hidden and hides it when it shows.
    /// `window` is the Word window, or the Outlook Explorer or Inspector, the button was clicked in.
    fn toggle_pane(&self, window: Option<IDispatch>) -> HRESULT {
        let key = window.as_ref().map_or(0, identity);
        log(&format!(
            "button clicked, window {}",
            if key == 0 { "unknown" } else { "known" }
        ));
        let existing = self
            .panes
            .borrow()
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| (entry.id, entry.pane.clone()));
        let (id, pane) = match existing {
            Some(found) => found,
            None => {
                let id = self.next_pane.get();
                self.next_pane.set(id + 1);
                match self.create_pane(window.as_ref(), id) {
                    Ok(pane) => {
                        self.panes.borrow_mut().push(WindowPane {
                            key,
                            id,
                            pane: pane.clone(),
                        });
                        (id, pane)
                    }
                    Err(hr) => {
                        // Nothing to show; the next click asks again. Failing the callback would only raise an error box in
                        // Office, so the person is told here instead.
                        if hr != E_FAIL || self.factory.borrow().is_some() {
                            tell_person(&format!(
                                "{} could not open its pane (error {:#010x}). The add-in's log says more.",
                                crate::brand::NAME,
                                hr.0
                            ));
                        }
                        return S_OK;
                    }
                }
            }
        };
        let mut visible = 0_i16;
        // SAFETY: `visible` is a live local the call writes through.
        let hr = unsafe { pane.get_Visible(&mut visible) };
        if hr.is_err() {
            log(&format!("get_Visible failed hr={:#010x}", hr.0));
            return hr;
        }
        let show = visible == 0;
        // SAFETY: a plain value argument.
        let hr = unsafe { pane.put_Visible(if show { -1 } else { 0 }) };
        log(&format!(
            "pane {} hr={:#010x}",
            if show { "shown" } else { "hidden" },
            hr.0
        ));
        if show {
            // The open item may have changed since the pane was last shown.
            if let Some(state) = pane_state(id) {
                state.refresh(self.context_of(window.as_ref()));
            }
        }
        hr
    }
}

/// A pane and the window it belongs to.
#[derive(Debug)]
struct WindowPane {
    /// The window's COM identity, or 0 when Office did not say which window.
    key: usize,
    id: u64,
    pane: ICustomTaskPane,
}

/// The identity of a COM object: the address of its `IUnknown`, the same through every interface.
fn identity(object: &IDispatch) -> usize {
    object
        .cast::<IUnknown>()
        .map_or(0, |unknown| unknown.as_raw() as usize)
}

/// `DISP_E_PARAMNOTFOUND`, as a variant: how a COM caller says an optional argument is absent.
fn missing_variant() -> VARIANT {
    let mut parent = VARIANT::default();
    // SAFETY: `parent` was just made empty, so its union is a plain variant this block fills in as VT_ERROR.
    unsafe {
        let inner = &mut *parent.Anonymous.Anonymous;
        inner.vt = VT_ERROR;
        inner.Anonymous.scode = PARAM_NOT_FOUND;
    }
    parent
}

/// The window a ribbon callback was called for: the `Context` of the `IRibbonControl` that is its first argument.
fn window_of(params: *const DISPPARAMS) -> Option<IDispatch> {
    if params.is_null() {
        return None;
    }
    // SAFETY: Office passes a DISPPARAMS that is valid for the call, with `cArgs` variants at `rgvarg`.
    let control = unsafe {
        let params = &*params;
        if params.cArgs < 1 || params.rgvarg.is_null() {
            return None;
        }
        IDispatch::try_from(&*params.rgvarg).ok()?
    };
    get_object(&control, "Context").ok()
}

impl Default for AddIn {
    fn default() -> Self {
        Self::new()
    }
}

impl IDTExtensibility2_Impl for AddIn_Impl {
    unsafe fn OnConnection(
        &self,
        application: *mut c_void,
        connect_mode: i32,
        _add_in_instance: *mut c_void,
        _custom: *mut *mut SAFEARRAY,
    ) -> HRESULT {
        log(&format!("OnConnection mode={connect_mode}"));
        // SAFETY: Office passes its Application object, borrowed for the call; the clone takes its own reference.
        *self.application.borrow_mut() =
            unsafe { IDispatch::from_raw_borrowed(&application) }.cloned();
        S_OK
    }

    unsafe fn OnDisconnection(&self, remove_mode: i32, _custom: *mut *mut SAFEARRAY) -> HRESULT {
        log(&format!("OnDisconnection mode={remove_mode}"));
        self.panes.borrow_mut().clear();
        self.factory.borrow_mut().take();
        self.application.borrow_mut().take();
        S_OK
    }

    unsafe fn OnAddInsUpdate(&self, _custom: *mut *mut SAFEARRAY) -> HRESULT {
        S_OK
    }

    unsafe fn OnStartupComplete(&self, _custom: *mut *mut SAFEARRAY) -> HRESULT {
        log("OnStartupComplete");
        S_OK
    }

    unsafe fn OnBeginShutdown(&self, _custom: *mut *mut SAFEARRAY) -> HRESULT {
        log("OnBeginShutdown");
        self.panes.borrow_mut().clear();
        self.factory.borrow_mut().take();
        self.application.borrow_mut().take();
        S_OK
    }
}

impl IRibbonExtensibility_Impl for AddIn_Impl {
    unsafe fn GetCustomUI(&self, ribbon_id: *const u16, out: *mut BSTR) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        let id = if ribbon_id.is_null() {
            String::new()
        } else {
            // SAFETY: Office passes a NUL-terminated BSTR it owns for the length of the call.
            unsafe { PCWSTR(ribbon_id).to_string().unwrap_or_default() }
        };
        log(&format!("GetCustomUI id={id}"));
        let xml = ribbon_xml(&id).map_or_else(BSTR::new, BSTR::from);
        // SAFETY: `out` is non-null and the caller's writable slot; `write` leaves whatever it held undropped.
        unsafe { out.write(xml) };
        S_OK
    }
}

impl ICustomTaskPaneConsumer_Impl for AddIn_Impl {
    unsafe fn CTPFactoryAvailable(&self, factory: *mut c_void) -> HRESULT {
        // SAFETY: Office passes its factory interface pointer, borrowed for the call; `from_raw_borrowed` takes no ownership.
        let Some(factory) = (unsafe { ICTPFactory::from_raw_borrowed(&factory) }) else {
            return E_POINTER;
        };

        log("CTPFactoryAvailable: factory kept, the pane is made on the first click");
        *self.factory.borrow_mut() = Some(factory.clone());
        S_OK
    }
}

/// Office reaches the ribbon button's callback by name through `IDispatch`, so this answers one name: [`TOGGLE_CALLBACK`].
impl IDispatch_Impl for AddIn_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _index: u32, _lcid: u32) -> Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }

    fn GetIDsOfNames(
        &self,
        _riid: *const GUID,
        names: *const PCWSTR,
        count: u32,
        _lcid: u32,
        ids: *mut i32,
    ) -> Result<()> {
        if names.is_null() || ids.is_null() {
            return Err(E_POINTER.into());
        }
        let mut unknown = false;
        for index in 0..count as usize {
            // SAFETY: the caller passes `count` names and room for `count` ids.
            let (name, id) = unsafe { (*names.add(index), ids.add(index)) };
            // SAFETY: each name is a NUL-terminated string the caller owns.
            let dispid = if index == 0 {
                match unsafe { name.to_string() } {
                    Ok(n) if n.eq_ignore_ascii_case(TOGGLE_CALLBACK) => TOGGLE_ID,
                    Ok(n) if n.eq_ignore_ascii_case(IMAGE_CALLBACK) => IMAGE_ID,
                    _ => -1,
                }
            } else {
                -1
            };
            // SAFETY: `id` is within the `count` slots the caller provided.
            unsafe { id.write(dispid) };
            unknown |= dispid < 0;
        }
        log(&format!("GetIDsOfNames count={count} known={}", !unknown));
        if unknown {
            Err(DISP_E_UNKNOWNNAME.into())
        } else {
            Ok(())
        }
    }

    fn Invoke(
        &self,
        id: i32,
        _riid: *const GUID,
        _lcid: u32,
        _flags: DISPATCH_FLAGS,
        params: *const DISPPARAMS,
        result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        _arg_error: *mut u32,
    ) -> Result<()> {
        log(&format!("Invoke id={id}"));
        match id {
            TOGGLE_ID => self.toggle_pane(window_of(params)).ok(),
            IMAGE_ID => {
                let picture = match mark_picture() {
                    Ok(picture) => picture,
                    Err(error) => {
                        log(&format!("GetImage failed hr={:#010x}", error.code().0));
                        return Err(error);
                    }
                };
                log("GetImage: picture returned");
                if result.is_null() {
                    return Err(E_POINTER.into());
                }
                // SAFETY: `result` is non-null and the caller's variant slot; `write` takes the picture's reference over.
                unsafe { result.write(VARIANT::from(picture)) };
                Ok(())
            }
            _ => Err(DISP_E_MEMBERNOTFOUND.into()),
        }
    }
}
