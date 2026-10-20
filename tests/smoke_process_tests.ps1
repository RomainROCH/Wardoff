# These tests launch only this script's inert PowerShell child modes.
# They never launch Wardoff, Cargo, powercfg, Task Scheduler or ETW operations.
[CmdletBinding()]
param(
    [ValidateSet('', 'Echo', 'Exit37', 'Burst', 'Wait')][string] $ChildMode = '',
    [AllowEmptyString()][string] $TestValue = ''
)

if ($ChildMode -ne '') {
    switch ($ChildMode) {
        'Echo' {
            [Console]::Out.Write((@{ Value = $TestValue; Marker = $env:WARDOFF_SMOKE_CHILD_TEST } | ConvertTo-Json -Compress))
            exit 0
        }
        'Exit37' {
            [Console]::Out.Write('expected-out')
            [Console]::Error.Write('expected-error')
            exit 37
        }
        'Burst' {
            [Console]::Out.Write(('o' * 100000))
            [Console]::Error.Write(('e' * 100000))
            exit 0
        }
        'Wait' {
            [Console]::Out.Write('started-out')
            [Console]::Error.Write('started-err')
            Start-Sleep -Seconds 30
            exit 0
        }
    }
}

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'These inert process tests require PowerShell 7.' }
. (Join-Path $PSScriptRoot 'smoke_process.ps1')

$pwsh = [Environment]::ProcessPath
$testScript = $PSCommandPath
$baseArguments = @('-NoProfile', '-NonInteractive', '-File', $testScript)
$workingDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$passed = [Collections.Generic.List[string]]::new()

