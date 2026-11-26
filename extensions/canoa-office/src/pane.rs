//! The control a task pane hosts: an in-place active ActiveX control with one child window.
//!
//! Office's pane host is an OLE container. It creates the control by ProgID, hands it a client site, asks it to activate in
//! place and tells it where to sit. The window the control makes is empty for now; it is where the chat's web view goes.

#![allow(unsafe_code)]

use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::mem::size_of;
use std::rc::Rc;
use std::sync::Once;
use windows::Win32::Foundation::{
    DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME, E_FAIL, E_NOTIMPL, E_POINTER, HINSTANCE, HMODULE,
    HWND, OLE_E_ADVISENOTSUPPORTED, OLEOBJ_E_INVALIDVERB, OLEOBJ_E_NOVERBS, RECT, RECTL, S_FALSE,
    SIZE,
};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush, HDC, LOGPALETTE};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPPARAMS, DVASPECT, DVTARGETDEVICE, EXCEPINFO, IAdviseSink, IConnectionPoint,
    IConnectionPointContainer, IConnectionPointContainer_Impl, IDataObject, IDispatch,
    IDispatch_Impl, IEnumConnectionPoints, IEnumSTATDATA, IMoniker, IPersist_Impl,
    IPersistStreamInit, IPersistStreamInit_Impl, IStream, ITypeInfo,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW,
};
use windows::Win32::System::Ole::{
    CONTROLINFO, IEnumOLEVERB, IOleClientSite, IOleControl, IOleControl_Impl,
    IOleInPlaceActiveObject, IOleInPlaceActiveObject_Impl, IOleInPlaceFrame, IOleInPlaceObject,
    IOleInPlaceObject_Impl, IOleInPlaceSite, IOleInPlaceUIWindow, IOleObject, IOleObject_Impl,
    IOleWindow_Impl, IProvideClassInfo, IProvideClassInfo_Impl, IViewObject_Impl, IViewObject2,
    IViewObject2_Impl, OLECLOSE, OLEGETMONIKER, OLEINPLACEFRAMEINFO, OLEIVERB, OLEIVERB_HIDE,
    OLEIVERB_INPLACEACTIVATE, OLEIVERB_PRIMARY, OLEIVERB_SHOW, OLEIVERB_UIACTIVATE, OLEMISC,
    OLEWHICHMK, USERCLASSTYPE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MSG, MoveWindow, RegisterClassW, SW_HIDE, SW_SHOW, ShowWindow,
    WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows::core::{
    BOOL, GUID, HRESULT, Interface, OutRef, PCWSTR, PWSTR, Ref, Result, implement, w,
};

use crate::exports::Live;
use crate::ids::CLSID_PANE;
use crate::log::log;
use crate::surface::PaneState;

/// `OLEMISC_RECOMPOSEONRESIZE | CANTLINKINSIDE | INSIDEOUT | ACTIVATEWHENVISIBLE | SETCLIENTSITEFIRST`: what an ATL control reports.
const MISC_STATUS: u32 = 0x0002_0191;

/// A size in HIMETRIC units (0.01 mm) before the container says otherwise: about 380 by 760 pixels at 96 dpi.
const DEFAULT_EXTENT: SIZE = SIZE {
    cx: 10_000,
    cy: 20_000,
};

const WINDOW_CLASS: PCWSTR = w!("OfficeChatPane");

/// The pane's control. Office creates it from `PANE_PROG_ID` and drives it through the OLE container interfaces.
#[implement(
    IOleObject,
    IOleInPlaceObject,
    IOleInPlaceActiveObject,
    IViewObject2,
    IPersistStreamInit,
    IOleControl,
    IProvideClassInfo,
    IConnectionPointContainer,
    IDispatch
)]
#[derive(Debug)]
pub struct PaneControl {
    site: RefCell<Option<IOleClientSite>>,
    in_place: RefCell<Option<IOleInPlaceSite>>,
    advise: RefCell<Option<IAdviseSink>>,
    window: Cell<HWND>,
    extent: Cell<SIZE>,
    state: Rc<PaneState>,
    _live: Live,
}

