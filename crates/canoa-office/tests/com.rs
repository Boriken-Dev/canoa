//! The add-in and the pane's control through COM, in this process: the class factory, the interfaces Office queries, the ribbon
//! and its callback, the task pane factory Office hands over, and in-place activation against a stand-in container.
//! No Office application is involved.

#![cfg(windows)]
#![allow(unsafe_code, non_snake_case)]

use core::ffi::c_void;
use core::ptr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use canoa_office::interfaces::{
    ICTPFactory, ICTPFactory_Impl, ICustomTaskPane, ICustomTaskPane_Impl, ICustomTaskPaneConsumer,
    IDTExtensibility2, IRibbonExtensibility,
};
use canoa_office::{
    CLSID_ADDIN, CLSID_PANE, DllCanUnloadNow, DllGetClassObject, PANE_PROG_ID, PaneControl,
};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME,
    E_FAIL, E_NOTIMPL, E_POINTER, HWND, RECT, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoRegisterClassObject, CoRevokeClassObject, DISPATCH_FLAGS, DISPATCH_METHOD, DISPPARAMS,
    EXCEPINFO, IClassFactory, IDispatch, IDispatch_Impl, ITypeInfo, REGCLS_MULTIPLEUSE,
};
use windows::Win32::System::Ole::{
    IOleClientSite, IOleClientSite_Impl, IOleContainer, IOleInPlaceFrame, IOleInPlaceObject,
    IOleInPlaceSite, IOleInPlaceSite_Impl, IOleInPlaceUIWindow, IOleObject, IOleWindow_Impl,
    IViewObject2, OLEINPLACEFRAMEINFO, OLEIVERB_HIDE, OLEIVERB_INPLACEACTIVATE,
};
use windows::Win32::System::Variant::{VARIANT, VT_DISPATCH, VT_ERROR};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetParent, IsWindow, IsWindowVisible,
    WINDOW_EX_STYLE, WS_POPUP,
};
use windows::core::{
    BOOL, BSTR, GUID, HRESULT, IUnknown, Interface, OutRef, PCWSTR, Result, implement, w,
};

/// What the stand-ins record, shared with the test body.
#[derive(Default)]
struct Seen {
    created: RefCell<Vec<(String, String, u16)>>,
    /// Office answers E_FAIL while there is no window to put a pane in.
    no_window: Cell<bool>,
    visible: Cell<i16>,
    dock: Cell<i32>,
    width: Cell<i32>,
    site_events: RefCell<Vec<&'static str>>,
}

macro_rules! no_type_info {
    ($name:ty) => {
        impl IDispatch_Impl for $name {
            fn GetTypeInfoCount(&self) -> Result<u32> {
                Ok(0)
            }
            fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> {
                Err(E_NOTIMPL.into())
            }
            fn GetIDsOfNames(
                &self,
                _: *const GUID,
                _: *const PCWSTR,
                _: u32,
                _: u32,
                _: *mut i32,
            ) -> Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn Invoke(
                &self,
                _: i32,
                _: *const GUID,
                _: u32,
                _: DISPATCH_FLAGS,
                _: *const DISPPARAMS,
                _: *mut VARIANT,
                _: *mut EXCEPINFO,
                _: *mut u32,
            ) -> Result<()> {
                Err(E_NOTIMPL.into())
            }
        }
    };
}

#[implement(ICustomTaskPane)]
struct FakePane {
    seen: Rc<Seen>,
}
no_type_info!(FakePane_Impl);

