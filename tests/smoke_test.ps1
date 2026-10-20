<#
Native integration smoke for an explicitly authorized disposable Windows VM/session.
Real Block/Allow, PowerRequests, ETW and scheduled-task effects occur when run.
No compilation: tests the supplied exact binary. Guards refuse known conflicts;
a dedicated session with no concurrent Wardoff actors is still required because
production CLI singleton forwarding cannot pin a target PID atomically.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $BinaryPath,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string] $ExpectedBinarySha256,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [switch] $IsolatedTarget,
    [switch] $RequireAutostartLifecycle
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'This harness requires PowerShell 7.' }
if (-not $IsolatedTarget) { throw 'Refusing native smoke: an authorized disposable VM/session is required (-IsolatedTarget describes that prerequisite, not authorization).' }
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$binaryPath = [IO.Path]::GetFullPath($BinaryPath)
if ([IO.Path]::GetFileName($binaryPath) -ine 'wardoff.exe') { throw 'The supplied production executable must retain the name wardoff.exe.' }
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) { throw 'The exact supplied binary does not exist; no build fallback is allowed.' }
$runRoot = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) ('smoke-' + [guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $runRoot) { throw 'Evidence destination already exists.' }
[void](New-Item -ItemType Directory -Path $runRoot -ErrorAction Stop)
$script:RuntimeEnvironment = @{ APPDATA=(Join-Path $runRoot 'appdata'); LOCALAPPDATA=(Join-Path $runRoot 'localappdata') }
foreach ($path in $script:RuntimeEnvironment.Values) { [void](New-Item -ItemType Directory -Path $path -ErrorAction Stop) }
$logPath = Join-Path $script:RuntimeEnvironment.LOCALAPPDATA 'Wardoff/logs/wardoff.jsonl'
$script:Results = [Collections.Generic.List[object]]::new()
$script:OwnedProcesses = [Collections.Generic.List[object]]::new()
$script:BackgroundProcess = $null
$script:InitialLogLineCount = 0
$script:RuntimeOutput = $null
$script:RuntimeError = $null
$script:AutostartLifecyclePassed = $false
$script:Lease = [Diagnostics.Stopwatch]::StartNew()
$script:BinaryLock = $null
. (Join-Path $PSScriptRoot 'smoke_helpers.ps1')
. (Join-Path $PSScriptRoot 'smoke_process.ps1')
. (Join-Path $PSScriptRoot 'smoke_resources.ps1')
$script:ResourceState = New-SmokeResourceState -BinaryPath $binaryPath -WorkingDirectory (Split-Path -Parent $binaryPath)

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-CurrentTokenElevationType {
    if (-not ('Wardoff.TokenElevationTypeReader' -as [type])) {
        Add-Type @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;

namespace Wardoff {
    public static class TokenElevationTypeReader {
        private const int TokenElevationTypeInformation = 18;

        [DllImport("advapi32.dll", SetLastError = true)]
        private static extern bool GetTokenInformation(
            IntPtr tokenHandle,
            int tokenInformationClass,
            out int tokenInformation,
            int tokenInformationLength,
            out int returnLength);

        public static int GetCurrentValue() {
            using (WindowsIdentity identity = WindowsIdentity.GetCurrent()) {
                int value;
                int returnLength;
                if (!GetTokenInformation(
                    identity.Token,
                    TokenElevationTypeInformation,
                    out value,
                    sizeof(int),
                    out returnLength)) {
                    throw new Win32Exception(
                        Marshal.GetLastWin32Error(),
                        "GetTokenInformation(TokenElevationType) failed");
                }

                if (returnLength < sizeof(int)) {
                    throw new InvalidOperationException(
                        "GetTokenInformation(TokenElevationType) returned an incomplete value");
                }

                return value;
            }
        }
    }
}
"@
    }

    $value = [Wardoff.TokenElevationTypeReader]::GetCurrentValue()
    if ($value -notin @(1, 2, 3)) {
        throw "Unexpected TokenElevationType value $value."
    }

    return $value
}

function Get-AutostartValidationDecision {
    param(
        [AllowNull()]
        [Nullable[int]] $TokenElevationType
    )

    if ($null -eq $TokenElevationType) {
        return 'query-failed'
    }

    switch ($TokenElevationType) {
        1 { return 'skip-no-linked-token' }
        2 { return 'run-lifecycle' }
        default { return 'fail-unexpected-value' }
    }
}

function Test-RebootTaskPresent {
    $scheduler = $null
    $folder = $null
    $task = $null
    try {
        $scheduler = New-Object -ComObject 'Schedule.Service'
        $scheduler.Connect()
        $folder = $scheduler.GetFolder('\Microsoft\Windows\UpdateOrchestrator')
        $task = $folder.GetTask('Reboot')
        return $true
    }
    catch {
        # Match the runtime's missing-file, missing-path and missing-object cases.
        # Permission and other scheduler errors must still fail validation.
        $hresult = $_.Exception.GetBaseException().HResult
        if ($hresult -in @(-2147024894, -2147024893, -2147023728)) {
            return $false
        }
        throw
    }
    finally {
        foreach ($comObject in @($task, $folder, $scheduler)) {
            if ($null -ne $comObject) {
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($comObject)
            }
        }
    }
}

$isAdmin = Test-IsAdministrator

if (-not $isAdmin) {
    Write-Host 'Run as admin for full test coverage.' -ForegroundColor Yellow
}

function Add-TestResult {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('PASS', 'FAIL', 'SKIP')]
        [string] $Status,
        [Parameter(Mandatory = $true)]
        [string] $Description,
        [string] $Details,
        [bool] $CountInSummary = $true
    )

    $script:Results.Add(
        [pscustomobject]@{
            Status         = $Status
            Description    = $Description
            Details        = $Details
            CountInSummary = $CountInSummary
        }
    ) | Out-Null

    $color = switch ($Status) {
        'PASS' { 'Green' }
        'FAIL' { 'Red' }
        default { 'Yellow' }
    }

    Write-Host "$Status - $Description" -ForegroundColor $color
    if ($Details) {
        Write-Host "       $Details"
    }
}