impl PaneControl {
    /// A new control with no site and no window.
    pub fn new() -> Self {
        log("pane control created");
        Self {
            site: RefCell::new(None),
            in_place: RefCell::new(None),
            advise: RefCell::new(None),
            window: Cell::new(HWND::default()),
            extent: Cell::new(DEFAULT_EXTENT),
            state: PaneState::for_next_control(),
            _live: Live::new(),
        }
    }

    /// Activates in place: asks the site's permission, finds the parent window and the room the control gets, and shows its
    /// window there.
    fn activate(
        &self,
        site: &IOleClientSite,
        hint_parent: HWND,
        hint_position: Option<RECT>,
    ) -> Result<()> {
        let mut parent = hint_parent;
        let mut position = hint_position.unwrap_or(RECT {
            left: 0,
            top: 0,
            right: 300,
            bottom: 400,
        });

        // The in-place site, when the container has one: its answers win over the hints DoVerb carried.
        match site.cast::<IOleInPlaceSite>() {
            Err(error) => log(&format!(
                "pane: the client site has no IOleInPlaceSite, hr={:#010x}",
                error.code().0
            )),
            Ok(place) => {
                // SAFETY: calls on a live interface the container gave us, with no pointer arguments.
                let can = unsafe { place.CanInPlaceActivate() };
                step("CanInPlaceActivate", &can);
                // SAFETY: as above.
                let on = unsafe { place.OnInPlaceActivate() };
                step("OnInPlaceActivate", &on);
                if on.is_ok() {
                    *self.in_place.borrow_mut() = Some(place.clone());
                }
                // SAFETY: as above.
                let window = unsafe { place.GetWindow() };
                step("GetWindow", &window);
                if let Ok(window) = window {
                    parent = window;
                }
                let mut frame: Option<IOleInPlaceFrame> = None;
                let mut document: Option<IOleInPlaceUIWindow> = None;
                let mut rect = RECT::default();
                let mut clip = RECT::default();
                let mut info = OLEINPLACEFRAMEINFO {
                    cb: size_of::<OLEINPLACEFRAMEINFO>() as u32,
                    ..Default::default()
                };
                // SAFETY: every argument is a live local the call fills in.
                let context = unsafe {
                    place.GetWindowContext(
                        &mut frame,
                        &mut document,
                        &mut rect,
                        &mut clip,
                        &mut info,
                    )
                };
                step("GetWindowContext", &context);
                if context.is_ok() && rect.right > rect.left && rect.bottom > rect.top {
                    position = rect;
                }
            }
        }

        if parent.is_invalid() {
            log("pane: no parent window from the site or from DoVerb");
            return Err(E_FAIL.into());
        }
        let window = self.window.get();
        if window.is_invalid() {
            let made = create_window(parent, &position);
            step("CreateWindow", &made);
            let made = made?;
            self.window.set(made);
            self.state.attach(made);
        } else {
            // SAFETY: `window` is the live window this control made.
            unsafe {
                let _ = MoveWindow(
                    window,
                    position.left,
                    position.top,
                    position.right - position.left,
                    position.bottom - position.top,
                    true,
                );
                let _ = ShowWindow(window, SW_SHOW);
            }
        }
        Ok(())
    }

    /// What a verb asks for: activate in place, or hide.
    fn do_verb(
        &self,
        verb: i32,
        active_site: Option<IOleClientSite>,
        parent: HWND,
        position: *const RECT,
    ) -> Result<()> {
        match OLEIVERB(verb) {
            OLEIVERB_PRIMARY | OLEIVERB_SHOW | OLEIVERB_UIACTIVATE | OLEIVERB_INPLACEACTIVATE => {
                let site = active_site
                    .or_else(|| self.site.borrow().clone())
                    .ok_or(E_POINTER)?;
                // SAFETY: when non-null, `position` is the container's RECT for the call.
                let hint = if position.is_null() {
                    None
                } else {
                    Some(unsafe { *position })
                };
                self.activate(&site, parent, hint)
            }
            OLEIVERB_HIDE => {
                let window = self.window.get();
                if !window.is_invalid() {
                    // SAFETY: `window` is the live window this control made.
                    unsafe {
                        let _ = ShowWindow(window, SW_HIDE);
                    }
                }
                Ok(())
            }
            _ => Err(OLEOBJ_E_INVALIDVERB.into()),
        }
    }

