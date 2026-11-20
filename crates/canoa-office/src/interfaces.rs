//! The interfaces Office defines that the add-in implements or calls, written out because the `windows` crate does not carry
//! them: Office's own type libraries are their only definition. Every one is a dual interface, so each begins with `IDispatch`.
//!
//! Methods follow the type library's order, which is the vtable's. Strings that come in are `*const u16` (a BSTR the caller
//! owns), so no method frees them.

#![allow(unsafe_code, non_snake_case, missing_docs)]

use core::ffi::c_void;
use windows::Win32::System::Com::{IDispatch, IDispatch_Impl, IDispatch_Vtbl, SAFEARRAY};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, HRESULT, interface};

/// The add-in contract every Office application calls (`IDTExtensibility2`).
#[interface("B65AD801-ABAF-11D0-BB8B-00A0C90F2744")]
pub unsafe trait IDTExtensibility2: IDispatch {
    pub fn OnConnection(
        &self,
        application: *mut c_void,
        connect_mode: i32,
        add_in_instance: *mut c_void,
        custom: *mut *mut SAFEARRAY,
    ) -> HRESULT;
    pub fn OnDisconnection(&self, remove_mode: i32, custom: *mut *mut SAFEARRAY) -> HRESULT;
    pub fn OnAddInsUpdate(&self, custom: *mut *mut SAFEARRAY) -> HRESULT;
    pub fn OnStartupComplete(&self, custom: *mut *mut SAFEARRAY) -> HRESULT;
    pub fn OnBeginShutdown(&self, custom: *mut *mut SAFEARRAY) -> HRESULT;
}

/// Asks the add-in for its ribbon's XML, once per ribbon (`IRibbonExtensibility`).
#[interface("000C0396-0000-0000-C000-000000000046")]
pub unsafe trait IRibbonExtensibility: IDispatch {
    pub fn GetCustomUI(&self, ribbon_id: *const u16, ribbon_xml: *mut BSTR) -> HRESULT;
}

/// Hands the add-in the factory that makes task panes (`ICustomTaskPaneConsumer`).
#[interface("000C033E-0000-0000-C000-000000000046")]
pub unsafe trait ICustomTaskPaneConsumer: IDispatch {
    pub fn CTPFactoryAvailable(&self, factory: *mut c_void) -> HRESULT;
}

/// Makes a task pane that hosts the ActiveX control named by a ProgID (`ICTPFactory`).
#[interface("000C033D-0000-0000-C000-000000000046")]
pub unsafe trait ICTPFactory: IDispatch {
    /// `parent` is the window the pane belongs to, or a missing-argument variant for the active one.
    pub fn CreateCTP(
        &self,
        control_prog_id: *const u16,
        title: *const u16,
        parent: VARIANT,
        pane: *mut *mut c_void,
    ) -> HRESULT;
}

/// A task pane (`_CustomTaskPane`). Boolean properties are `VARIANT_BOOL`: 0 or -1.
#[interface("000C033B-0000-0000-C000-000000000046")]
pub unsafe trait ICustomTaskPane: IDispatch {
    pub fn get_Title(&self, title: *mut BSTR) -> HRESULT;
    pub fn get_Application(&self, application: *mut *mut c_void) -> HRESULT;
    pub fn get_Window(&self, window: *mut *mut c_void) -> HRESULT;
    pub fn get_Visible(&self, visible: *mut i16) -> HRESULT;
    pub fn put_Visible(&self, visible: i16) -> HRESULT;
    pub fn get_ContentControl(&self, control: *mut *mut c_void) -> HRESULT;
    pub fn get_Height(&self, height: *mut i32) -> HRESULT;
    pub fn put_Height(&self, height: i32) -> HRESULT;
    pub fn get_Width(&self, width: *mut i32) -> HRESULT;
    pub fn put_Width(&self, width: i32) -> HRESULT;
    pub fn get_DockPosition(&self, position: *mut i32) -> HRESULT;
    pub fn put_DockPosition(&self, position: i32) -> HRESULT;
    pub fn get_DockPositionRestrict(&self, restrict: *mut i32) -> HRESULT;
    pub fn put_DockPositionRestrict(&self, restrict: i32) -> HRESULT;
    pub fn Delete(&self) -> HRESULT;
}

/// `MsoCTPDockPosition`: where a pane docks.
pub const DOCK_RIGHT: i32 = 2;