function Invoke-TestCase {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Description,
        [Parameter(Mandatory = $true)]
        [scriptblock] $ScriptBlock
    )

    try {
        & $ScriptBlock
        Add-TestResult -Status 'PASS' -Description $Description
    }
    catch {
        Add-TestResult -Status 'FAIL' -Description $Description -Details $_.Exception.Message
    }
}

function Skip-TestCase {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Description,
        [Parameter(Mandatory = $true)]
        [string] $Reason
    )

    Add-TestResult -Status 'SKIP' -Description $Description -Details $Reason -CountInSummary $false
}

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)]
        [bool] $Condition,
        [Parameter(Mandatory = $true)]
        [string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-ExternalCommand {
    param([Parameter(Mandatory)][string] $FilePath, [string[]] $Arguments=@(),
          [string] $WorkingDirectory=(Split-Path -Parent $binaryPath))
    if ($script:Lease.Elapsed.TotalSeconds -gt 180) { throw 'Smoke runtime lease exceeded 180 seconds.' }
    $isProduct = [string]::Equals([IO.Path]::GetFullPath($FilePath), $binaryPath, [StringComparison]::OrdinalIgnoreCase)
    if ($isProduct -and (@($Arguments | Where-Object { $_ -in @('--block','--allow','--autostart') }).Count -gt 0)) {
        Assert-SmokeOwnedRuntime
    }
    if ($isProduct -and (@($Arguments | Where-Object { $_ -in @('--block','--allow','--hide') }).Count -gt 0)) {
        $Arguments += '--wardoff-detached-runtime'
    }
    $options=@{ FilePath=$FilePath; Arguments=$Arguments; WorkingDirectory=$WorkingDirectory; Environment=$script:RuntimeEnvironment; TimeoutMilliseconds=15000 }
    if ($isProduct) {
        $options.RetainProcess=$true
        $options.OnStarted={ param($process) $script:OwnedProcesses.Add($process) }
    }
    Invoke-SmokeChildCommand @options
}

function Invoke-ExternalCommandWithRepoProcessTracking {
    param([Parameter(Mandatory)][string] $FilePath, [string[]] $Arguments=@(),
          [string] $WorkingDirectory=(Split-Path -Parent $binaryPath),
          [int] $PollIntervalMilliseconds=25, [int] $PostExitGraceMilliseconds=250)
    $initial=@(Get-WardoffProcessesByName | ForEach-Object { [int]$_.ProcessId })
    $observed=[Collections.Generic.HashSet[int]]::new()
    $repoObserved=[Collections.Generic.HashSet[int]]::new()
    $started=@{ Id=0 }
    $r=Invoke-SmokeChildCommand -FilePath $FilePath -Arguments $Arguments -WorkingDirectory $WorkingDirectory `
        -Environment $script:RuntimeEnvironment -TimeoutMilliseconds 15000 -RetainProcess `
        -OnStarted { param($p) $started.Id=$p.Id; $script:OwnedProcesses.Add($p) } `
        -OnPoll {
            foreach($p in @(Get-WardoffProcessesByName)){ [void]$observed.Add([int]$p.ProcessId) }
            foreach($p in @(Get-RepoWardoffProcesses)){ [void]$repoObserved.Add([int]$p.ProcessId) }
        }
    [pscustomobject]@{ ExitCode=$r.ExitCode; StandardOutput=$r.StandardOutput; StandardError=$r.StandardError; Output=$r.Output;
        LaunchedProcessId=$r.LaunchedProcessId; InitialWardoffProcessIds=$initial;
        FinalWardoffProcessIds=@(Get-WardoffProcessesByName | ForEach-Object { [int]$_.ProcessId });
        ObservedWardoffProcessIds=@($observed); ObservedRepoProcessIds=@($repoObserved) }
}

function Convert-ToSingleQuotedPowerShellLiteral {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string] $Text
    )

    return "'" + $Text.Replace("'", "''") + "'"
}

function Invoke-DirectPowerShellWardoffCommand {
    param(
        [string[]] $Arguments = @()
    )

    $argumentLiterals = @(
        $Arguments | ForEach-Object { Convert-ToSingleQuotedPowerShellLiteral -Text ([string] $_) }
    ) -join ', '
    $binaryLiteral = Convert-ToSingleQuotedPowerShellLiteral -Text ([string] $binaryPath)
    $command = "& $binaryLiteral @($argumentLiterals)`nexit `$LASTEXITCODE"

    return Invoke-ExternalCommand -FilePath 'powershell' -Arguments @(
        '-NoProfile',
        '-NonInteractive',
        '-ExecutionPolicy',
        'Bypass',
        '-Command',
        $command
    )
}

function Assert-SmokePreflight {
    param([switch] $Initial)
    Assert-SmokePreconditions -WardoffProcesses @(Get-WardoffProcessesByName) `
        -CurrentSessionId (Get-SmokeCurrentSessionId) -BinaryPath $binaryPath `
        -OwnedProcesses $script:OwnedProcesses.ToArray() -AutostartTaskProbe {
            if ($Initial) { return (& $script:ResourceState.Adapters.ReadTask '\Wardoff').Exists }
            return $false
        }
    if ($Initial) { Assert-SmokeNamespaceUnused }
}

function Assert-SmokeNamespaceUnused {
        $session=Get-SmokeCurrentSessionId
        $mutex=$null
        try {
            $mutex=[Threading.Mutex]::OpenExisting("Local\WardoffInstance-Session-$session")
            throw 'A primary Wardoff mutex already exists, including a possible renamed runtime.'
        } catch [Threading.WaitHandleCannotBeOpenedException] {
            # The named object is absent. Other errors remain fatal.
        } finally { if($null -ne $mutex){ $mutex.Dispose() } }
        $pipeNames=@([IO.Directory]::GetFiles('\\.\pipe\'))
        foreach($name in $pipeNames) {
            if ($name -match ('(?i)(?:^|\\)Wardoff(?:Control|Status)-Session-' + $session + '$')) {
                throw 'A Wardoff pipe already exists; refusing to contact its owner.'
            }
        }
}

function Assert-SmokeOwnedRuntime {
    if ($null -eq $script:BackgroundProcess -or $script:BackgroundProcess.HasExited) {
        throw 'The owned runtime is not alive; refusing a CLI command that could start or control another primary.'
    }
    Assert-SmokePreflight
}

function Start-SmokeRuntime {
    Assert-SmokePreflight -Initial
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=$binaryPath; $info.WorkingDirectory=Split-Path -Parent $binaryPath
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    foreach($arg in @('--block','--hide','--wardoff-detached-runtime')){ $info.ArgumentList.Add($arg) }
    foreach($key in $script:RuntimeEnvironment.Keys){ $info.Environment[$key]=$script:RuntimeEnvironment[$key] }
    $p=[Diagnostics.Process]::new();$p.StartInfo=$info
    try {
        [void]$p.Start();[void]$p.Handle
        $script:OwnedProcesses.Add($p);$script:BackgroundProcess=$p
        $script:RuntimeOutput=$p.StandardOutput.ReadToEndAsync();$script:RuntimeError=$p.StandardError.ReadToEndAsync()
    } catch { if($null -eq $script:BackgroundProcess){$p.Dispose()};throw }
}

function Get-WardoffProcessesByName {
    $processes = Get-CimInstance Win32_Process -Filter "Name='wardoff.exe'" -ErrorAction Stop

    if ($null -eq $processes) {
        return @()
    }

    return @($processes)
}

function Get-RepoWardoffProcesses {
    if (-not (Test-Path $binaryPath)) {
        return @()
    }

    $resolvedBinaryPath = [System.IO.Path]::GetFullPath($binaryPath)
    $processes = Get-WardoffProcessesByName

    return @(
        $processes | Where-Object {
            $_.ExecutablePath -and
            [string]::Equals(
                [System.IO.Path]::GetFullPath($_.ExecutablePath),
                $resolvedBinaryPath,
                [System.StringComparison]::OrdinalIgnoreCase
            )
        }
    )
}

function Get-EmbeddedManifestContent {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if (-not ('Wardoff.ManifestReader' -as [type])) {
        Add-Type @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace Wardoff {
    public static class ManifestReader {
        private const uint LOAD_LIBRARY_AS_DATAFILE = 0x00000002;
        private static readonly IntPtr ManifestResourceId = new IntPtr(1);
        private static readonly IntPtr ManifestResourceType = new IntPtr(24);

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        private static extern IntPtr LoadLibraryEx(string lpFileName, IntPtr hFile, uint dwFlags);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr FindResource(IntPtr hModule, IntPtr lpName, IntPtr lpType);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr LoadResource(IntPtr hModule, IntPtr hResInfo);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr LockResource(IntPtr hResData);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint SizeofResource(IntPtr hModule, IntPtr hResInfo);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool FreeLibrary(IntPtr hModule);

        public static string ReadManifest(string path) {
            var module = LoadLibraryEx(path, IntPtr.Zero, LOAD_LIBRARY_AS_DATAFILE);
            if (module == IntPtr.Zero) {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "LoadLibraryEx failed for " + path);
            }

            try {
                var manifestInfo = FindResource(module, ManifestResourceId, ManifestResourceType);
                if (manifestInfo == IntPtr.Zero) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "FindResource could not locate the embedded manifest.");
                }

                var manifestBytesLength = SizeofResource(module, manifestInfo);
                if (manifestBytesLength == 0) {
                    throw new InvalidOperationException("The embedded manifest resource was empty.");
                }

                var manifestHandle = LoadResource(module, manifestInfo);
                if (manifestHandle == IntPtr.Zero) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "LoadResource failed for the embedded manifest.");
                }

                var manifestPointer = LockResource(manifestHandle);
                if (manifestPointer == IntPtr.Zero) {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "LockResource failed for the embedded manifest.");
                }

                var manifestBytes = new byte[(int)manifestBytesLength];
                Marshal.Copy(manifestPointer, manifestBytes, 0, (int)manifestBytesLength);
                return Encoding.UTF8.GetString(manifestBytes);
            }
            finally {
                FreeLibrary(module);
            }
        }
    }
}
"@
    }

    return [Wardoff.ManifestReader]::ReadManifest([System.IO.Path]::GetFullPath($Path))
}

function Wait-ForBinaryUnlock {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path,
        [int] $Attempts = 5,
        [int] $DelayMilliseconds = 400
    )

    if (-not (Test-Path $Path)) {
        return
    }

    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        $stream = $null

        try {
            $stream = [System.IO.File]::Open(
                $Path,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
            return
        }
        catch [System.IO.IOException] {
            if ($attempt -eq $Attempts) {
                throw "Timed out waiting for '$Path' to become readable and unlocked after Wardoff cleanup. A stale wardoff.exe process or another handle may still be holding the binary. Stop any remaining Wardoff processes and retry the smoke test. Last error: $($_.Exception.Message)"
            }
        }
        catch [System.UnauthorizedAccessException] {
            if ($attempt -eq $Attempts) {
                throw "Timed out waiting for '$Path' to become readable and unlocked after Wardoff cleanup. A stale wardoff.exe process or another handle may still be holding the binary. Stop any remaining Wardoff processes and retry the smoke test. Last error: $($_.Exception.Message)"
            }
        }
        finally {
            if ($null -ne $stream) {
                $stream.Dispose()
            }
        }

        Start-Sleep -Milliseconds $DelayMilliseconds
    }
}

function Stop-RepoWardoffProcesses {
    $errors=[Collections.Generic.List[string]]::new()
    foreach($p in $script:OwnedProcesses.ToArray()) {
        try { Capture-SmokeOwnedEtw -State $script:ResourceState -Process $p } catch { $errors.Add($_.Exception.Message) }
    }
    try {
        Stop-SmokeOwnedProcesses -OwnedProcesses $script:OwnedProcesses.ToArray() -StopAction {
            param($p) $p.Kill($true)
        } -WaitAction {
            param($p) if (-not $p.WaitForExit(5000)) { throw "Owned child $($p.Id) did not exit." }
        }
    } catch { $errors.Add($_.Exception.Message) }
    if($errors.Count){ throw ($errors -join '; ') }
}

function Get-StatusJson {
    param(
        [Parameter(Mandatory = $true)]
        [string] $JsonText
    )

    try {
        return $JsonText | ConvertFrom-Json -ErrorAction Stop
    }
    catch {
        throw "Output was not valid JSON: $JsonText"
    }
}

function Get-PowercfgRequests {
    return Invoke-ExternalCommand -FilePath 'powercfg' -Arguments @('/requests')
}

function Get-WardoffStatusResult {
    return Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--status')
}

function Wait-ForActiveWardoffStatus {
    $lastResult = $null

    for ($attempt = 0; $attempt -lt 5; $attempt++) {
        $lastResult = Get-WardoffStatusResult

        if ($lastResult.ExitCode -eq 0) {
            $status = Get-StatusJson -JsonText $lastResult.Output
            if ($status.state -eq 'block') {
                return [pscustomobject]@{
                    Result = $lastResult
                    Status = $status
                }
            }
        }

        Start-Sleep -Seconds 1
    }

    if ($null -eq $lastResult) {
        throw 'Wardoff never returned a status result while waiting for an active Block state.'
    }

    throw "Wardoff did not report an active Block state. Last exit code: $($lastResult.ExitCode). Last output: $($lastResult.Output)"
}

function Wait-ForInactiveWardoffStatus {
    $lastResult = $null

    for ($attempt = 0; $attempt -lt 5; $attempt++) {
        $lastResult = Get-WardoffStatusResult

        if ($lastResult.ExitCode -eq 1) {
            $status = Get-StatusJson -JsonText $lastResult.Output
            if ($status.state -eq 'inactive') {
                return $lastResult
            }
        }

        Start-Sleep -Seconds 1
        Stop-RepoWardoffProcesses
    }

    if ($null -eq $lastResult) {
        throw 'Wardoff never returned a status result while waiting for an inactive state.'
    }

    throw "Wardoff did not become inactive. Last exit code: $($lastResult.ExitCode). Last output: $($lastResult.Output)"
}

function Get-StructuredLogLines {
    if (-not (Test-Path $logPath)) {
        return @()
    }

    return @(Get-Content -Path $logPath -ErrorAction Stop | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Resolve-PowercfgRequestsCheck {
    try {
        $result = Get-PowercfgRequests
    }
    catch {
        return [pscustomobject]@{
            State          = 'fail'
            Result         = $null
            SkipReason     = $null
            FailureMessage = $_.Exception.Message
        }
    }

    if ($result.ExitCode -eq 0) {
        return [pscustomobject]@{
            State          = 'testable'
            Result         = $result
            SkipReason     = $null
            FailureMessage = $null
        }
    }

    $details = @($result.StandardError, $result.StandardOutput, $result.Output) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        Select-Object -Unique
    $detailsText = ($details -join [Environment]::NewLine).Trim()

    if ($detailsText -match '(?i)administrator privileges|elevated command prompt|access is denied|requires elevation') {
        return [pscustomobject]@{
            State          = 'skip'
            Result         = $null
            SkipReason     = 'powercfg /requests is not testable in this session because Windows requires elevation here.'
            FailureMessage = $null
        }
    }

    return [pscustomobject]@{
        State          = 'fail'
        Result         = $null
        SkipReason     = $null
        FailureMessage = "powercfg /requests exited with code $($result.ExitCode). Output: $detailsText"
    }
}

try {
    Assert-SmokePreflight -Initial
    $script:BinaryLock=[IO.File]::Open($binaryPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $hash=[Security.Cryptography.SHA256]::Create()
    try { $actualHash=[BitConverter]::ToString($hash.ComputeHash($script:BinaryLock)).Replace('-','').ToLowerInvariant() } finally { $hash.Dispose() }
    Assert-Condition ($actualHash -ceq $ExpectedBinarySha256.ToLowerInvariant()) 'The supplied binary differs from its expected SHA256.'
    Initialize-SmokeResources -State $script:ResourceState

    Invoke-TestCase 'The supplied frozen binary exists without rebuilding' {
        Assert-Condition (Test-Path $binaryPath) "Expected binary at $binaryPath."
    }

    Invoke-TestCase 'The supplied frozen binary embeds an asInvoker manifest' {
        $manifestContent = Get-EmbeddedManifestContent -Path $binaryPath
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($manifestContent)) 'The embedded manifest was empty.'
        Assert-Condition ($manifestContent -match 'requestedExecutionLevel\s+level="asInvoker"\s+uiAccess="false"') "The embedded manifest did not request asInvoker. Manifest: $manifestContent"
    }

    Invoke-TestCase 'wardoff --help exits 0 and prints non-empty usage text' {
        $result = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--help')
        Assert-Condition ($result.ExitCode -eq 0) "wardoff --help exited with code $($result.ExitCode)."
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($result.Output)) 'wardoff --help did not print any output.'
        Assert-Condition ($result.Output -match '(?i)wardoff') "wardoff --help did not mention 'wardoff'."
    }

    Invoke-TestCase 'wardoff --version exits 0 and prints a version string' {
        $result = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--version')
        Assert-Condition ($result.ExitCode -eq 0) "wardoff --version exited with code $($result.ExitCode)."
        Assert-Condition (-not [string]::IsNullOrWhiteSpace($result.Output)) 'wardoff --version did not print any output.'
        Assert-Condition ($result.Output -match '^wardoff\s+\S+$') "wardoff --version did not print a recognizable version string. Output: '$($result.Output)'."
    }

    if ($isAdmin) {
        Skip-TestCase 'direct non-admin PowerShell invocation of wardoff --status stays inline without an elevation-startup error' 'Current smoke script is already elevated, so the required non-elevated PowerShell repro path is unavailable.'
    }
    else {
        Invoke-TestCase 'direct non-admin PowerShell invocation of wardoff --status stays inline without an elevation-startup error' {
            Stop-RepoWardoffProcesses

            $result = Invoke-DirectPowerShellWardoffCommand -Arguments @('--status')
            $status = Get-StatusJson -JsonText $result.Output

            Assert-Condition ($result.ExitCode -eq 1) "direct PowerShell wardoff --status exited with code $($result.ExitCode) instead of 1."
            Assert-Condition ($status.state -eq 'inactive') "direct PowerShell wardoff --status returned state '$($status.state)' instead of 'inactive'."
            Assert-Condition ($result.Output -notmatch '(?i)elevation|required administrator rights|failed to start') "direct PowerShell wardoff --status reported an unexpected startup or elevation failure: $($result.Output)"
        }
    }

    if ($isAdmin) {
        Skip-TestCase 'wardoff --status from a non-elevated session does not spawn another Wardoff process' 'Current smoke script is already elevated, so the required non-elevated no-spawn check is unavailable.'
    }
    else {
        Invoke-TestCase 'wardoff --status from a non-elevated session does not spawn another Wardoff process' {
            Stop-RepoWardoffProcesses

            $statusResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--status')
            $status = Get-StatusJson -JsonText $statusResult.Output
            $trackingResult = Invoke-ExternalCommandWithRepoProcessTracking -FilePath $binaryPath -Arguments @('--status')
            $launchedProcessId = $trackingResult.LaunchedProcessId
            $initialWardoffProcessIds = @($trackingResult.InitialWardoffProcessIds)
            $finalWardoffProcessIds = @($trackingResult.FinalWardoffProcessIds)
            $unexpectedWardoffProcessIds = @(
                $trackingResult.ObservedWardoffProcessIds |
                    Where-Object {
                        ($_ -ne $launchedProcessId) -and
                        ($_ -notin $initialWardoffProcessIds)
                    } |
                    Sort-Object -Unique
            )
            $unexpectedProcessIds = @(
                $trackingResult.ObservedRepoProcessIds |
                    Where-Object { $_ -ne $launchedProcessId } |
                    Sort-Object -Unique
            )

            Assert-Condition ($statusResult.ExitCode -eq 1) "wardoff --status exited with code $($statusResult.ExitCode) instead of 1."
            Assert-Condition ($status.state -eq 'inactive') "wardoff --status returned state '$($status.state)' instead of 'inactive'."
            Assert-Condition ($unexpectedWardoffProcessIds.Count -eq 0) "wardoff --status spawned additional wardoff.exe process id(s): $($unexpectedWardoffProcessIds -join ', ')."
            Assert-Condition ($unexpectedProcessIds.Count -eq 0) "wardoff --status spawned additional Wardoff process id(s): $($unexpectedProcessIds -join ', ')."
            Assert-Condition (@($finalWardoffProcessIds | Where-Object { $_ -notin $initialWardoffProcessIds }).Count -eq 0) "wardoff --status left new wardoff.exe process id(s) running after it exited: $((@($finalWardoffProcessIds | Where-Object { $_ -notin $initialWardoffProcessIds }) | Sort-Object -Unique) -join ', ')."
            Assert-Condition (@(Get-RepoWardoffProcesses).Count -eq 0) 'wardoff --status left a Wardoff process running after it exited.'
        }
    }

    Invoke-TestCase 'wardoff --status without an instance exits 1 and reports inactive JSON' {
        Stop-RepoWardoffProcesses
        $result = Wait-ForInactiveWardoffStatus
        $status = Get-StatusJson -JsonText $result.Output

        Assert-Condition ($result.ExitCode -eq 1) "wardoff --status exited with code $($result.ExitCode) instead of 1."
        Assert-Condition ($status.state -eq 'inactive') "wardoff --status returned state '$($status.state)' instead of 'inactive'."
    }

    Invoke-TestCase 'Start-Process launches wardoff --block --hide in the background' {
        Stop-RepoWardoffProcesses
        $script:InitialLogLineCount = @(Get-StructuredLogLines).Count
        Start-SmokeRuntime
        Start-Sleep -Seconds 2
        $script:BackgroundProcess.Refresh()
        Assert-Condition (-not $script:BackgroundProcess.HasExited) "Wardoff exited early with code $($script:BackgroundProcess.ExitCode)."
    }

    Invoke-TestCase 'wardoff --status reports valid JSON with state block while the background instance is running' {
        $activeStatus = Wait-ForActiveWardoffStatus
        $result = $activeStatus.Result
        $status = $activeStatus.Status

        Assert-Condition ($result.ExitCode -eq 0) "wardoff --status exited with code $($result.ExitCode) while Wardoff was running."
        Assert-Condition ($status.state -eq 'block') "wardoff --status returned state '$($status.state)' instead of 'block'."
        Assert-Condition ($status.layers.sleep -eq $true) 'Block mode did not acquire both idle-power requests.'
        Assert-Condition ($null -ne $status.layers) 'wardoff --status did not include a layers object while Wardoff was running.'
        Assert-Condition ($null -ne $status.layers.local_shutdown) 'wardoff --status did not include the layers.local_shutdown field.'
        Assert-Condition ($status.layers.local_shutdown -is [bool]) 'wardoff --status returned a non-boolean layers.local_shutdown field.'
    }

    Invoke-TestCase 'wardoff --log --tail 3 exits 0 and returns 3 JSON log lines while the background instance is running' {
        $result = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--log', '--tail', '3')
        Assert-Condition ($result.ExitCode -eq 0) "wardoff --log --tail 3 exited with code $($result.ExitCode)."

        $lines = @($result.Output -split "`r?`n" | Where-Object { $_ -ne '' })
        Assert-Condition ($lines.Count -eq 3) "wardoff --log --tail 3 returned $($lines.Count) line(s) instead of 3."

        foreach ($line in $lines) {
            try {
                $null = $line | ConvertFrom-Json -ErrorAction Stop
            }
            catch {
                throw "wardoff --log --tail 3 returned a non-JSON line: $line"
            }
        }
    }

    $powercfgRequestsCheck = Resolve-PowercfgRequestsCheck
    if ($powercfgRequestsCheck.State -eq 'testable') {
        Invoke-TestCase 'powercfg /requests mentions Wardoff while blocking is active' {
            $result = $powercfgRequestsCheck.Result
            $reason = 'Wardoff Block mode: prevent idle sleep and automatic display timeout.'
            Assert-Condition ([regex]::Matches($result.Output, [regex]::Escape($reason)).Count -eq 2) 'Expected the Wardoff reason once for SYSTEM and once for DISPLAY.'
        }
    }
    elseif ($powercfgRequestsCheck.State -eq 'skip') {
        Skip-TestCase 'powercfg /requests mentions Wardoff while blocking is active' $powercfgRequestsCheck.SkipReason
    }
    else {
        Add-TestResult -Status 'FAIL' -Description 'powercfg /requests mentions Wardoff while blocking is active' -Details $powercfgRequestsCheck.FailureMessage
    }

    Invoke-TestCase '$env:LOCALAPPDATA\Wardoff\logs\wardoff.jsonl exists and contains valid JSON lines' {
        Assert-Condition (Test-Path $logPath) "Expected log file at $logPath."

        $lines = @(Get-StructuredLogLines)
        Assert-Condition ($lines.Count -gt 0) "Log file $logPath was empty."
        Assert-Condition ($lines.Count -gt $script:InitialLogLineCount) "Wardoff did not append any new structured log lines during this run."

        $hasValidJson = $false
        foreach ($line in $lines[$script:InitialLogLineCount..($lines.Count - 1)]) {
            try {
                $null = $line | ConvertFrom-Json -ErrorAction Stop
                $hasValidJson = $true
                break
            }
            catch {
            }
        }

        Assert-Condition $hasValidJson "Log file $logPath did not contain a valid JSON line."
    }

    if ($isAdmin) {
        $tokenElevationType = $null
        try {
            $tokenElevationType = Get-CurrentTokenElevationType
            Write-Host "Observed native TokenElevationType: $tokenElevationType" -ForegroundColor Cyan
        }
        catch {
            Add-TestResult -Status 'FAIL' -Description 'native TokenElevationType query succeeds in the elevated smoke session' -Details $_.Exception.Message
        }

        $layer3Description = 'wardoff --status reports Layer 3 UpdateOrchestrator protection as active when run as admin'
        try {
            if (Test-RebootTaskPresent) {
                Invoke-TestCase $layer3Description {
                    $activeStatus = Wait-ForActiveWardoffStatus
                    Assert-Condition ($activeStatus.Status.layers.update -eq $true) 'wardoff --status did not report layers.update=true in an elevated session.'
                }
            }
            else {
                Skip-TestCase $layer3Description 'The UpdateOrchestrator Reboot task is absent on this Windows installation.'
            }
        }
        catch {
            Add-TestResult -Status 'FAIL' -Description $layer3Description -Details $_.Exception.Message
        }

        Invoke-TestCase 'wardoff --status reports Layer 4 AbortSystemShutdown protection as active when run as admin' {
            $activeStatus = Wait-ForActiveWardoffStatus
            Assert-Condition ($activeStatus.Status.layers.remote -eq $true) 'wardoff --status did not report layers.remote=true in an elevated session.'
        }

        Invoke-TestCase 'schtasks /query for Microsoft\Windows\UpdateOrchestrator\Reboot completes without crashing the script' {
            $null = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Microsoft\Windows\UpdateOrchestrator\Reboot')
        }

        $autostartDescription = 'Autostart trust-boundary behavior leaves task state verified'
        $autostartDecision = Get-AutostartValidationDecision $tokenElevationType
        if ($autostartDecision -eq 'query-failed') {
            Invoke-TestCase $autostartDescription {
                throw 'Cannot classify the elevated token because the native TokenElevationType query failed.'
            }
        }
        elseif ($autostartDecision -eq 'skip-no-linked-token') {
            Skip-TestCase $autostartDescription 'Skipped: native TokenElevationTypeDefault (1) means this elevated token has no linked token; validate the autostart lifecycle manually from an interactive UAC session.'
        }
        elseif ($autostartDecision -eq 'fail-unexpected-value') {
            Invoke-TestCase $autostartDescription {
                throw "Unexpected TokenElevationType value $tokenElevationType in the elevated smoke session; interactive UAC validation is required."
            }
        }
        else {
            Invoke-TestCase $autostartDescription {
            Start-SmokeAutostartAttempt -State $script:ResourceState

            try {
                try { $enableResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'on') }
                finally { Capture-SmokeAutostartOwnership -State $script:ResourceState }
                if ($enableResult.ExitCode -eq 0) {
                    $queryEnabled = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')
                    Assert-Condition ($queryEnabled.ExitCode -eq 0) 'schtasks /query /tn "Wardoff" did not find the task after --autostart on.'

                    Capture-SmokeAutostartOwnership -State $script:ResourceState
                    $disableResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'off')
                    Assert-Condition ($disableResult.ExitCode -eq 0) "wardoff --autostart off exited with code $($disableResult.ExitCode)."

                    $queryDisabled = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')
                    Assert-Condition ($queryDisabled.ExitCode -ne 0) 'schtasks /query /tn "Wardoff" still found the task after --autostart off.'
                    $script:AutostartLifecyclePassed = $true
                    Add-TestResult -Status 'PASS' -Description 'Positive autostart on/off lifecycle' -Details ''
                }
                else {
                    Assert-Condition ($enableResult.Output -match 'untrusted location') "wardoff --autostart on failed without the expected trusted-location refusal. Output: $($enableResult.Output)"

                    $queryAfterRefusal = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')
                    Assert-Condition ($queryAfterRefusal.ExitCode -ne 0) 'wardoff --autostart on changed the Wardoff task state even though it refused an untrusted executable location.'
                    Skip-TestCase 'Positive autostart on/off lifecycle' 'Only trusted-location refusal was tested; a protected authorized VM path is required for positive coverage.'
                }
            }
            finally {
                Capture-SmokeAutostartOwnership -State $script:ResourceState
            }
            }
        }
    }
    else {
        $adminOnlyMessage = 'Run as admin for full test coverage'
        Write-Host $adminOnlyMessage -ForegroundColor Yellow
        Skip-TestCase 'wardoff --status reports Layer 3 UpdateOrchestrator protection as active when run as admin' $adminOnlyMessage
        Skip-TestCase 'wardoff --status reports Layer 4 AbortSystemShutdown protection as active when run as admin' $adminOnlyMessage
        Skip-TestCase 'schtasks /query for Microsoft\Windows\UpdateOrchestrator\Reboot completes without crashing the script' $adminOnlyMessage
        Skip-TestCase 'wardoff --autostart on creates the Wardoff task and --autostart off removes it' $adminOnlyMessage
    }

    Invoke-TestCase 'Allow releases idle-power requests without stopping the runtime' {
        $repoProcesses = @(Get-RepoWardoffProcesses)
        Assert-Condition ($repoProcesses.Count -eq 1) 'Expected one background Wardoff process.'
        $allowResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--allow')
        Assert-Condition ($allowResult.ExitCode -eq 0) "wardoff --allow exited with code $($allowResult.ExitCode)."
        $statusResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--status')
        Assert-Condition ($statusResult.ExitCode -eq 0) 'Status failed after Allow.'
        $status = $statusResult.Output | ConvertFrom-Json
        Assert-Condition ($status.state -eq 'allow' -and $status.layers.sleep -eq $false) 'Allow did not report released idle-power requests.'
        $remaining = @(Get-RepoWardoffProcesses)
        Assert-Condition ($remaining.Count -eq 1 -and $remaining[0].ProcessId -eq $repoProcesses[0].ProcessId) 'Allow replaced or stopped the runtime.'
    }

    $allowRequestsCheck = Resolve-PowercfgRequestsCheck
    if ($allowRequestsCheck.State -eq 'testable') {
        Invoke-TestCase 'powercfg /requests has no Wardoff entry in Allow mode' {
            Assert-Condition ($allowRequestsCheck.Result.Output -notmatch '(?i)wardoff') 'Wardoff requests remained after Allow.'
        }
    }
    elseif ($allowRequestsCheck.State -eq 'skip') {
        Skip-TestCase 'powercfg /requests has no Wardoff entry in Allow mode' $allowRequestsCheck.SkipReason
    }
    else {
        Add-TestResult -Status 'FAIL' -Description 'powercfg /requests has no Wardoff entry in Allow mode' -Details $allowRequestsCheck.FailureMessage
    }

    Invoke-TestCase 'Repeated Block reacquires idle-power requests before abrupt process termination' {
        foreach ($iteration in 1..2) {
            $blockResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--block')
            Assert-Condition ($blockResult.ExitCode -eq 0) 'Block failed after Allow.'
            $activeStatus = Wait-ForActiveWardoffStatus
            Assert-Condition ($activeStatus.Status.layers.sleep -eq $true) 'Block did not reacquire idle-power requests.'
        }
        # Deliberately do not call the normal cleanup helper: it sends --allow first.
        Assert-SmokeOwnedRuntime
        Capture-SmokeOwnedEtw -State $script:ResourceState -Process $script:BackgroundProcess
        $ownedProcess = $script:BackgroundProcess
        Stop-Process -InputObject $ownedProcess -Force -ErrorAction Stop
        Assert-Condition ($ownedProcess.WaitForExit(5000)) 'The terminated runtime did not exit within five seconds.'
        Assert-Condition (@(Get-RepoWardoffProcesses).Count -eq 0) 'Wardoff was still running after cleanup.'
    }

    $powercfgRequestsCheck = Resolve-PowercfgRequestsCheck
    if ($powercfgRequestsCheck.State -eq 'testable') {
        Invoke-TestCase 'powercfg /requests no longer mentions Wardoff after cleanup' {
            $requestCleared = $false
            for ($attempt = 0; $attempt -lt 5; $attempt++) {
                $result = Get-PowercfgRequests
                Assert-Condition ($result.ExitCode -eq 0) "powercfg /requests exited with code $($result.ExitCode)."

                if ($result.Output -notmatch '(?i)wardoff') {
                    $requestCleared = $true
                    break
                }

                Start-Sleep -Seconds 1
            }

            Assert-Condition $requestCleared 'powercfg /requests still mentioned Wardoff after cleanup.'
        }
    }
    elseif ($powercfgRequestsCheck.State -eq 'skip') {
        Skip-TestCase 'powercfg /requests no longer mentions Wardoff after cleanup' $powercfgRequestsCheck.SkipReason
    }
    else {
        Add-TestResult -Status 'FAIL' -Description 'powercfg /requests no longer mentions Wardoff after cleanup' -Details $powercfgRequestsCheck.FailureMessage
    }
}
catch {
    Add-TestResult -Status 'FAIL' -Description 'Smoke harness aborted' -Details $_.Exception.Message
}
finally {
    try { Stop-RepoWardoffProcesses } catch { Add-TestResult -Status 'FAIL' -Description 'Owned process cleanup' -Details $_.Exception.Message }
    $runtimeExited=$true
    foreach($p in $script:OwnedProcesses.ToArray()) {
        try { if(-not $p.HasExited){$runtimeExited=$false} } catch {$runtimeExited=$false}
    }
    try { Restore-SmokeResources -State $script:ResourceState -RuntimeExited:$runtimeExited -OwnedProcesses $script:OwnedProcesses.ToArray() }
    catch { Add-TestResult -Status 'FAIL' -Description 'Resource restoration' -Details $_.Exception.Message }
    if ($RequireAutostartLifecycle -and -not $script:AutostartLifecyclePassed) {
        Add-TestResult -Status 'FAIL' -Description 'Required positive autostart lifecycle' -Details 'Positive on/off coverage did not pass; a skip or trusted-location refusal does not satisfy it.'
    }
    try {
        if($null -ne $script:BinaryLock) {
            $script:BinaryLock.Position=0;$hash=[Security.Cryptography.SHA256]::Create()
            try {$finalHash=[BitConverter]::ToString($hash.ComputeHash($script:BinaryLock)).Replace('-','').ToLowerInvariant()}finally{$hash.Dispose()}
            Assert-Condition ($finalHash -ceq $ExpectedBinarySha256.ToLowerInvariant()) 'Binary bytes changed during smoke.'
        }
        foreach($task in @($script:RuntimeOutput,$script:RuntimeError)){if($null -ne $task){Assert-Condition ($task.Wait(2000)) 'Runtime output drain did not complete.'}}
    } catch { Add-TestResult -Status 'FAIL' -Description 'Binary and output finalization' -Details $_.Exception.Message }
    finally {
        if($null -ne $script:BinaryLock){$script:BinaryLock.Dispose()}
        foreach($p in $script:OwnedProcesses.ToArray()){$p.Dispose()}
    }
    [ordered]@{schema=1;scope='NATIVE_SMOKE_ISOLATED_TARGET';binary_path=$binaryPath;expected_binary_sha256=$ExpectedBinarySha256;
        runtime_cleanup_complete=$runtimeExited;autostart_lifecycle_passed=$script:AutostartLifecyclePassed;results=@($script:Results);qualification='LIMITED_SMOKE_ONLY';physical_acceptance=$false} |
        ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'smoke-result.json') -Encoding utf8

    $passed = @($script:Results | Where-Object { $_.CountInSummary -and $_.Status -eq 'PASS' }).Count
    $counted = @($script:Results | Where-Object { $_.CountInSummary }).Count
    $skipped = @($script:Results | Where-Object { $_.Status -eq 'SKIP' }).Count

    if ($skipped -gt 0) {
        Write-Host "$passed/$counted tests passed ($skipped skipped)"
    }
    else {
        Write-Host "$passed/$counted tests passed"
    }

}
if (@($script:Results | Where-Object { $_.Status -eq 'FAIL' }).Count -gt 0) { exit 1 }