    /// Destroys the window and tells the site the control is no longer active.
    fn deactivate(&self) {
        self.state.detach();
        let window = self.window.replace(HWND::default());
        if !window.is_invalid() {
            // SAFETY: `window` is the live window this control made, and it is not used again.
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        if let Some(place) = self.in_place.borrow_mut().take() {
            // SAFETY: a call on a live interface with no arguments.
            unsafe {
                let _ = place.OnInPlaceDeactivate();
            }
        }
    }
}

impl Default for PaneControl {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PaneControl {
    fn drop(&mut self) {
        self.deactivate();
    }
}

/// The module this code lives in, which owns the window class.
fn module() -> HINSTANCE {
    static ANCHOR: u8 = 0;
    let mut module = HMODULE::default();
    // SAFETY: the address is a static of this DLL, so the module is this DLL; `UNCHANGED_REFCOUNT` takes no reference to release.
    unsafe {
        let _ = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(&raw const ANCHOR as *const u16),
            &mut module,
        );
    }
    HINSTANCE(module.0)
}

fn create_window(parent: HWND, position: &RECT) -> Result<HWND> {
    static REGISTER: Once = Once::new();
    let instance = module();
    REGISTER.call_once(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(crate::surface::window_proc),
            hInstance: instance,
            lpszClassName: WINDOW_CLASS,
            // SAFETY: a stock system brush, never freed.
            hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
            ..Default::default()
        };
        // SAFETY: `class` is fully initialised and names a window procedure that lives as long as the DLL.
        unsafe {
            RegisterClassW(&class);
        }
    });
    // SAFETY: the class was registered above and `parent` is the container's window.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WINDOW_CLASS,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            position.left,
            position.top,
            position.right - position.left,
            position.bottom - position.top,
            Some(parent),
            None,
            Some(instance),
            None,
        )
    }
}

impl IOleObject_Impl for PaneControl_Impl {
    fn SetClientSite(&self, site: Ref<'_, IOleClientSite>) -> Result<()> {
        log(&format!("pane SetClientSite null={}", site.is_null()));
        *self.site.borrow_mut() = site.as_ref().cloned();
        if site.is_null() {
            self.deactivate();
        }
        Ok(())
    }

    fn GetClientSite(&self) -> Result<IOleClientSite> {
        self.site.borrow().clone().ok_or_else(|| E_POINTER.into())
    }

    fn SetHostNames(&self, _app: &PCWSTR, _object: &PCWSTR) -> Result<()> {
        log("pane SetHostNames");
        Ok(())
    }

    fn Close(&self, _save: &OLECLOSE) -> Result<()> {
        log("pane Close");
        self.deactivate();
        Ok(())
    }

    fn SetMoniker(&self, _which: &OLEWHICHMK, _moniker: Ref<'_, IMoniker>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetMoniker(&self, _assign: &OLEGETMONIKER, _which: &OLEWHICHMK) -> Result<IMoniker> {
        Err(E_NOTIMPL.into())
    }

