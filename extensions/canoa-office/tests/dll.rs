//! The built DLL itself, not the library linked into a test: loaded by path, its two COM entry points found by name, and the
//! add-in made through them. This is what Office does, minus Office.

#![cfg(windows)]
#![allow(unsafe_code)]

use core::ffi::c_void;
use core::ptr;
use std::path::PathBuf;
use std::process::Command;

use canoa_office::interfaces::{IDTExtensibility2, IRibbonExtensibility};
use canoa_office::{CLSID_ADDIN, CLSID_PANE};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, FreeLibrary, S_OK};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IClassFactory};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{BSTR, GUID, HRESULT, Interface, PCWSTR, s};

type GetClassObject =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type CanUnloadNow = unsafe extern "system" fn() -> HRESULT;
type Proc = unsafe extern "system" fn() -> isize;

/// Builds the DLL in this profile (the test harness does not build a library's `cdylib`) and says where it is.
fn built_dll() -> PathBuf {
    let profile_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut build = Command::new(env!("CARGO"));
    build.args([
        "build",
        "-p",
        "canoa-office",
        "--manifest-path",
        concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
    ]);
    if profile_dir
        .file_name()
        .is_some_and(|name| name == "release")
    {
        build.arg("--release");
    }
    // `cargo test --target T` puts the test in target/T/<profile>: build for T as well.
    if let Some(triple) = profile_dir
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| name.contains("-windows-"))
    {
        build.args(["--target", triple]);
    }
    let status = build.status().expect("cargo starts");
    assert!(status.success(), "the DLL did not build");
    let dll = profile_dir.join("canoa_office.dll");
    assert!(dll.is_file(), "no DLL at {}", dll.display());
    dll
}

#[test]
fn the_built_dll_exports_the_com_entry_points_and_makes_the_add_in() {
    let dll = built_dll();
    let wide: Vec<u16> = dll.to_string_lossy().encode_utf16().chain([0]).collect();
    // SAFETY: a path to the DLL just built; it is unloaded at the end and nothing it made outlives that.
    let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())).expect("the DLL loads") };

    // SAFETY: the two names are the DLL's documented exports, and the signatures are COM's own.
    let (get_class_object, can_unload_now): (GetClassObject, CanUnloadNow) = unsafe {
        (
            core::mem::transmute::<Proc, GetClassObject>(
                GetProcAddress(module, s!("DllGetClassObject"))
                    .expect("DllGetClassObject is exported"),
            ),
            core::mem::transmute::<Proc, CanUnloadNow>(
                GetProcAddress(module, s!("DllCanUnloadNow")).expect("DllCanUnloadNow is exported"),
            ),
        )
    };
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };

    // SAFETY: valid GUIDs and writable slots throughout.
    unsafe {
        let mut out: *mut c_void = ptr::null_mut();
        assert_eq!(
            get_class_object(&GUID::from_u128(1), &IClassFactory::IID, &mut out),
            CLASS_E_CLASSNOTAVAILABLE
        );

        for clsid in [CLSID_ADDIN, CLSID_PANE] {
            assert_eq!(
                get_class_object(&clsid, &IClassFactory::IID, &mut out),
                S_OK
            );
            drop(IClassFactory::from_raw(out));
        }

        assert_eq!(
            get_class_object(&CLSID_ADDIN, &IClassFactory::IID, &mut out),
            S_OK
        );
        let factory = IClassFactory::from_raw(out);
        let addin: IDTExtensibility2 = factory.CreateInstance(None).unwrap();
        let ribbon: IRibbonExtensibility = addin.cast().unwrap();
        let id = BSTR::from("Microsoft.Word.Document");
        let mut xml = BSTR::new();
        assert!(ribbon.GetCustomUI(id.as_ptr(), &mut xml).is_ok());
        assert!(xml.to_string().contains("OnTogglePane"));
        assert_ne!(can_unload_now(), S_OK, "the add-in is alive");
        drop((ribbon, addin, factory));
        assert_eq!(can_unload_now(), S_OK);
        FreeLibrary(module).unwrap();
    }
}
