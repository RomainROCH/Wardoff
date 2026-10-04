[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot 'smoke_helpers.ps1'
. $helperPath

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Assert-Throws {
    param([scriptblock]$Action, [string]$MessageFragment)
    $caught = $null
    try {
        & $Action
    }
    catch {
        $caught = $_.Exception.Message
    }
    if ($null -eq $caught) {
        throw "Expected an exception containing '$MessageFragment'."
    }
    if ($caught -notmatch [regex]::Escape($MessageFragment)) {
        throw "Unexpected exception: $caught"
    }
}

try {
    $binaryPath = Join-Path $env:TEMP 'Wardoff\target\release\wardoff.exe'
    $canaryCaught = $false
    try { Assert-Throws {} 'canary' } catch { $canaryCaught = $true }
    Assert-True $canaryCaught 'Assert-Throws accepted an action that did not throw.'

    $probeState = @{ Calls = 0 }
    $emptyProbe = { $probeState.Calls++; return $false }
    Assert-SmokePreconditions -WardoffProcesses @() -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe $emptyProbe
    Assert-True ($probeState.Calls -eq 1) 'The autostart probe was not called for a clean fixture.'

    $sameSession = [pscustomobject]@{ ProcessId = 101; SessionId = 7; ExecutablePath = 'C:\other\wardoff.exe'; CreationDate = $null }
    $probeState.Calls = 0
    Assert-Throws {
        Assert-SmokePreconditions -WardoffProcesses @($sameSession) -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { $probeState.Calls++; return $false }
    } 'conflicting Wardoff runtime process'
    Assert-True ($probeState.Calls -eq 0) 'The task probe ran after a process guard failure.'

    $otherSession = [pscustomobject]@{ ProcessId = 102; SessionId = 8; ExecutablePath = 'C:\other\wardoff.exe'; CreationDate = $null }
    Assert-SmokePreconditions -WardoffProcesses @($otherSession) -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { return $false }

    $sameBinaryOtherSession = [pscustomobject]@{ ProcessId = 103; SessionId = 8; ExecutablePath = $binaryPath; CreationDate = $null }
    Assert-Throws {
        Assert-SmokePreconditions -WardoffProcesses @($sameBinaryOtherSession) -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { return $false }
    } 'conflicting Wardoff runtime process'

    $exitedOwned = [pscustomobject]@{ Id = 103; HasExited = $true; StartTime = [datetime]::UtcNow }
    $observedReusedPid = [pscustomobject]@{ ProcessId = 103; SessionId = 7; ExecutablePath = $binaryPath; CreationDate = [datetime]::UtcNow }
    Assert-Throws {
        Assert-SmokePreconditions -WardoffProcesses @($observedReusedPid) -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { return $false } -OwnedProcesses @($exitedOwned)
    } 'conflicting Wardoff runtime process'

    Assert-Throws {
        Assert-SmokePreconditions -WardoffProcesses @() -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { return $true }
    } 'autostart task already exists'

    Assert-Throws {
        Assert-SmokePreconditions -WardoffProcesses @() -CurrentSessionId 7 -BinaryPath $binaryPath -AutostartTaskProbe { throw 'Task Scheduler query failed.' }
    } 'Task Scheduler query failed'

    $taskIdentity = [pscustomobject]@{ Xml = '<Task><Action /></Task>'; Path = $binaryPath; WorkingDirectory = (Split-Path -Parent $binaryPath) }
    $sameTaskIdentity = [pscustomobject]@{ Xml = $taskIdentity.Xml; Path = $taskIdentity.Path; WorkingDirectory = $taskIdentity.WorkingDirectory }
    $changedTaskIdentity = [pscustomobject]@{ Xml = '<Task><ChangedAction /></Task>'; Path = $taskIdentity.Path; WorkingDirectory = $taskIdentity.WorkingDirectory }
    Assert-True (Test-SmokeAutostartTaskIdentity -ExpectedIdentity $taskIdentity -ActualIdentity $sameTaskIdentity) 'An unchanged smoke-created task was not recognized as owned.'
    Assert-True (-not (Test-SmokeAutostartTaskIdentity -ExpectedIdentity $taskIdentity -ActualIdentity $changedTaskIdentity)) 'A changed task definition was still recognized as owned.'

    $owned = [pscustomobject]@{ Id = 201; HasExited = $false }
    $unowned = [pscustomobject]@{ Id = 202; HasExited = $false }
    $stopped = [System.Collections.Generic.List[int]]::new()
    Stop-SmokeOwnedProcesses -OwnedProcesses @($owned) -StopAction {
        param($process)
        $stopped.Add([int]$process.Id)
    } -WaitAction {
        param($process)
    }
    Assert-True ($stopped.Count -eq 1 -and $stopped[0] -eq 201) 'Cleanup did not stop exactly the owned process handle.'
    Assert-True (-not ($stopped -contains $unowned.Id)) 'Cleanup attempted to stop an unowned process.'

    $syntaxErrors = $null
    foreach ($path in @($helperPath, (Join-Path $PSScriptRoot 'smoke_test.ps1'))) {
        [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$syntaxErrors) | Out-Null
        if ($syntaxErrors) {
            throw (($syntaxErrors | ForEach-Object { $_.Message }) -join [Environment]::NewLine)
        }
    }
    Write-Output 'Smoke safety tests passed.'
    exit 0
}
catch {
    Write-Error $_.Exception.Message
    exit 1
}
