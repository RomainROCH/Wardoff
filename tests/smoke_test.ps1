[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Results = [System.Collections.Generic.List[object]]::new()
$script:BackgroundProcess = $null
$script:InitialLogLineCount = 0

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$logPath = Join-Path (Join-Path $env:LOCALAPPDATA 'Wardoff') 'logs\wardoff.jsonl'

function Resolve-BinaryPath {
    $defaultBinaryPath = Join-Path $repoRoot 'target\release\wardoff.exe'

    try {
        $metadataJson = & cargo metadata --format-version 1 --no-deps 2>$null
        if (-not [string]::IsNullOrWhiteSpace($metadataJson)) {
            $metadata = $metadataJson | ConvertFrom-Json -ErrorAction Stop
            if (-not [string]::IsNullOrWhiteSpace($metadata.target_directory)) {
                return Join-Path $metadata.target_directory 'release\wardoff.exe'
            }
        }
    }
    catch {
    }

    return $defaultBinaryPath
}

$binaryPath = Resolve-BinaryPath

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
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
    param(
        [Parameter(Mandatory = $true)]
        [string] $FilePath,
        [string[]] $Arguments = @(),
        [string] $WorkingDirectory = $repoRoot
    )

    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()

    try {
        $process = Start-Process -FilePath $FilePath `
            -ArgumentList $Arguments `
            -WorkingDirectory $WorkingDirectory `
            -Wait `
            -PassThru `
            -NoNewWindow `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath

        $stdout = if (Test-Path $stdoutPath) {
            Get-Content -Path $stdoutPath -Raw -ErrorAction SilentlyContinue
        }
        else {
            ''
        }
        $stderr = if (Test-Path $stderrPath) {
            Get-Content -Path $stderrPath -Raw -ErrorAction SilentlyContinue
        }
        else {
            ''
        }
        $exitCode = $process.ExitCode
    }
    catch {
        throw "Failed to run '$FilePath': $($_.Exception.Message)"
    }
    finally {
        Remove-Item -Path $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }

    $stdout = if ($null -eq $stdout) { '' } else { [string]$stdout }
    $stderr = if ($null -eq $stderr) { '' } else { [string]$stderr }
    $stdout = [string] $stdout
    $stderr = [string] $stderr
    $stdout = $stdout.Trim()
    $stderr = $stderr.Trim()
    $text = @($stdout, $stderr) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $text = ($text -join [Environment]::NewLine).Trim()
    return [pscustomobject]@{
        ExitCode       = $exitCode
        StandardOutput = $stdout
        StandardError  = $stderr
        Output         = $text
    }
}

