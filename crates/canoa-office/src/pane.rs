//! The control a task pane hosts: an in-place active ActiveX control with one child window.
//!
//! Office's pane host is an OLE container. It creates the control by ProgID, hands it a client site, asks it to activate in
//! place and tells it where to sit. The window the control makes is empty for now; it is where the chat's web view goes.

#![allow(unsafe_code)]

use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::mem::size_of;
use std::sync::Once;
use windows::Win32::Foundation::{
    E_NOTIMPL, E_POINTER, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, OLE_E_ADVISENOTSUPPORTED,
    OLEOBJ_E_NOVERBS, RECT, RECTL, S_FALSE, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawTextW, EndPaint, FillRect,
    GetSysColorBrush, HDC, LOGPALETTE, PAINTSTRUCT,
};
use windows::Win32::System::Com::{
    DVASPECT, DVTARGETDEVICE, IAdviseSink, IDataObject, IEnumSTATDATA, IMoniker, IPersist_Impl,
    IPersistStreamInit, IPersistStreamInit_Impl, IStream,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW,
};
use windows::Win32::System::Ole::{
    IEnumOLEVERB, IOleClientSite, IOleInPlaceFrame, IOleInPlaceObject, IOleInPlaceObject_Impl,
    IOleInPlaceSite, IOleInPlaceUIWindow, IOleObject, IOleObject_Impl, IOleWindow_Impl,
    IViewObject_Impl, IViewObject2, IViewObject2_Impl, OLECLOSE, OLEGETMONIKER,
    OLEINPLACEFRAMEINFO, OLEIVERB, OLEIVERB_HIDE, OLEIVERB_INPLACEACTIVATE, OLEIVERB_PRIMARY,
    OLEIVERB_SHOW, OLEIVERB_UIACTIVATE, OLEMISC, OLEWHICHMK, USERCLASSTYPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, MSG, MoveWindow, RegisterClassW,
    SW_HIDE, SW_SHOW, ShowWindow, WINDOW_EX_STYLE, WM_PAINT, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_VISIBLE,
};
use windows::core::{
    BOOL, GUID, HRESULT, Interface, OutRef, PCWSTR, PWSTR, Ref, Result, implement, w,
};

use crate::exports::Live;
use crate::ids::CLSID_PANE;

/// `OLEMISC_RECOMPOSEONRESIZE | CANTLINKINSIDE | INSIDEOUT | ACTIVATEWHENVISIBLE | SETCLIENTSITEFIRST`: what an ATL control reports.
const MISC_STATUS: u32 = 0x0002_0191;

/// A size in HIMETRIC units (0.01 mm) before the container says otherwise: about 380 by 760 pixels at 96 dpi.
const DEFAULT_EXTENT: SIZE = SIZE {
    cx: 10_000,
    cy: 20_000,
};

const WINDOW_CLASS: PCWSTR = w!("CanoaOfficePane");

/// The pane's control. Office creates it from `PANE_PROG_ID` and drives it through the OLE container interfaces.
#[implement(IOleObject, IOleInPlaceObject, IViewObject2, IPersistStreamInit)]
#[derive(Debug)]
pub struct PaneControl {
    site: RefCell<Option<IOleClientSite>>,
    in_place: RefCell<Option<IOleInPlaceSite>>,
    advise: RefCell<Option<IAdviseSink>>,
    window: Cell<HWND>,
    extent: Cell<SIZE>,
    _live: Live,
}

impl PaneControl {
    /// A new control with no site and no window.
    pub fn new() -> Self {
        Self {
            site: RefCell::new(None),
            in_place: RefCell::new(None),
            advise: RefCell::new(None),
            window: Cell::new(HWND::default()),
            extent: Cell::new(DEFAULT_EXTENT),
            _live: Live::new(),
        }
    }

    /// Activates in place: asks the site's permission, finds the parent window and the room the control gets, and shows its
    /// window there.
    fn activate(&self, site: &IOleClientSite) -> Result<()> {
        let place: IOleInPlaceSite = site.cast()?;
        // SAFETY: calls on a live interface the container gave us, with no pointer arguments.
        unsafe {
            place.CanInPlaceActivate()?;
            place.OnInPlaceActivate()?;
        }
        // SAFETY: as above.
        let parent = unsafe { place.GetWindow()? };
        let mut frame: Option<IOleInPlaceFrame> = None;
        let mut document: Option<IOleInPlaceUIWindow> = None;
        let mut position = RECT::default();
        let mut clip = RECT::default();
        let mut info = OLEINPLACEFRAMEINFO {
            cb: size_of::<OLEINPLACEFRAMEINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: every argument is a live local the call fills in.
        unsafe {
            place.GetWindowContext(
                &mut frame,
                &mut document,
                &mut position,
                &mut clip,
                &mut info,
            )?
        };
        *self.in_place.borrow_mut() = Some(place);

        let window = self.window.get();
        if window.is_invalid() {
            self.window.set(create_window(parent, &position)?);
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

    /// Destroys the window and tells the site the control is no longer active.
    fn deactivate(&self) {
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
            lpfnWndProc: Some(window_proc),
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

/// Paints the pane's background and its name.
///
/// # Safety
///
/// Windows calls it with the window's own handle and message arguments.
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: `window` is the window being painted and `paint` is a live local `BeginPaint` fills; `EndPaint` pairs it.
        unsafe {
            let dc: HDC = BeginPaint(window, &mut paint);
            let mut area = RECT::default();
            let _ = GetClientRect(window, &mut area);
            FillRect(dc, &area, GetSysColorBrush(COLOR_WINDOW));
            let mut text: Vec<u16> = "Canoa".encode_utf16().collect();
            DrawTextW(
                dc,
                &mut text,
                &mut area,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            let _ = EndPaint(window, &paint);
        }
        return LRESULT(0);
    }
    // SAFETY: the arguments are the ones Windows passed.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

impl IOleObject_Impl for PaneControl_Impl {
    fn SetClientSite(&self, site: Ref<'_, IOleClientSite>) -> Result<()> {
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
        Ok(())
    }

    fn Close(&self, _save: &OLECLOSE) -> Result<()> {
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
        _parent: HWND,
        _position: *const RECT,
    ) -> Result<()> {
        match OLEIVERB(verb) {
            OLEIVERB_PRIMARY | OLEIVERB_SHOW | OLEIVERB_UIACTIVATE | OLEIVERB_INPLACEACTIVATE => {
                let site = active_site
                    .as_ref()
                    .cloned()
                    .or_else(|| self.site.borrow().clone())
                    .ok_or(E_POINTER)?;
                self.activate(&site)
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
            _ => Err(windows::Win32::Foundation::OLEOBJ_E_INVALIDVERB.into()),
        }
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
        Ok(())
    }
}

impl IOleInPlaceObject_Impl for PaneControl_Impl {
    fn InPlaceDeactivate(&self) -> Result<()> {
        self.deactivate();
        Ok(())
    }

    fn UIDeactivate(&self) -> Result<()> {
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
        Ok(CLSID_PANE)
    }
}

impl IPersistStreamInit_Impl for PaneControl_Impl {
    fn IsDirty(&self) -> HRESULT {
        S_FALSE
    }

    fn Load(&self, _stream: Ref<'_, IStream>) -> Result<()> {
        Ok(())
    }

    fn Save(&self, _stream: Ref<'_, IStream>, _clear_dirty: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetSizeMax(&self) -> Result<u64> {
        Ok(0)
    }

    fn InitNew(&self) -> Result<()> {
        Ok(())
    }
}
