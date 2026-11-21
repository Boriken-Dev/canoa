//! The add-in object: Office calls it to connect, to get its ribbon and to hand it the task pane factory.

#![allow(unsafe_code)]

use core::cell::RefCell;
use core::ffi::c_void;
use core::ptr;
use windows::Win32::Foundation::{
    DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME, E_NOTIMPL, E_POINTER, S_OK,
};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IDispatch_Impl, ITypeInfo, SAFEARRAY,
};
use windows::Win32::System::Variant::{VARIANT, VT_ERROR};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
use windows::core::{BSTR, GUID, HRESULT, HSTRING, Interface, PCWSTR, Result, implement, w};

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

/// Says something to the person in a message box, for a click that cannot do what it was for.
/// A test sets `CANOA_OFFICE_NO_DIALOG` so none opens.
fn tell_person(text: &str) {
    log(&format!("told the person: {text}"));
    if std::env::var_os("CANOA_OFFICE_NO_DIALOG").is_some() {
        return;
    }
    let text = HSTRING::from(text);
    // SAFETY: both strings outlive the call; no owner window, so the box is the desktop's.
    unsafe {
        MessageBoxW(None, &text, w!("Canoa"), MB_OK | MB_ICONWARNING);
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
    pane: RefCell<Option<ICustomTaskPane>>,
    _live: Live,
}

impl AddIn {
    /// A new add-in with no pane yet.
    pub fn new() -> Self {
        Self {
            factory: RefCell::new(None),
            pane: RefCell::new(None),
            _live: Live::new(),
        }
    }

    /// Asks the kept factory for the pane, for the active window.
    fn create_pane(&self) -> HRESULT {
        let Some(factory) = self.factory.borrow().clone() else {
            log("no pane factory yet: Office has not handed it over");
            return S_OK;
        };
        let prog_id = BSTR::from(PANE_PROG_ID);
        let title = BSTR::from("Canoa");
        let mut parent = VARIANT::default();
        // SAFETY: `parent` was just made empty, so its union is a plain variant this block fills in as VT_ERROR.
        unsafe {
            let inner = &mut *parent.Anonymous.Anonymous;
            inner.vt = VT_ERROR;
            inner.Anonymous.scode = PARAM_NOT_FOUND;
        }
        let mut raw: *mut c_void = ptr::null_mut();
        // SAFETY: both strings outlive the call, `parent` is a plain variant and `raw` is a writable slot.
        let hr = unsafe { factory.CreateCTP(prog_id.as_ptr(), title.as_ptr(), parent, &mut raw) };
        if hr.is_err() {
            log(&format!("CreateCTP failed hr={:#010x}", hr.0));
            return hr;
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
        *self.pane.borrow_mut() = Some(pane);
        S_OK
    }

    /// The button: makes the pane on the first click, then shows it when it is hidden and hides it when it shows.
    fn toggle_pane(&self) -> HRESULT {
        log("button clicked");
        if self.pane.borrow().is_none() {
            let hr = self.create_pane();
            if hr.is_err() || self.pane.borrow().is_none() {
                // Nothing to show; the next click asks again. Failing the callback would only raise an error box in Office,
                // so the person is told here instead.
                if hr.is_err() {
                    tell_person(&format!(
                        "Canoa could not open its pane (error {:#010x}). The add-in's log says more.",
                        hr.0
                    ));
                }
                return S_OK;
            }
        }
        let pane = self.pane.borrow();
        let Some(pane) = pane.as_ref() else {
            return S_OK;
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
        hr
    }
}

impl Default for AddIn {
    fn default() -> Self {
        Self::new()
    }
}

impl IDTExtensibility2_Impl for AddIn_Impl {
    unsafe fn OnConnection(
        &self,
        _application: *mut c_void,
        connect_mode: i32,
        _add_in_instance: *mut c_void,
        _custom: *mut *mut SAFEARRAY,
    ) -> HRESULT {
        log(&format!("OnConnection mode={connect_mode}"));
        S_OK
    }

    unsafe fn OnDisconnection(&self, remove_mode: i32, _custom: *mut *mut SAFEARRAY) -> HRESULT {
        log(&format!("OnDisconnection mode={remove_mode}"));
        self.pane.borrow_mut().take();
        self.factory.borrow_mut().take();
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
        self.pane.borrow_mut().take();
        self.factory.borrow_mut().take();
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
        _params: *const DISPPARAMS,
        result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        _arg_error: *mut u32,
    ) -> Result<()> {
        log(&format!("Invoke id={id}"));
        match id {
            TOGGLE_ID => self.toggle_pane().ok(),
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