    fn InitFromData(
        &self,
        _data: Ref<'_, IDataObject>,
        _creation: BOOL,
        _reserved: u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetClipboardData(&self, _reserved: u32) -> Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn DoVerb(
        &self,
        verb: i32,
        _message: *const MSG,
        active_site: Ref<'_, IOleClientSite>,
        _index: i32,
        parent: HWND,
        position: *const RECT,
    ) -> Result<()> {
        log(&format!(
            "pane DoVerb {verb} parent_given={}",
            !parent.is_invalid()
        ));
        let result = self.do_verb(verb, active_site.as_ref().cloned(), parent, position);
        match &result {
            Ok(()) => log("pane DoVerb -> S_OK"),
            Err(error) => log(&format!("pane DoVerb -> hr={:#010x}", error.code().0)),
        }
        result
    }

    fn EnumVerbs(&self) -> Result<IEnumOLEVERB> {
        Err(OLEOBJ_E_NOVERBS.into())
    }

    fn Update(&self) -> Result<()> {
        Ok(())
    }

    fn IsUpToDate(&self) -> Result<()> {
        Ok(())
    }

    fn GetUserClassID(&self) -> Result<GUID> {
        log("pane GetUserClassID");
        Ok(CLSID_PANE)
    }

    fn GetUserType(&self, _form: &USERCLASSTYPE) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn SetExtent(&self, _aspect: DVASPECT, size: *const SIZE) -> Result<()> {
        if size.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `size` is non-null and the caller's SIZE for the call.
        self.extent.set(unsafe { *size });
        Ok(())
    }

    fn GetExtent(&self, _aspect: DVASPECT) -> Result<SIZE> {
        Ok(self.extent.get())
    }

    fn Advise(&self, _sink: Ref<'_, IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn Unadvise(&self, _connection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn GetMiscStatus(&self, _aspect: DVASPECT) -> Result<OLEMISC> {
        log("pane GetMiscStatus");
        Ok(OLEMISC(MISC_STATUS as i32))
    }

    fn SetColorScheme(&self, _palette: *const LOGPALETTE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IOleWindow_Impl for PaneControl_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        let window = self.window.get();
        if window.is_invalid() {
            Err(E_NOTIMPL.into())
        } else {
            Ok(window)
        }
    }

    fn ContextSensitiveHelp(&self, _enter: BOOL) -> Result<()> {
        log("pane ContextSensitiveHelp");
        Ok(())
    }
}

impl IOleInPlaceObject_Impl for PaneControl_Impl {
    fn InPlaceDeactivate(&self) -> Result<()> {
        log("pane InPlaceDeactivate");
        self.deactivate();
        Ok(())
    }

    fn UIDeactivate(&self) -> Result<()> {
        log("pane UIDeactivate");
        Ok(())
    }

    fn SetObjectRects(&self, position: *const RECT, _clip: *const RECT) -> Result<()> {
        let window = self.window.get();
        if position.is_null() {
            return Err(E_POINTER.into());
        }
        if !window.is_invalid() {
            // SAFETY: `position` is non-null and the caller's RECT for the call; `window` is the live window this control made.
            unsafe {
                let rect = *position;
                let _ = MoveWindow(
                    window,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    true,
                );
            }
        }
        Ok(())
    }

    fn ReactivateAndUndo(&self) -> Result<()> {
        Ok(())
    }
}

impl IViewObject_Impl for PaneControl_Impl {
    fn Draw(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut c_void,
        _device: *const DVTARGETDEVICE,
        _target: HDC,
        _draw: HDC,
        _bounds: *const RECTL,
        _window_bounds: *const RECTL,
        _continue: isize,
        _continue_arg: usize,
    ) -> Result<()> {
        // The control draws itself in its window; there is nothing to render into a metafile or a printer.
        Ok(())
    }