function Assert-ProcessTest([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
}

function Invoke-InertChild {
    param([string] $Mode, [string] $Value = '', [hashtable] $Overrides = @{}, [int] $Timeout = 15000,
        [scriptblock] $Started = $null, [scriptblock] $Poll = $null, [int] $Limit = 1048576)
    Invoke-SmokeChildCommand -FilePath $pwsh -Arguments ($baseArguments + @('-ChildMode', $Mode, '-TestValue', $Value)) `
        -WorkingDirectory $workingDirectory -Environment $Overrides -TimeoutMilliseconds $Timeout `
        -OnStarted $Started -OnPoll $Poll -MaxOutputCharacters $Limit
}

$value = 'spaces "quotes" apostrophe'' $literal `tick \ and Unicode é Ω'
$parentMarker = $env:WARDOFF_SMOKE_CHILD_TEST
try {
    $env:WARDOFF_SMOKE_CHILD_TEST = 'parent-marker'
    $echo = Invoke-InertChild -Mode Echo -Value $value -Overrides @{ WARDOFF_SMOKE_CHILD_TEST = 'child-marker' }
    $decoded = $echo.StandardOutput | ConvertFrom-Json
    Assert-ProcessTest ($echo.ExitCode -eq 0 -and $decoded.Value -ceq $value) 'Quoted or Unicode arguments changed.'
    Assert-ProcessTest ($decoded.Marker -ceq 'child-marker') 'The child environment override was absent.'
    Assert-ProcessTest ($env:WARDOFF_SMOKE_CHILD_TEST -ceq 'parent-marker') 'The helper changed the parent environment.'
    $passed.Add('quoted-arguments-and-isolated-environment')
    $empty = Invoke-InertChild -Mode Echo -Value ''
    Assert-ProcessTest (($empty.StandardOutput | ConvertFrom-Json).Value -ceq '') 'An empty argument changed.'
    $passed.Add('empty-argument')
}
finally {
    $env:WARDOFF_SMOKE_CHILD_TEST = $parentMarker
}

$nonzero = Invoke-InertChild -Mode Exit37
Assert-ProcessTest ($nonzero.ExitCode -eq 37) 'The expected nonzero child exit was not retained.'
Assert-ProcessTest ($nonzero.StandardOutput -ceq 'expected-out' -and $nonzero.StandardError -ceq 'expected-error') 'One redirected stream was lost.'
$passed.Add('native-nonzero-and-both-streams')

$burst = Invoke-InertChild -Mode Burst
Assert-ProcessTest ($burst.ExitCode -eq 0 -and $burst.StandardOutput.Length -eq 100000 -and $burst.StandardError.Length -eq 100000) 'Concurrent output drain lost data or deadlocked.'
$passed.Add('concurrent-bounded-output-drain')

$retained = @{ Process = $null }
try {
    $retainedResult = Invoke-SmokeChildCommand -FilePath $pwsh -Arguments ($baseArguments + @('-ChildMode', 'Exit37')) `
        -WorkingDirectory $workingDirectory -RetainProcess -OnStarted {
            param($child)
            $retained.Process = $child
        }
    Assert-ProcessTest ($retainedResult.ExitCode -eq 37 -and $retained.Process.HasExited) 'A retained Process was not observable after return.'
    Assert-ProcessTest ($retained.Process.Id -eq $retainedResult.LaunchedProcessId -and $retained.Process.Handle -ne [IntPtr]::Zero) 'The retained Process lost its original handle or identity.'
    $passed.Add('retained-process-handle-owned-by-caller')
}
finally { if ($null -ne $retained.Process) { $retained.Process.Dispose() } }

$foreign = [Diagnostics.Process]::new()
$foreign.StartInfo = [Diagnostics.ProcessStartInfo]::new()
$foreign.StartInfo.FileName = $pwsh
$foreign.StartInfo.WorkingDirectory = $workingDirectory
$foreign.StartInfo.UseShellExecute = $false
$foreign.StartInfo.CreateNoWindow = $true
foreach ($argument in ($baseArguments + @('-ChildMode', 'Wait'))) { $foreign.StartInfo.ArgumentList.Add($argument) }
$observer = $null
try {
    [void]$foreign.Start()
    $null = $foreign.Handle
    $state = @{ Observer = $null }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $caught = $null
    try {
        $null = Invoke-InertChild -Mode Wait -Timeout 1500 -Started {
            param($child)
            $state.Observer = [Diagnostics.Process]::GetProcessById($child.Id)
            $null = $state.Observer.Handle
        }
    }
    catch { $caught = $_.Exception }
    $observer = $state.Observer
    $watch.Stop()
    Assert-ProcessTest ($null -ne $caught -and $caught.Message -match 'timed out') 'Timeout did not fail explicitly.'
    Assert-ProcessTest ($watch.ElapsedMilliseconds -lt 8000) 'Timeout or reap was not bounded.'
    Assert-ProcessTest ($null -ne $observer -and $observer.HasExited) 'The timed-out owned child was not reaped.'
    Assert-ProcessTest ([string]$caught.Data['StandardOutput'] -match 'started-out') 'Timeout lost captured stdout.'
    Assert-ProcessTest ([string]$caught.Data['StandardError'] -match 'started-err') 'Timeout lost captured stderr.'
    Assert-ProcessTest (-not $foreign.HasExited) 'Cleanup stopped a process it did not own.'
    $passed.Add('timeout-reap-and-foreign-process-preserved')
}
finally {
    if ($null -ne $observer) { $observer.Dispose() }
    if (-not $foreign.HasExited) { $foreign.Kill($true); [void]$foreign.WaitForExit(2000) }
    $foreign.Dispose()
}

foreach ($case in @('capture-limit', 'callback-failure')) {
    $state = @{ Observer = $null }
    $caught = $null
    try {
        $mode = if ($case -eq 'capture-limit') { 'Burst' } else { 'Wait' }
        $poll = if ($case -eq 'callback-failure') { { throw 'expected-poll-failure' } } else { $null }
        $limit = if ($case -eq 'capture-limit') { 8192 } else { 1048576 }
        $null = Invoke-InertChild -Mode $mode -Limit $limit -Poll $poll -Started {
            param($child)
            $state.Observer = [Diagnostics.Process]::GetProcessById($child.Id)
            $null = $state.Observer.Handle
        }
    }
    catch { $caught = $_.Exception }
    try {
        $pattern = if ($case -eq 'capture-limit') { 'capture limit' } else { 'expected-poll-failure' }
        Assert-ProcessTest ($null -ne $caught -and $caught.Message -match $pattern) "$case did not fail explicitly."
        Assert-ProcessTest ($null -ne $state.Observer -and $state.Observer.HasExited) "$case left its owned child alive."
        $passed.Add($case)
    }
    finally { if ($null -ne $state.Observer) { $state.Observer.Dispose() } }
}

[pscustomobject]@{ Status = 'PASS_INERT_PROCESS_TESTS'; Cases = $passed.Count; Passed = @($passed.ToArray()); RealWardoffExecuted = $false } | ConvertTo-Json -Depth 4