impl ICustomTaskPane_Impl for FakePane_Impl {
    unsafe fn get_Title(&self, _: *mut BSTR) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn get_Application(&self, _: *mut *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn get_Window(&self, _: *mut *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn get_Visible(&self, visible: *mut i16) -> HRESULT {
        unsafe { visible.write(self.seen.visible.get()) };
        S_OK
    }
    unsafe fn put_Visible(&self, visible: i16) -> HRESULT {
        self.seen.visible.set(visible);
        S_OK
    }
    unsafe fn get_ContentControl(&self, _: *mut *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn get_Height(&self, _: *mut i32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn put_Height(&self, _: i32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn get_Width(&self, _: *mut i32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn put_Width(&self, width: i32) -> HRESULT {
        self.seen.width.set(width);
        S_OK
    }
    unsafe fn get_DockPosition(&self, position: *mut i32) -> HRESULT {
        unsafe { position.write(self.seen.dock.get()) };
        S_OK
    }
    unsafe fn put_DockPosition(&self, position: i32) -> HRESULT {
        self.seen.dock.set(position);
        S_OK
    }
    unsafe fn get_DockPositionRestrict(&self, _: *mut i32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn put_DockPositionRestrict(&self, _: i32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn Delete(&self) -> HRESULT {
        S_OK
    }
}

#[implement(ICTPFactory)]
struct FakeFactory {
    seen: Rc<Seen>,
}
no_type_info!(FakeFactory_Impl);

impl ICTPFactory_Impl for FakeFactory_Impl {
    unsafe fn CreateCTP(
        &self,
        prog_id: *const u16,
        title: *const u16,
        parent: VARIANT,
        pane: *mut *mut c_void,
    ) -> HRESULT {
        // SAFETY: the add-in passes NUL-terminated BSTRs; the variant's tag is read from a plain by-value copy.
        let (prog_id, title, vt) = unsafe {
            (
                PCWSTR(prog_id).to_string().unwrap(),
                PCWSTR(title).to_string().unwrap(),
                parent.Anonymous.Anonymous.vt.0,
            )
        };
        self.seen.created.borrow_mut().push((prog_id, title, vt));
        if self.seen.no_window.get() {
            return E_FAIL;
        }
        let made: ICustomTaskPane = FakePane {
            seen: self.seen.clone(),
        }
        .into();
        unsafe { pane.write(made.into_raw()) };
        S_OK
    }
}

/// A container: a client site that lets the control activate in place in a window of the test's own.
#[implement(IOleClientSite, IOleInPlaceSite)]
struct FakeSite {
    parent: HWND,
    seen: Rc<Seen>,
    /// An in-place site that answers neither its window nor its context, as a host that carries both in `DoVerb` does.
    blind: bool,
}

/// A container that is only a client site: no `IOleInPlaceSite` at all.
#[implement(IOleClientSite)]
struct BareSite;

impl IOleClientSite_Impl for BareSite_Impl {
    fn SaveObject(&self) -> Result<()> {
        Ok(())
    }
    fn GetMoniker(
        &self,
        _: &windows::Win32::System::Ole::OLEGETMONIKER,
        _: &windows::Win32::System::Ole::OLEWHICHMK,
    ) -> Result<windows::Win32::System::Com::IMoniker> {
        Err(E_NOTIMPL.into())
    }
    fn GetContainer(&self) -> Result<IOleContainer> {
        Err(E_NOTIMPL.into())
    }
    fn ShowObject(&self) -> Result<()> {
        Ok(())
    }
    fn OnShowWindow(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn RequestNewObjectLayout(&self) -> Result<()> {
        Ok(())
    }
}

impl IOleClientSite_Impl for FakeSite_Impl {
    fn SaveObject(&self) -> Result<()> {
        Ok(())
    }
    fn GetMoniker(
        &self,
        _: &windows::Win32::System::Ole::OLEGETMONIKER,
        _: &windows::Win32::System::Ole::OLEWHICHMK,
    ) -> Result<windows::Win32::System::Com::IMoniker> {
        Err(E_NOTIMPL.into())
    }
    fn GetContainer(&self) -> Result<IOleContainer> {
        Err(E_NOTIMPL.into())
    }
    fn ShowObject(&self) -> Result<()> {
        Ok(())
    }
    fn OnShowWindow(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn RequestNewObjectLayout(&self) -> Result<()> {
        Ok(())
    }
}

impl IOleWindow_Impl for FakeSite_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        if self.blind {
            return Err(E_FAIL.into());
        }
        Ok(self.parent)
    }
    fn ContextSensitiveHelp(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

impl IOleInPlaceSite_Impl for FakeSite_Impl {
    fn CanInPlaceActivate(&self) -> Result<()> {
        Ok(())
    }
    fn OnInPlaceActivate(&self) -> Result<()> {
        self.seen.site_events.borrow_mut().push("activate");
        Ok(())
    }
    fn OnUIActivate(&self) -> Result<()> {
        Ok(())
    }
    fn GetWindowContext(
        &self,
        _: OutRef<'_, IOleInPlaceFrame>,
        _: OutRef<'_, IOleInPlaceUIWindow>,
        position: *mut RECT,
        clip: *mut RECT,
        _: *mut OLEINPLACEFRAMEINFO,
    ) -> Result<()> {
        if self.blind {
            return Err(E_FAIL.into());
        }
        // SAFETY: the control passes live locals.
        unsafe {
            position.write(RECT {
                left: 0,
                top: 0,
                right: 300,
                bottom: 400,
            });
            clip.write(RECT {
                left: 0,
                top: 0,
                right: 300,
                bottom: 400,
            });
        }
        Ok(())
    }
    fn Scroll(&self, _: &windows::Win32::Foundation::SIZE) -> Result<()> {
        Ok(())
    }
    fn OnUIDeactivate(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn OnInPlaceDeactivate(&self) -> Result<()> {
        self.seen.site_events.borrow_mut().push("deactivate");
        Ok(())
    }
    fn DiscardUndoState(&self) -> Result<()> {
        Ok(())
    }
    fn DeactivateAndUndo(&self) -> Result<()> {
        Ok(())
    }
    fn OnPosRectChange(&self, _: *const RECT) -> Result<()> {
        Ok(())
    }
}

/// Office drops a ribbon whose XML does not parse and says nothing: PowerShell's XML parser stands in for Office's.
fn assert_well_formed(xml: &str) {
    assert!(
        !xml.contains("'"),
        "the check quotes the XML in single quotes"
    );
    let shell = ["pwsh", "powershell"]
        .into_iter()
        .find(|name| {
            std::process::Command::new(name)
                .args(["-NoProfile", "-Command", "exit 0"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("the tests need PowerShell");
    let done = std::process::Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("[xml]'{xml}' | Out-Null"),
        ])
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "not well-formed XML: {}",
        String::from_utf8_lossy(&done.stderr)
    );
}

/// The picture is a 32 by 32 bitmap whose pixels keep the mark's transparency: some opaque, some clear.
fn assert_picture_is_the_mark(picture: &IDispatch) {
    use windows::Win32::Graphics::Gdi::{BITMAP, GetObjectW, HBITMAP, HGDIOBJ};
    use windows::Win32::System::Ole::IPicture;
    let picture: IPicture = picture.cast().expect("IPicture");
    // SAFETY: plain property reads on a live picture; the bitmap belongs to it.
    let handle = unsafe { picture.Handle().unwrap() };
    let mut bitmap = BITMAP::default();
    // SAFETY: `bitmap` is a live BITMAP and the size says so.
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(handle.0 as usize as *mut c_void),
            size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut BITMAP as *mut c_void),
        )
    };
    assert_ne!(got, 0);
    assert_eq!(
        (bitmap.bmWidth, bitmap.bmHeight, bitmap.bmBitsPixel),
        (32, 32, 32)
    );
    // SAFETY: a DIB section's bits are `width * height * 4` bytes, alive as long as the picture.
    let pixels = unsafe { core::slice::from_raw_parts(bitmap.bmBits as *const u8, 32 * 32 * 4) };
    let alphas: Vec<u8> = pixels.chunks(4).map(|px| px[3]).collect();
    assert!(alphas.contains(&0), "a transparent pixel was lost");
    assert!(alphas.contains(&255), "no opaque pixel");
    let _ = HBITMAP::default();
}

fn class_factory(clsid: &GUID) -> IClassFactory {
    let mut out: *mut c_void = ptr::null_mut();
    // SAFETY: valid GUIDs and a writable slot.
    let hr = unsafe { DllGetClassObject(clsid, &IClassFactory::IID, &mut out) };
    assert!(hr.is_ok(), "{hr:?}");
    // SAFETY: on success `out` is an owned IClassFactory pointer.
    unsafe { IClassFactory::from_raw(out) }
}

fn size(window: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    // SAFETY: a live window and a live local.
    unsafe { GetClientRect(window, &mut rect).unwrap() };
    (rect.right - rect.left, rect.bottom - rect.top)
}

#[test]
fn entry_points_refuse_what_they_do_not_make() {
    let mut out: *mut c_void = ptr::dangling_mut();
    // SAFETY: valid pointers throughout; the unknown class id is the case under test.
    unsafe {
        let unknown = GUID::from_u128(0x12345678_1234_1234_1234_123456789abc);
        assert_eq!(
            DllGetClassObject(&unknown, &IClassFactory::IID, &mut out),
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert!(out.is_null(), "the out slot is written first");
        assert_eq!(
            DllGetClassObject(&CLSID_ADDIN, &IClassFactory::IID, ptr::null_mut()),
            E_POINTER
        );
        assert_eq!(
            DllGetClassObject(ptr::null(), &IClassFactory::IID, &mut out),
            E_POINTER
        );
        assert_eq!(
            DllGetClassObject(&CLSID_ADDIN, ptr::null(), &mut out),
            E_POINTER
        );
    }
}

#[test]
fn the_add_in_and_the_pane_work_through_com() {
    // SAFETY: the log variable is set before any object exists; this test is the only one in this process that makes objects.
    let log = std::env::temp_dir().join(format!("canoa-office-test-{}.log", std::process::id()));
    unsafe {
        std::env::set_var("CANOA_OFFICE_LOG", &log);
        std::env::set_var("CANOA_OFFICE_NO_DIALOG", "1");
    }
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
    assert_eq!(DllCanUnloadNow(), S_OK, "nothing is alive yet");

    // The add-in through the COM runtime itself: a class object registered in this process, found by CLSID.
    let factory = class_factory(&CLSID_ADDIN);
    // SAFETY: a live factory, registered and revoked on this thread.
    let cookie = unsafe {
        CoRegisterClassObject(
            &CLSID_ADDIN,
            &factory,
            CLSCTX_INPROC_SERVER,
            REGCLS_MULTIPLEUSE,
        )
        .unwrap()
    };
    // SAFETY: as above.
    let extensibility: IDTExtensibility2 =
        unsafe { CoCreateInstance(&CLSID_ADDIN, None, CLSCTX_INPROC_SERVER).unwrap() };
    // SAFETY: the cookie came from the registration above.
    unsafe { CoRevokeClassObject(cookie).unwrap() };
    assert_eq!(DllCanUnloadNow(), S_FALSE, "an object is alive");

    // Office asks the object for the other interfaces.
    let ribbon: IRibbonExtensibility = extensibility.cast().expect("IRibbonExtensibility");
    let consumer: ICustomTaskPaneConsumer = extensibility.cast().expect("ICustomTaskPaneConsumer");
    let dispatch: IDispatch = extensibility.cast().expect("IDispatch");

    // Connecting and starting log metadata only.
    // SAFETY: null for what the add-in ignores.
    unsafe {
        assert!(
            extensibility
                .OnConnection(ptr::null_mut(), 3, ptr::null_mut(), ptr::null_mut())
                .is_ok()
        );
        assert!(extensibility.OnStartupComplete(ptr::null_mut()).is_ok());
    }
    let lines = std::fs::read_to_string(&log).unwrap();
    assert!(
        lines.contains("OnConnection mode=3") && lines.contains("OnStartupComplete"),
        "{lines}"
    );
    assert!(
        lines.contains(&format!("pid={}", std::process::id()))
            && lines.contains(&format!("arch={}", std::env::consts::ARCH)),
        "{lines}"
    );

    // The ribbon: XML for the ribbons it adds to, empty for any other.
    let ask = |id: &str| -> String {
        let id = BSTR::from(id);
        let mut xml = BSTR::new();
        // SAFETY: a BSTR in, a writable BSTR slot out.
        unsafe { assert!(ribbon.GetCustomUI(id.as_ptr(), &mut xml).is_ok()) };
        xml.to_string()
    };
    for id in [
        "Microsoft.Word.Document",
        "Microsoft.Excel.Workbook",
        "Microsoft.PowerPoint.Presentation",
        "Microsoft.Outlook.Explorer",
        "Microsoft.Outlook.Mail.Read",
        "Microsoft.Outlook.Mail.Compose",
    ] {
        let xml = ask(id);
        assert!(
            xml.contains(r#"onAction="OnTogglePane""#) && xml.starts_with("<customUI "),
            "{id}: {xml}"
        );
        assert_eq!(xml, canoa_office::ribbon_xml(id).unwrap());
        assert_well_formed(&xml);
    }
    assert_eq!(ask("Microsoft.Outlook.Appointment"), "");

    // The callback is found by name, case-insensitively; another name is refused.
    let names = [w!("onTogglePane")];
    let mut id = 0_i32;
    // SAFETY: one name in, one id out.
    unsafe {
        dispatch
            .GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0, &mut id)
            .unwrap()
    };
    assert_eq!(id, 1);
    let other = [w!("Nothing")];
    let mut other_id = 0_i32;
    // SAFETY: as above.
    let refused =
        unsafe { dispatch.GetIDsOfNames(&GUID::zeroed(), other.as_ptr(), 1, 0, &mut other_id) };
    assert_eq!(refused.unwrap_err().code(), DISP_E_UNKNOWNNAME);
    assert_eq!(other_id, -1);
    let invoke = |dispid: i32| -> Result<()> {
        // SAFETY: empty parameters and no result slots are what a callback with no arguments is called with.
        unsafe {
            dispatch.Invoke(
                dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_METHOD,
                &DISPPARAMS::default(),
                None,
                None,
                None,
            )
        }
    };
    assert!(invoke(1).is_ok(), "no pane yet: the click does nothing");
    assert_eq!(invoke(77).unwrap_err().code(), DISP_E_MEMBERNOTFOUND);

    // Office hands over its pane factory: the add-in asks for the control's ProgID, docked right, hidden until the button.
    let seen = Rc::new(Seen::default());
    let office_factory: ICTPFactory = FakeFactory { seen: seen.clone() }.into();
    // SAFETY: a live interface pointer, borrowed for the call.
    unsafe {
        assert!(
            consumer
                .CTPFactoryAvailable(office_factory.as_raw())
                .is_ok()
        )
    };
    // Word hands the factory over before any window exists: nothing is made yet.
    assert!(
        seen.created.borrow().is_empty(),
        "no pane before the first click"
    );

    // A click while Office has no window: it fails, the log and the person are told, and the next click asks again.
    seen.no_window.set(true);
    invoke(1).unwrap();
    assert_eq!(seen.created.borrow().len(), 1);
    assert_eq!(seen.visible.get(), 0);
    let lines = std::fs::read_to_string(&log).unwrap();
    assert!(lines.contains("button clicked"), "{lines}");
    assert!(lines.contains("CreateCTP failed hr=0x80004005"), "{lines}");
    assert!(
        lines.contains("told the person: Canoa could not open its pane (error 0x80004005)"),
        "{lines}"
    );

    // With a window the same click makes the pane for it, docked right, and shows it; the next click hides it.
    seen.no_window.set(false);
    invoke(1).unwrap();
    assert_eq!(
        seen.created.borrow()[1],
        (PANE_PROG_ID.to_owned(), "Canoa".to_owned(), VT_ERROR.0)
    );
    assert_eq!(seen.created.borrow().len(), 2);
    assert_eq!(seen.dock.get(), 2, "docked right");
    assert_eq!(seen.width.get(), canoa_office::pane_width());
    assert!(seen.width.get() >= 400, "a sensible starting width");
    assert_eq!(seen.visible.get(), -1, "the first click shows the pane");
    seen.width.set(555); // the person drags the pane
    invoke(1).unwrap();
    assert_eq!(seen.visible.get(), 0, "the second hides it");
    assert_eq!(seen.width.get(), 555, "their width is kept");
    invoke(1).unwrap();
    assert_eq!(seen.visible.get(), -1, "and the third shows it again");
    assert_eq!(seen.created.borrow().len(), 2, "one pane, made once");
    let lines = std::fs::read_to_string(&log).unwrap();
    assert!(
        lines.contains("pane shown") && lines.contains("pane hidden"),
        "{lines}"
    );

    // The button's picture: Office asks for it by name and gets the mark as a picture in the call's result.
    let image = [w!("GetImage")];
    let mut image_id = 0_i32;
    // SAFETY: one name in, one id out.
    unsafe {
        dispatch
            .GetIDsOfNames(&GUID::zeroed(), image.as_ptr(), 1, 0, &mut image_id)
            .unwrap()
    };
    assert_eq!(image_id, 2);
    let mut result = VARIANT::default();
    // SAFETY: empty parameters and a result slot, as Office calls a getImage callback.
    unsafe {
        dispatch
            .Invoke(
                image_id,
                &GUID::zeroed(),
                0,
                DISPATCH_METHOD,
                &DISPPARAMS::default(),
                Some(&mut result),
                None,
                None,
            )
            .unwrap()
    };
    // SAFETY: the variant was filled by the call above.
    assert_eq!(unsafe { result.Anonymous.Anonymous.vt }, VT_DISPATCH);
    assert_picture_is_the_mark(&IDispatch::try_from(&result).unwrap());

    // A class that is not the add-in's own is refused aggregation.
    // SAFETY: no outer unknown, the ordinary call.
    let outer: IUnknown = unsafe { factory.CreateInstance::<_, IUnknown>(None).unwrap() };
    // SAFETY: a valid outer unknown, so the factory must answer that aggregation is not supported.
    let aggregated = unsafe { factory.CreateInstance::<_, IUnknown>(&outer) };
    assert_eq!(aggregated.unwrap_err().code(), CLASS_E_NOAGGREGATION);
    drop(outer);

    // The pane's control: the interfaces a container queries.
    // SAFETY: no outer unknown, the ordinary call.
    let control: IOleObject = unsafe {
        class_factory(&CLSID_PANE)
            .CreateInstance::<_, IOleObject>(None)
            .unwrap()
    };
    let in_place: IOleInPlaceObject = control.cast().expect("IOleInPlaceObject");
    let _view: IViewObject2 = control.cast().expect("IViewObject2");
    let _persist: windows::Win32::System::Com::IPersistStreamInit =
        control.cast().expect("IPersistStreamInit");
    // SAFETY: plain calls on a live interface.
    unsafe {
        assert_eq!(control.GetUserClassID().unwrap(), CLSID_PANE);
        assert_eq!(
            control
                .GetMiscStatus(windows::Win32::System::Com::DVASPECT_CONTENT)
                .unwrap()
                .0,
            131473
        );
    }

    // In-place activation inside a window of the test's own.
    // SAFETY: the STATIC class is the system's; the window is destroyed below.
    let parent = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            500,
            500,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let site: IOleClientSite = FakeSite {
        parent,
        seen: seen.clone(),
        blind: false,
    }
    .into();
    // SAFETY: calls on live interfaces with valid arguments.
    unsafe {
        control.SetClientSite(&site).unwrap();
        control
            .DoVerb(
                OLEIVERB_INPLACEACTIVATE.0,
                ptr::null(),
                &site,
                0,
                parent,
                ptr::null(),
            )
            .unwrap();
        let window = in_place.GetWindow().unwrap();
        assert!(IsWindow(Some(window)).as_bool());
        assert_eq!(GetParent(window).unwrap(), parent);
        assert!(
            IsWindowVisible(window).as_bool() || !IsWindowVisible(parent).as_bool(),
            "the child is visible when its parent is"
        );
        assert_eq!(size(window), (300, 400));

        // The container moves it.
        let moved = RECT {
            left: 10,
            top: 20,
            right: 210,
            bottom: 120,
        };
        in_place.SetObjectRects(&moved, &moved).unwrap();
        assert_eq!(size(window), (200, 100));

        // Hiding keeps the window; deactivating destroys it and tells the site.
        control
            .DoVerb(OLEIVERB_HIDE.0, ptr::null(), &site, 0, parent, ptr::null())
            .unwrap();
        assert!(!IsWindowVisible(window).as_bool());
        in_place.InPlaceDeactivate().unwrap();
        assert!(!IsWindow(Some(window)).as_bool(), "the window is gone");
        control.SetClientSite(None::<&IOleClientSite>).unwrap();
        let _ = DestroyWindow(parent);
    }
    assert_eq!(*seen.site_events.borrow(), vec!["activate", "deactivate"]);

    // A host that carries the parent window and the room in DoVerb itself: the in-place site answers neither, or there is
    // no in-place site at all. The control must still make its window where DoVerb says.
    let hint = RECT {
        left: 0,
        top: 0,
        right: 150,
        bottom: 250,
    };
    for blind in [true, false] {
        let site: IOleClientSite = if blind {
            FakeSite {
                parent,
                seen: seen.clone(),
                blind: true,
            }
            .into()
        } else {
            BareSite.into()
        };
        // SAFETY: the STATIC class is the system's; the window is destroyed below.
        let parent = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                500,
                500,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        // SAFETY: the factory call has no outer unknown.
        let control: IOleObject = unsafe {
            class_factory(&CLSID_PANE)
                .CreateInstance::<_, IOleObject>(None)
                .unwrap()
        };
        let place: IOleInPlaceObject = control.cast().unwrap();
        // SAFETY: calls on live interfaces; `hint` outlives them.
        unsafe {
            control.SetClientSite(&site).unwrap();
            control
                .DoVerb(
                    OLEIVERB_INPLACEACTIVATE.0,
                    ptr::null(),
                    &site,
                    0,
                    parent,
                    &hint,
                )
                .unwrap();
            let window = place.GetWindow().unwrap();
            assert_eq!(GetParent(window).unwrap(), parent, "blind={blind}");
            assert_eq!(size(window), (150, 250), "blind={blind}");
            place.InPlaceDeactivate().unwrap();
            control.SetClientSite(None::<&IOleClientSite>).unwrap();
            let _ = DestroyWindow(parent);
        }
        drop((place, control, site));
    }
    let lines = std::fs::read_to_string(&log).unwrap();
    assert!(
        lines.contains("pane DoVerb -5 parent_given=true"),
        "{lines}"
    );
    assert!(lines.contains("pane CanInPlaceActivate -> ok"), "{lines}");
    assert!(
        lines.contains("pane GetWindowContext -> hr=0x80004005"),
        "{lines}"
    );
    assert!(
        lines.contains("the client site has no IOleInPlaceSite"),
        "{lines}"
    );
    assert!(lines.contains("pane DoVerb -> S_OK"), "{lines}");

    // Everything released: the DLL may unload.
    let _ = PaneControl::new;
    drop((
        in_place,
        _view,
        _persist,
        control,
        site,
        ribbon,
        consumer,
        dispatch,
        office_factory,
    ));
    // SAFETY: the connection is over.
    unsafe { assert!(extensibility.OnDisconnection(1, ptr::null_mut()).is_ok()) };
    drop(extensibility);
    drop(factory);
    assert_eq!(DllCanUnloadNow(), S_OK, "no object is left");
    let _ = std::fs::remove_file(&log);
}
