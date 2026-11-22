#requires -Version 5.1
# The helper the host runs to take a Microsoft Graph access token from the person's Windows sign-in, through the Web Account
# Manager (WAM), without asking them anything. It reads one JSON request (base64, as the argument of Invoke-CanoaMail), asks WAM
# for a token silently for the app registration the request names, and writes one JSON answer to standard output:
# {"ok":true,"result":{"token":"..."}} or {"ok":false,"error":"..."}.
#
# The token is the only secret that passes through here; it is written to standard output for the host and nowhere else. A silent
# request never shows a window: when WAM would have to ask the person to sign in or to consent, the answer says so instead.
# WinRT is reachable from Windows PowerShell 5.1 only, so the host starts powershell.exe, never PowerShell 7.

$ErrorActionPreference = 'Stop'

function Wait-WinRT($operation, [Type]$result) {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    } | Select-Object -First 1
    $task = $asTask.MakeGenericMethod($result).Invoke($null, @($operation))
    [void]$task.Wait(-1)
    return $task.Result
}

function Get-WamToken($request) {
    $manager = [Windows.Security.Authentication.Web.Core.WebAuthenticationCoreManager, Windows.Security.Authentication.Web.Core, ContentType = WindowsRuntime]
    $providerType = [Windows.Security.Credentials.WebAccountProvider, Windows.Security.Credentials, ContentType = WindowsRuntime]
    $requestType = [Windows.Security.Authentication.Web.Core.WebTokenRequest, Windows.Security.Authentication.Web.Core, ContentType = WindowsRuntime]
    $resultType = [Windows.Security.Authentication.Web.Core.WebTokenRequestResult, Windows.Security.Authentication.Web.Core, ContentType = WindowsRuntime]

    $authority = 'organizations'
    if ($request.tenant) { $authority = [string]$request.tenant }
    $provider = Wait-WinRT ($manager::FindAccountProviderAsync('https://login.microsoft.com', $authority)) $providerType
    if (-not $provider) { throw "Windows has no Microsoft account provider for $authority" }

    $ask = $requestType::new($provider, [string]$request.scope, [string]$request.client_id)
    # Ask for a version 2 token, the kind Graph takes for a scope.
    # The request's property map is a WinRT collection that PowerShell cannot call: a few lines of C# add to it.
    if (-not ('CanoaMap' -as [type])) {
        Add-Type -TypeDefinition @"
using System.Collections.Generic;
public static class CanoaMap {
    public static void Put(object map, string key, string value) { ((IDictionary<string, string>)map).Add(key, value); }
}
"@
    }
    [CanoaMap]::Put($ask.Properties, 'wam_compat', '2.0')
    if ($request.login_hint) { [CanoaMap]::Put($ask.Properties, 'login_hint', [string]$request.login_hint) }

    $answer = Wait-WinRT ($manager::GetTokenSilentlyAsync($ask)) $resultType
    switch ([string]$answer.ResponseStatus) {
        'Success' {
            $token = [string]$answer.ResponseData[0].Token
            if (-not $token) { throw 'Windows answered with no token' }
            return @{ token = $token }
        }
        'UserInteractionRequired' {
            throw 'Windows would have to ask the person to sign in or to allow the app, which a silent request never does'
        }
        'AccountProviderNotAvailable' { throw 'the Microsoft account provider is not available' }
        'UserCancel' { throw 'the sign-in was cancelled' }
        default {
            $why = ''
            try { $why = ' (' + [string]$answer.ResponseError.ErrorCode + ' ' + [string]$answer.ResponseError.ErrorMessage + ')' } catch { }
            throw ('Windows refused the token: ' + [string]$answer.ResponseStatus + $why)
        }
    }
}

function Invoke-CanoaMail([string]$encoded) {
    try {
        $json = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encoded))
        $request = $json | ConvertFrom-Json
        $result = switch ($request.op) {
            'token' { Get-WamToken $request }
            default { throw "no operation is named $($request.op)" }
        }
        $answer = @{ ok = $true; result = $result }
    } catch {
        $answer = @{ ok = $false; error = [string]$_.Exception.Message }
    }
    $text = ConvertTo-Json -InputObject $answer -Depth 4 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($text)
    $out = [Console]::OpenStandardOutput()
    $out.Write($bytes, 0, $bytes.Length)
    $out.Flush()
}
