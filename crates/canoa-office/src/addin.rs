//! The add-in object: Office calls it to connect, to get its ribbon and to hand it the task pane factory.

#![allow(unsafe_code)]

use core::cell::RefCell;
use core::ffi::c_void;
use core::ptr;
use std::io::Write;
use windows::Win32::Foundation::{
    DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME, E_NOTIMPL, E_POINTER, S_OK,
};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IDispatch_Impl, ITypeInfo, SAFEARRAY,
};
use windows::Win32::System::Variant::{VARIANT, VT_ERROR};
use windows::core::{BSTR, GUID, HRESULT, Interface, PCWSTR, Result, implement};

use crate::exports::Live;
use crate::ids::PANE_PROG_ID;
use crate::interfaces::{
    DOCK_RIGHT, ICTPFactory, ICustomTaskPane, ICustomTaskPaneConsumer,
    ICustomTaskPaneConsumer_Impl, IDTExtensibility2, IDTExtensibility2_Impl, IRibbonExtensibility,
    IRibbonExtensibility_Impl,
};
use crate::ribbon::{TOGGLE_CALLBACK, ribbon_xml};

/// The dispatch id of the button's callback.
const TOGGLE_ID: i32 = 1;

/// `DISP_E_PARAMNOTFOUND`: how a COM caller says an optional argument is absent.
const PARAM_NOT_FOUND: i32 = 0x8002_0004_u32 as i32;

/// The Canoa add-in: what Office creates from its CLSID.
#[implement(IDTExtensibility2, IRibbonExtensibility, ICustomTaskPaneConsumer)]
#[derive(Debug)]
pub struct AddIn {
    pane: RefCell<Option<ICustomTaskPane>>,
    _live: Live,
}

impl AddIn {
    /// A new add-in with no pane yet.
    pub fn new() -> Self {
        Self {
            pane: RefCell::new(None),
            _live: Live::new(),
        }
    }

    /// Shows the pane when it is hidden and hides it when it shows.
    fn toggle_pane(&self) -> HRESULT {
        let pane = self.pane.borrow();
        let Some(pane) = pane.as_ref() else {
            return S_OK;
        };
        let mut visible = 0_i16;
        // SAFETY: `visible` is a live local the call writes through.
        let hr = unsafe { pane.get_Visible(&mut visible) };
        if hr.is_err() {
            return hr;
        }
        // SAFETY: a plain value argument.
        unsafe { pane.put_Visible(if visible == 0 { -1 } else { 0 }) }
    }
}

impl Default for AddIn {
    fn default() -> Self {
        Self::new()
    }
}

/// One line of metadata per event, so a person can see whether and when Office loaded the DLL. No content is ever written.
/// `CANOA_OFFICE_LOG` names the file; otherwise it is `office-addin.log` in `%LOCALAPPDATA%\Canoa`.
fn log(event: &str) {
    let path = match std::env::var_os("CANOA_OFFICE_LOG") {
        Some(path) => std::path::PathBuf::from(path),
        None => match std::env::var_os("LOCALAPPDATA") {
            Some(base) => std::path::Path::new(&base)
                .join("Canoa")
                .join("office-addin.log"),
            None => return,
        },
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let _ = writeln!(
        file,
        "{ms} pid={} exe={} arch={} {event}",
        std::process::id(),
        exe.as_deref().unwrap_or("?"),
        std::env::consts::ARCH
    );
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
        unsafe {
            let _ = pane.put_DockPosition(DOCK_RIGHT);
        }
        log("task pane created");
        *self.pane.borrow_mut() = Some(pane);
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
            let known = index == 0
                && unsafe { name.to_string() }
                    .is_ok_and(|n| n.eq_ignore_ascii_case(TOGGLE_CALLBACK));
            // SAFETY: `id` is within the `count` slots the caller provided.
            unsafe { id.write(if known { TOGGLE_ID } else { -1 }) };
            unknown |= !known;
        }
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
        _result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        _arg_error: *mut u32,
    ) -> Result<()> {
        if id != TOGGLE_ID {
            return Err(DISP_E_MEMBERNOTFOUND.into());
        }
        self.toggle_pane().ok()
    }
}