    fn GetColorSet(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut c_void,
        _device: *const DVTARGETDEVICE,
        _target: HDC,
        _colors: *mut *mut LOGPALETTE,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Freeze(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _aspect_info: *mut c_void,
        _freeze: *mut u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Unfreeze(&self, _freeze: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetAdvise(&self, _aspects: DVASPECT, _flags: u32, sink: Ref<'_, IAdviseSink>) -> Result<()> {
        log("pane SetAdvise");
        *self.advise.borrow_mut() = sink.as_ref().cloned();
        Ok(())
    }

    fn GetAdvise(
        &self,
        aspects: *mut u32,
        flags: *mut u32,
        sink: OutRef<'_, IAdviseSink>,
    ) -> Result<()> {
        // SAFETY: each out-pointer is either null (skipped) or the caller's writable slot.
        unsafe {
            if !aspects.is_null() {
                aspects.write(0);
            }
            if !flags.is_null() {
                flags.write(0);
            }
        }
        if sink.is_null() {
            Ok(())
        } else {
            sink.write(self.advise.borrow().clone())
        }
    }
}

impl IViewObject2_Impl for PaneControl_Impl {
    fn GetExtent(
        &self,
        _aspect: DVASPECT,
        _index: i32,
        _device: *const DVTARGETDEVICE,
    ) -> Result<SIZE> {
        Ok(self.extent.get())
    }
}

impl IPersist_Impl for PaneControl_Impl {
    fn GetClassID(&self) -> Result<GUID> {
        log("pane GetClassID");
        Ok(CLSID_PANE)
    }
}

impl IPersistStreamInit_Impl for PaneControl_Impl {
    fn IsDirty(&self) -> HRESULT {
        S_FALSE
    }

    fn Load(&self, _stream: Ref<'_, IStream>) -> Result<()> {
        log("pane Load");
        Ok(())
    }

    fn Save(&self, _stream: Ref<'_, IStream>, _clear_dirty: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetSizeMax(&self) -> Result<u64> {
        Ok(0)
    }

    fn InitNew(&self) -> Result<()> {
        log("pane InitNew");
        Ok(())
    }
}

impl IOleControl_Impl for PaneControl_Impl {
    fn GetControlInfo(&self, _info: *mut CONTROLINFO) -> Result<()> {
        log("pane GetControlInfo");
        Err(E_NOTIMPL.into())
    }

    fn OnMnemonic(&self, _message: *const MSG) -> Result<()> {
        Ok(())
    }

    fn OnAmbientPropertyChange(&self, _dispid: i32) -> Result<()> {
        log("pane OnAmbientPropertyChange");
        Ok(())
    }

    fn FreezeEvents(&self, _freeze: BOOL) -> Result<()> {
        Ok(())
    }
}

/// Logs what a container call answered.
fn step<T>(name: &str, result: &Result<T>) {
    match result {
        Ok(_) => log(&format!("pane {name} -> ok")),
        Err(error) => log(&format!("pane {name} -> hr={:#010x}", error.code().0)),
    }
}

/// `CONNECT_E_NOCONNECTION`: no connection point for the interface asked.
const CONNECT_E_NOCONNECTION: HRESULT = HRESULT(0x8004_0200_u32 as i32);

impl IOleInPlaceActiveObject_Impl for PaneControl_Impl {
    fn TranslateAccelerator(&self, _message: *const MSG) -> Result<()> {
        // Not handled: the container's own accelerators apply.
        Err(S_FALSE.into())
    }

    fn OnFrameWindowActivate(&self, _activate: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnDocWindowActivate(&self, _activate: BOOL) -> Result<()> {
        Ok(())
    }

    fn ResizeBorder(
        &self,
        _border: *const RECT,
        _window: Ref<'_, IOleInPlaceUIWindow>,
        _frame: BOOL,
    ) -> Result<()> {
        Ok(())
    }

    fn EnableModeless(&self, _enable: BOOL) -> Result<()> {
        Ok(())
    }
}

impl IProvideClassInfo_Impl for PaneControl_Impl {
    fn GetClassInfo(&self) -> Result<ITypeInfo> {
        log("pane GetClassInfo -> E_NOTIMPL (the control has no type library)");
        Err(E_NOTIMPL.into())
    }
}

impl IConnectionPointContainer_Impl for PaneControl_Impl {
    fn EnumConnectionPoints(&self) -> Result<IEnumConnectionPoints> {
        Err(E_NOTIMPL.into())
    }

    fn FindConnectionPoint(&self, _riid: *const GUID) -> Result<IConnectionPoint> {
        log("pane FindConnectionPoint -> CONNECT_E_NOCONNECTION");
        Err(CONNECT_E_NOCONNECTION.into())
    }
}

/// The control has no automation surface: Office may ask for `IDispatch` for ambient properties and gets nothing.
impl IDispatch_Impl for PaneControl_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _index: u32, _lcid: u32) -> Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }

    fn GetIDsOfNames(
        &self,
        _riid: *const GUID,
        _names: *const PCWSTR,
        _count: u32,
        _lcid: u32,
        _ids: *mut i32,
    ) -> Result<()> {
        Err(DISP_E_UNKNOWNNAME.into())
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
        log(&format!("pane Invoke id={id} -> DISP_E_MEMBERNOTFOUND"));
        Err(DISP_E_MEMBERNOTFOUND.into())
    }
}
