# `canoa-office` — API header

Header-file-style reference for `canoa-office`: Canoa's COM add-in for classic desktop Office (Word, Excel, PowerPoint, Outlook) on Windows. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `crate-type = ["cdylib", "rlib"]`: the DLL is what Office loads, the rlib is what the tests link. The crate is **empty off Windows** (`#![cfg(windows)]`). Depends on `windows` and `windows-core` (the `implement` and `interface` macros name the second in what they write); `windows-registry` in tests. `unsafe_code = "deny"`; the five COM-glue files `addin.rs`, `exports.rs`, `icon.rs`, `interfaces.rs` and `pane.rs` each `#![allow(unsafe_code)]`, every block carries `// SAFETY:`, and `tests/unsafe_files.rs` lists them and fails when another file gains `unsafe`.

## What the DLL exports

```rust
pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT;
pub extern "system" fn DllCanUnloadNow() -> HRESULT;   // S_OK when no object and no LockServer lock is alive
```

No `DllRegisterServer`, `DllMain` or definition file: registration is `tools/office_addin.ps1`. `DllGetClassObject` writes `*ppv = null` first; answers `E_POINTER` for a null argument and `CLASS_E_CLASSNOTAVAILABLE` for any other class; no panic crosses it.

| Class | CLSID | ProgID | Interfaces |
| --- | --- | --- | --- |
| `AddIn` | `CLSID_ADDIN` `{F1780EA2-B255-41D0-9014-97A28C29EB01}` | `PROG_ID` `dev.boriken.canoa` | `IDTExtensibility2`, `IRibbonExtensibility`, `ICustomTaskPaneConsumer`, `IDispatch` |
| `PaneControl` | `CLSID_PANE` `{9A886EBD-07B1-4695-B5AF-C082D892E989}` | `PANE_PROG_ID` `dev.boriken.canoa.pane` | `IOleObject`, `IOleInPlaceObject`, `IViewObject2`, `IPersistStreamInit`, `IOleControl` |

Both are apartment-threaded, never aggregated (`CLASS_E_NOAGGREGATION`). Office creates the add-in by ProgID from its `Addins` key. It hands over its task pane factory (`CTPFactoryAvailable`) before any window exists, so the add-in only keeps it. The ribbon button (`onAction="OnTogglePane"`) makes the pane on its first click, for the active window (`CreateCTP` with `PANE_PROG_ID`, docked right), shows it, and then shows and hides it. A failed `CreateCTP` is logged with its HRESULT and told to the person in a message box (`CANOA_OFFICE_NO_DIALOG` set: no box, for tests); the next click asks again. Office reaches the callbacks by name through `IDispatch` (`GetIDsOfNames` answers `OnTogglePane` and `GetImage`, case-insensitively). `GetImage` returns Canoa's mark (`MARK_PNG`, the browser extension's 32 px PNG, embedded at build time) as an `IPictureDisp` over a 32-bit premultiplied bitmap (`mark_picture()`, WIC), so its transparency is kept. `ribbon_xml(id)` is the ribbon XML for Word, Excel, PowerPoint and Outlook's explorer, read and compose ribbons, `None` for any other id.

`pub mod interfaces` declares what Office defines in type libraries and `windows` does not carry: `IDTExtensibility2`, `IRibbonExtensibility`, `ICustomTaskPaneConsumer`, `ICTPFactory`, `ICustomTaskPane`. Method order is the type library's, hence the vtable's; boolean properties are `VARIANT_BOOL` (0 or -1) and incoming strings `*const u16` (a BSTR the caller owns).

The pane's control makes one child window on `DoVerb` (`OLEIVERB_INPLACEACTIVATE`, `SHOW`, `UIACTIVATE`, `PRIMARY`) inside the container's window, follows `SetObjectRects`, and destroys it on `InPlaceDeactivate`, `Close`, `SetClientSite(null)` and drop. Its window paints a name only. `IOleControl` is answered with no control info. Each step (created, SetClientSite, DoVerb) is logged.

## Registration: `tools/office_addin.ps1`

`add|remove|status -Dll <path> [-Apps Word,Outlook,...] [-Root <HKCU key>]`. It writes, under the per-user `HKCU\Software`: the two ProgIDs and CLSIDs (`InprocServer32` = the DLL, `ThreadingModel = Apartment`; the pane's CLSID also `Control` and `MiscStatus\1 = 131473`) and one `Microsoft\Office\<App>\Addins\dev.boriken.canoa` per application (`FriendlyName`, `Description`, `LoadBehavior = 3`). `remove` deletes exactly those keys. A `-Root` outside `HKCU:\` is refused; the tests run it against `HKCU:\Software\CanoaTest\...`. The ids in the script equal `ids.rs` (a test compares them).

## Behaviour that is easy to get wrong

- **The DLL's architecture must be the Office process's.** A 64-bit x64 Office loads an x64 or an ARM64EC DLL and never a native ARM64 one; a native ARM64 Office loads ARM64 or ARM64EC; 32-bit Office needs x86 and the `WOW6432Node` view. The classes are registered in the 64-bit view.
- Office disables an add-in that is slow to load or crashes it: `OnConnection` only logs. `OnConnection` and every other event append one line of metadata (time, pid, exe, architecture, event; never content) to `%LOCALAPPDATA%\Canoa\office-addin.log`, or to the file `CANOA_OFFICE_LOG` names.
- Registering and unregistering with the Office application closed: it reads the keys at start.
- `cargo test` does not build a library's `cdylib`; `tests/dll.rs` runs `cargo build` itself (same profile, same `--target`) and loads the DLL by path. An x64 DLL cannot be loaded by an ARM64 test process: run the tests for that target to cover it (`cargo test --target x86_64-pc-windows-msvc`, emulated on ARM64).
- The linker's `LNK4104` for the two exports is silenced in `build.rs`.
