//! The two entry points COM calls in a DLL, and the class factory behind them.

#![allow(unsafe_code)]

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};
use std::panic::catch_unwind;
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

use crate::ids::{CLSID_ADDIN, CLSID_PANE};
use crate::{AddIn, PaneControl};

/// Objects alive, and `LockServer` locks held: the DLL may unload only when both are zero.
static LIVE: AtomicI32 = AtomicI32::new(0);
static LOCKS: AtomicI32 = AtomicI32::new(0);

/// Counts one object for as long as the guard lives.
#[derive(Debug)]
pub(crate) struct Live;

impl Live {
    pub(crate) fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Live
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

#[implement(IClassFactory)]
struct Factory {
    make: fn() -> IUnknown,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `object` is non-null and the COM contract makes it a writable pointer slot.
        unsafe { object.write(ptr::null_mut()) };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let made = (self.make)();
        // SAFETY: `riid` is non-null and names an interface id; `object` is the slot written above.
        unsafe { made.query(riid, object).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

fn make_addin() -> IUnknown {
    AddIn::new().into()
}

fn make_pane() -> IUnknown {
    PaneControl::new().into()
}

/// COM's entry for a class: the factory of `rclsid` as `riid`.
///
/// # Safety
///
/// `rclsid` and `riid` point to GUIDs and `ppv` to a writable pointer slot, or any of them is null (answered `E_POINTER`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: `ppv` is non-null and the caller promises it is writable.
    unsafe { ppv.write(ptr::null_mut()) };
    if rclsid.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: `rclsid` is non-null and the caller promises it points to a GUID.
    let clsid = unsafe { *rclsid };
    let make: fn() -> IUnknown = if clsid == CLSID_ADDIN {
        make_addin
    } else if clsid == CLSID_PANE {
        make_pane
    } else {
        return CLASS_E_CLASSNOTAVAILABLE;
    };
    // No panic may cross into Office.
    let result = catch_unwind(|| {
        let factory: IClassFactory = Factory { make }.into();
        // SAFETY: `riid` is non-null and `ppv` is the slot written above.
        unsafe { factory.query(riid, ppv) }
    });
    result.unwrap_or(E_FAIL)
}

/// COM's question whether the DLL may be unloaded: `S_OK` when no object and no lock is alive.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
