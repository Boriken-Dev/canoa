<#
.SYNOPSIS
Registers or removes Canoa's COM add-in for classic desktop Office, for the current user only.

.DESCRIPTION
A COM add-in is a class (a CLSID and a ProgID that name the DLL) plus, under each Office application, an Addins key named by the
ProgID that tells the application to load it. This writes the keys below and removes exactly those; nothing is written outside the
current user's hive, so no administrator rights are needed. Run it with the Office applications closed.

  <Root>\Classes\dev.boriken.canoa                  the add-in's ProgID
  <Root>\Classes\CLSID\{F1780EA2-...}               the add-in class, its InprocServer32 naming the DLL
  <Root>\Classes\dev.boriken.canoa.pane             the ProgID Office is given for the task pane's control
  <Root>\Classes\CLSID\{9A886EBD-...}               the control class, marked as a control
  <Root>\Microsoft\Office\<App>\Addins\dev.boriken.canoa   one per application in -Apps: FriendlyName, Description, LoadBehavior = 3

  add     writes the keys (again, with the same result, when they are there already)
  remove  removes exactly those keys
  status  says which of them are there

.PARAMETER Action
add, remove or status.

.PARAMETER Dll
The add-in's DLL. Its architecture must be the one the Office application runs as (x64 or ARM64EC for a 64-bit Office).

.PARAMETER Apps
The applications to register for: any of Outlook, Word, Excel, PowerPoint, as a list or comma separated. The default is Word.

.PARAMETER Root
The registry key that stands for HKCU\Software. The default is the real one; a different key (under HKCU) only serves to try this
script without Office. A key outside HKCU is refused.

.EXAMPLE
pwsh crates/canoa-office/tools/office_addin.ps1 add -Dll C:\Canoa\canoa_office.dll -Apps Word
#>
[CmdletBinding(SupportsShouldProcess)]
param(
  [Parameter(Mandatory, Position = 0)][ValidateSet('add', 'remove', 'status')][string]$Action,
  [Parameter(Mandatory)][string]$Dll,
  [string[]]$Apps = @('Word'),
  [string]$Root = 'HKCU:\Software'
)

$ErrorActionPreference = 'Stop'

# Kept equal to crates/canoa-office/src/ids.rs by a test.
$progId = 'dev.boriken.canoa'
$clsid = '{F1780EA2-B255-41D0-9014-97A28C29EB01}'
$paneProgId = 'dev.boriken.canoa.pane'
$paneClsid = '{9A886EBD-07B1-4695-B5AF-C082D892E989}'

# `-File` hands a comma list over as one string.
$Apps = @($Apps | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
foreach ($app in $Apps) {
  if ($app -notin 'Outlook', 'Word', 'Excel', 'PowerPoint') { throw "Unknown application: $app (Outlook, Word, Excel or PowerPoint)" }
}

if ($Root -notmatch '^HKCU:\\') { throw "The root must be under HKCU:\, never another hive: $Root" }
$Root = $Root.TrimEnd('\')
$classes = "$Root\Classes"
$office = "$Root\Microsoft\Office"

# The Addins key is named by the ProgID, so the ProgID is what has to be removed with it.
$classKeys = @(
  "$classes\$progId",
  "$classes\CLSID\$clsid",
  "$classes\$paneProgId",
  "$classes\CLSID\$paneClsid"
)
$addinKeys = @($Apps | ForEach-Object { "$office\$_\Addins\$progId" })

function Set-Default($key, $value) {
  New-Item -Path $key -Force | Out-Null
  Set-Item -LiteralPath $key -Value $value
}

switch ($Action) {
  'add' {
    if (-not (Test-Path -LiteralPath $Dll -PathType Leaf)) { throw "The DLL is not a file: $Dll" }
    $path = (Resolve-Path -LiteralPath $Dll).ProviderPath
    if ($PSCmdlet.ShouldProcess($Root, "Register the add-in ($path) for $($Apps -join ', ')")) {
      Set-Default "$classes\$progId" 'Canoa'
      Set-Default "$classes\$progId\CLSID" $clsid
      Set-Default "$classes\CLSID\$clsid" 'Canoa Office add-in'
      Set-Default "$classes\CLSID\$clsid\ProgID" $progId
      Set-Default "$classes\CLSID\$clsid\InprocServer32" $path
      Set-ItemProperty -LiteralPath "$classes\CLSID\$clsid\InprocServer32" -Name ThreadingModel -Value 'Apartment' -Type String

      Set-Default "$classes\$paneProgId" 'Canoa pane'
      Set-Default "$classes\$paneProgId\CLSID" $paneClsid
      Set-Default "$classes\CLSID\$paneClsid" 'Canoa Office pane'
      Set-Default "$classes\CLSID\$paneClsid\ProgID" $paneProgId
      Set-Default "$classes\CLSID\$paneClsid\InprocServer32" $path
      Set-ItemProperty -LiteralPath "$classes\CLSID\$paneClsid\InprocServer32" -Name ThreadingModel -Value 'Apartment' -Type String
      New-Item -Path "$classes\CLSID\$paneClsid\Control" -Force | Out-Null
      Set-Default "$classes\CLSID\$paneClsid\MiscStatus" '0'
      Set-Default "$classes\CLSID\$paneClsid\MiscStatus\1" '131473'
      Set-Default "$classes\CLSID\$paneClsid\Version" '1.0'
      Set-Default "$classes\CLSID\$paneClsid\VersionIndependentProgID" $paneProgId
      # The component category of controls, which a container may check before it hosts one.
      New-Item -Path "$classes\CLSID\$paneClsid\Implemented Categories\{40FC6ED4-2438-11CF-A3DB-080036F12502}" -Force | Out-Null

      foreach ($key in $addinKeys) {
        New-Item -Path $key -Force | Out-Null
        Set-ItemProperty -LiteralPath $key -Name FriendlyName -Value 'Canoa' -Type String
        Set-ItemProperty -LiteralPath $key -Name Description -Value 'Canoa chat in a task pane' -Type String
        Set-ItemProperty -LiteralPath $key -Name LoadBehavior -Value 3 -Type DWord
      }
    }
    "Registered $path for $($Apps -join ', ')"
  }
  'remove' {
    $removed = 0
    foreach ($key in @($addinKeys) + $classKeys) {
      if (Test-Path -LiteralPath $key) {
        if ($PSCmdlet.ShouldProcess($key, 'Remove')) { Remove-Item -LiteralPath $key -Recurse -Force; $removed += 1 }
      }
    }
    if ($removed -eq 0) { 'Nothing registered' } else { "Removed $removed key(s)" }
  }
  'status' {
    foreach ($key in @($addinKeys) + $classKeys) {
      $mark = if (Test-Path -LiteralPath $key) { 'present' } else { 'absent' }
      "${mark}: $key"
    }
  }
}
