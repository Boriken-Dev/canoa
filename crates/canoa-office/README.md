# canoa-office

Canoa's COM add-in for classic desktop Office on Windows: a DLL that Word, Excel, PowerPoint and Outlook load in their own
process. It is not published. The crate is empty on other platforms.

It offers three things Office asks every COM add-in for:

- `IDTExtensibility2`, the add-in contract: Office tells the add-in it was connected, disconnected and started.
- `IRibbonExtensibility`: one ribbon tab, Canoa, with one button that shows or hides the task pane.
- `ICustomTaskPaneConsumer`: Office hands the add-in the factory that makes task panes. The pane hosts an ActiveX control the
  DLL also provides; its window is empty for now.

The new Outlook for Windows and Office on the web run web add-ins only (see `clients/office`); a COM add-in loads in classic
desktop Office alone.

## Registering

`tools/office_addin.ps1` writes the per-user registration, and removes exactly what it wrote. Nothing is written outside
`HKCU`, so no administrator rights are needed. Run it with the Office applications closed:

```powershell
pwsh crates/canoa-office/tools/office_addin.ps1 add -Dll <path>\canoa_office.dll -Apps Word
pwsh crates/canoa-office/tools/office_addin.ps1 status -Dll <path>\canoa_office.dll -Apps Word
pwsh crates/canoa-office/tools/office_addin.ps1 remove -Dll <path>\canoa_office.dll -Apps Word
```

The DLL must match the application: a 64-bit Office loads an x64 or ARM64EC DLL (an ARM64 one only on a 64-bit ARM Office), a
32-bit Office an x86 one.

## Log

Each event Office sends the add-in appends one line of metadata (time, process, architecture, event; never content) to
`%LOCALAPPDATA%\Canoa\office-addin.log`, or to the file `CANOA_OFFICE_LOG` names.