function Invoke-ExternalCommandWithRepoProcessTracking {
    param(
        [Parameter(Mandatory = $true)]
        [string] $FilePath,
        [string[]] $Arguments = @(),
        [string] $WorkingDirectory = $repoRoot,
        [int] $PollIntervalMilliseconds = 25,
        [int] $PostExitGraceMilliseconds = 250
    )

    $observedRepoProcessIds = [System.Collections.Generic.HashSet[int]]::new()
    $observedWardoffProcessIds = [System.Collections.Generic.HashSet[int]]::new()
    $initialWardoffProcessIds = @(
        Get-WardoffProcessesByName |
            ForEach-Object { [int] $_.ProcessId } |
            Sort-Object -Unique
    )

    try {
        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $FilePath
        $startInfo.Arguments = @(
            $Arguments | ForEach-Object {
                $argumentText = [string] $_
                if ($argumentText -match '[\s"]') {
                    '"' + ($argumentText -replace '"', '\"') + '"'
                }
                else {
                    $argumentText
                }
            }
        ) -join ' '
        $startInfo.WorkingDirectory = $WorkingDirectory
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true

        $process = [System.Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        $null = $process.Start()

        do {
            $process.Refresh()

            foreach ($wardoffProcess in @(Get-WardoffProcessesByName)) {
                $null = $observedWardoffProcessIds.Add([int] $wardoffProcess.ProcessId)
            }

            foreach ($repoProcess in @(Get-RepoWardoffProcesses)) {
                $null = $observedRepoProcessIds.Add([int] $repoProcess.ProcessId)
            }

            if ($process.HasExited) {
                break
            }

            Start-Sleep -Milliseconds $PollIntervalMilliseconds
        }
        while ($true)

        $null = $process.WaitForExit()
        $process.Refresh()

        $extraPolls = [Math]::Max(1, [int] [Math]::Ceiling($PostExitGraceMilliseconds / [Math]::Max(1, $PollIntervalMilliseconds)))
        for ($poll = 0; $poll -lt $extraPolls; $poll++) {
            Start-Sleep -Milliseconds $PollIntervalMilliseconds
            foreach ($wardoffProcess in @(Get-WardoffProcessesByName)) {
                $null = $observedWardoffProcessIds.Add([int] $wardoffProcess.ProcessId)
            }
            foreach ($repoProcess in @(Get-RepoWardoffProcesses)) {
                $null = $observedRepoProcessIds.Add([int] $repoProcess.ProcessId)
            }
        }

        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $exitCode = $process.ExitCode
    }
    catch {
        throw "Failed to run '$FilePath' with process tracking: $($_.Exception.Message)"
    }

    $stdout = if ($null -eq $stdout) { '' } else { [string] $stdout }
    $stderr = if ($null -eq $stderr) { '' } else { [string] $stderr }
    $stdout = $stdout.Trim()
    $stderr = $stderr.Trim()
    $text = @($stdout, $stderr) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $text = ($text -join [Environment]::NewLine).Trim()

    return [pscustomobject]([ordered]@{
        ExitCode                 = $exitCode
        StandardOutput           = $stdout
        StandardError            = $stderr
        Output                   = $text
        LaunchedProcessId        = $process.Id
        InitialWardoffProcessIds = $initialWardoffProcessIds
        FinalWardoffProcessIds   = @(
            Get-WardoffProcessesByName |
                ForEach-Object { [int] $_.ProcessId } |
                Sort-Object -Unique
        )
        ObservedWardoffProcessIds = @($observedWardoffProcessIds | Sort-Object)
        ObservedRepoProcessIds    = @($observedRepoProcessIds | Sort-Object)
    })
}

function Get-WardoffProcessesByName {
    $processes = Get-CimInstance Win32_Process -Filter "Name='wardoff.exe'" -ErrorAction SilentlyContinue

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
    $processes = @(Get-RepoWardoffProcesses)

    if ($processes.Count -gt 0 -and (Test-Path $binaryPath)) {
        try {
            $null = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--allow')
            Start-Sleep -Seconds 1
        }
        catch {
        }
    }

    foreach ($process in $processes) {
        try {
            Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
        }
        catch {
            if ($_.Exception.Message -notmatch 'cannot find') {
                Write-Warning "Could not stop Wardoff process $($process.ProcessId): $($_.Exception.Message)"
            }
        }
    }

    foreach ($process in $processes) {
        try {
            Wait-Process -Id $process.ProcessId -Timeout 5 -ErrorAction Stop
        }
        catch {
        }
    }

    if (Test-Path $binaryPath) {
        Wait-ForBinaryUnlock -Path $binaryPath
    }
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
    Stop-RepoWardoffProcesses

    Invoke-TestCase 'cargo build --release succeeds' {
        $result = Invoke-ExternalCommand -FilePath 'cargo' -Arguments @('build', '--release')
        Assert-Condition ($result.ExitCode -eq 0) "cargo build --release failed with exit code $($result.ExitCode)."
    }

    Invoke-TestCase 'target\release\wardoff.exe exists after the release build' {
        Assert-Condition (Test-Path $binaryPath) "Expected binary at $binaryPath."
    }

    Invoke-TestCase 'target\release\wardoff.exe embeds an asInvoker manifest' {
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
        $script:BackgroundProcess = Start-Process -FilePath $binaryPath -ArgumentList @('--block', '--hide') -WorkingDirectory $repoRoot -PassThru
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
            Assert-Condition ($result.Output -match '(?i)wardoff') 'powercfg /requests did not mention Wardoff.'
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
        Invoke-TestCase 'wardoff --status reports Layer 3 UpdateOrchestrator protection as active when run as admin' {
            $activeStatus = Wait-ForActiveWardoffStatus
            Assert-Condition ($activeStatus.Status.layers.update -eq $true) 'wardoff --status did not report layers.update=true in an elevated session.'
        }

        Invoke-TestCase 'wardoff --status reports Layer 4 AbortSystemShutdown protection as active when run as admin' {
            $activeStatus = Wait-ForActiveWardoffStatus
            Assert-Condition ($activeStatus.Status.layers.remote -eq $true) 'wardoff --status did not report layers.remote=true in an elevated session.'
        }

        Invoke-TestCase 'schtasks /query for Microsoft\Windows\UpdateOrchestrator\Reboot completes without crashing the script' {
            $null = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Microsoft\Windows\UpdateOrchestrator\Reboot')
        }

        Invoke-TestCase 'wardoff --autostart on creates the Wardoff task and --autostart off removes it' {
            $taskOriginallyPresent = (Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')).ExitCode -eq 0

            try {
                $enableResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'on')
                Assert-Condition ($enableResult.ExitCode -eq 0) "wardoff --autostart on exited with code $($enableResult.ExitCode)."

                $queryEnabled = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')
                Assert-Condition ($queryEnabled.ExitCode -eq 0) 'schtasks /query /tn "Wardoff" did not find the task after --autostart on.'

                $disableResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'off')
                Assert-Condition ($disableResult.ExitCode -eq 0) "wardoff --autostart off exited with code $($disableResult.ExitCode)."

                $queryDisabled = Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')
                Assert-Condition ($queryDisabled.ExitCode -ne 0) 'schtasks /query /tn "Wardoff" still found the task after --autostart off.'
            }
            finally {
                try {
                    $taskPresentAfterTest = (Invoke-ExternalCommand -FilePath 'schtasks' -Arguments @('/query', '/tn', 'Wardoff')).ExitCode -eq 0
                    if ($taskOriginallyPresent -and -not $taskPresentAfterTest) {
                        $null = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'on')
                    }
                    elseif (-not $taskOriginallyPresent -and $taskPresentAfterTest) {
                        $null = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--autostart', 'off')
                    }
                }
                catch {
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

    Invoke-TestCase 'The background Wardoff instance can be stopped and cleaned up' {
        $repoProcesses = @(Get-RepoWardoffProcesses)
        Assert-Condition ($repoProcesses.Count -gt 0) 'No background Wardoff process was running to stop.'

        try {
            $allowResult = Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--allow')
            Assert-Condition ($allowResult.ExitCode -eq 0) "wardoff --allow exited with code $($allowResult.ExitCode)."
        }
        catch {
        }

        Stop-RepoWardoffProcesses
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
finally {
    Stop-RepoWardoffProcesses

    $passed = @($script:Results | Where-Object { $_.CountInSummary -and $_.Status -eq 'PASS' }).Count
    $counted = @($script:Results | Where-Object { $_.CountInSummary }).Count
    $skipped = @($script:Results | Where-Object { $_.Status -eq 'SKIP' }).Count

    if ($skipped -gt 0) {
        Write-Host "$passed/$counted tests passed ($skipped skipped)"
    }
    else {
        Write-Host "$passed/$counted tests passed"
    }

    if (@($script:Results | Where-Object { $_.Status -eq 'FAIL' }).Count -gt 0) {
        exit 1
    }
}
