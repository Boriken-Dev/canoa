<#
.SYNOPSIS
Registers or removes Canoa's add-in manifest for Word, Excel and PowerPoint on Windows.

.DESCRIPTION
Office on Windows loads every manifest listed under HKCU\Software\Microsoft\Office\16.0\WEF\Developer when an application
starts: the value's name is the add-in's id and its data is the path of the manifest file. This writes and removes that one
value, nothing else, which is what Microsoft's office-addin-debugging tool does for a sideloaded add-in. Run it yourself, with
Office closed; start the application again afterwards.

  add     writes the value (again, with the same result, when it is there already)
  remove  removes the value, and any value of this manifest that an older tool named by its path
  status  says whether the value is there

.PARAMETER Action
add, remove or status.

.PARAMETER Manifest
The rendered manifest file (clients/office/render.mjs documents ...). Its <Id> names the value.

.PARAMETER Key
The registry key to use. The default is the one Office reads; a different one only serves to try this script without Office.

.EXAMPLE
pwsh clients/office/sideload.ps1 add -Manifest $env:LOCALAPPDATA\Canoa\office\documents.xml
#>
[CmdletBinding(SupportsShouldProcess)]
param(
  [Parameter(Mandatory, Position = 0)][ValidateSet('add', 'remove', 'status')][string]$Action,
  [Parameter(Mandatory)][string]$Manifest,
  [string]$Key = 'HKCU:\Software\Microsoft\Office\16.0\WEF\Developer'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
  throw "The manifest is not a file: $Manifest"
}
$path = (Resolve-Path -LiteralPath $Manifest).ProviderPath
[xml]$xml = Get-Content -LiteralPath $path -Raw
$id = $xml.OfficeApp.Id
if ($id -isnot [string] -or $id -notmatch '^[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}$') {
  throw "The manifest has no <Id> that is a GUID: $path"
}

function Get-Entries {
  if (-not (Test-Path -LiteralPath $Key)) { return @() }
  $item = Get-Item -LiteralPath $Key
  foreach ($name in $item.GetValueNames()) {
    [pscustomobject]@{ Name = $name; Data = [string]$item.GetValue($name) }
  }
}

switch ($Action) {
  'add' {
    if ($PSCmdlet.ShouldProcess("$Key  $id = $path", 'Register the add-in')) {
      if (-not (Test-Path -LiteralPath $Key)) { New-Item -Path $Key -Force | Out-Null }
      Set-ItemProperty -LiteralPath $Key -Name $id -Value $path -Type String
    }
    "Registered $id -> $path"
  }
  'remove' {
    $found = @(Get-Entries | Where-Object { $_.Name -eq $id -or $_.Data -eq $path })
    foreach ($entry in $found) {
      if ($PSCmdlet.ShouldProcess("$Key  $($entry.Name)", 'Remove the registration')) {
        Remove-ItemProperty -LiteralPath $Key -Name $entry.Name
      }
    }
    if ($found.Count -eq 0) { "Nothing registered for $id" } else { "Removed $($found.Count) registration(s) of $id" }
  }
  'status' {
    $found = @(Get-Entries | Where-Object { $_.Name -eq $id -or $_.Data -eq $path })
    if ($found.Count -eq 0) { "Not registered: $id" } else { $found | ForEach-Object { "Registered: $($_.Name) -> $($_.Data)" } }
  }
}
