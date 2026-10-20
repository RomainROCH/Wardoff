[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'smoke_resources.ps1')

function Assert-ResourceTest([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
}
function Assert-ResourceThrows([scriptblock] $Action, [string] $Message) {
    $caught = $null
    try { & $Action } catch { $caught = $_.Exception.Message }
    Assert-ResourceTest ($null -ne $caught -and $caught.Contains($Message)) "Expected failure: $Message; received: $caught"
}
function New-ResourceFixture([bool] $RebootExists = $true, [bool] $Enabled = $true) {
    $fixture = @{
        Reboot = [pscustomobject]@{ Exists = $RebootExists; Xml = '<Task><Settings><Enabled>true</Enabled></Settings><Actions><Exec /></Actions></Task>'; Enabled = $Enabled; Sddl = 'fixture-security'; Principal = 'S-1-fixture'; ActionPath = ''; WorkingDirectory = '' }
        Auto = [pscustomobject]@{ Exists = $false }; Etw = [Collections.Generic.List[string]]::new()
        Writes = [Collections.Generic.List[string]]::new(); FailRestore = $false; FailStop = $false; ReadDenied = $false
    }
    $adapters = @{
        ReadTask = { param($path) if ($fixture.ReadDenied) { throw 'fixture task access denied' }; if ($path -eq '\Wardoff') { return $fixture.Auto }; return $fixture.Reboot }.GetNewClosure()
        SetTaskEnabled = { param($path, $enabled) $fixture.Writes.Add('enabled'); if ($fixture.FailRestore) { throw 'fixture restoration failed' }; $fixture.Reboot.Enabled = $enabled }.GetNewClosure()
        DeleteTask = { param($path) $fixture.Writes.Add('delete'); $fixture.Auto = [pscustomobject]@{ Exists = $false } }.GetNewClosure()
        ListEtw = { return @($fixture.Etw.ToArray()) }.GetNewClosure()
        StopEtw = { param($name) $fixture.Writes.Add('stop:' + $name); if ($fixture.FailStop) { throw 'fixture trace stop failed' }; [void]$fixture.Etw.Remove($name) }.GetNewClosure()
        Principal = { return 'S-1-fixture' }
    }
    $state = New-SmokeResourceState -BinaryPath 'C:\fixture\wardoff.exe' -WorkingDirectory 'C:\fixture' -Adapters $adapters
    return [pscustomobject]@{ Fixture = $fixture; State = $state }
}
function New-FixtureAutostart {
    return [pscustomobject]@{ Exists = $true; Xml = '<Task><Owned /></Task>'; Enabled = $true; Sddl = 'fixture-auto-security'; Principal = 'S-1-fixture'; ActionPath = 'C:\fixture\wardoff.exe'; WorkingDirectory = 'C:\fixture' }
}

$cases = 0
try {
    $canary = $false
    try { Assert-ResourceThrows {} 'canary' } catch { $canary = $true }
    Assert-ResourceTest $canary 'Throw assertion accepted an action that did not throw.'

    foreach ($initial in @($true, $false)) {
        $f = New-ResourceFixture -Enabled $initial; Initialize-SmokeResources $f.State
        $f.Fixture.Reboot.Enabled = -not $initial
        Restore-SmokeResources $f.State -RuntimeExited $true
        Assert-ResourceTest ($f.Fixture.Reboot.Enabled -eq $initial) 'Initial Reboot state was not restored.'
        $writes = $f.Fixture.Writes.Count; Restore-SmokeResources $f.State -RuntimeExited $true
        Assert-ResourceTest ($writes -eq $f.Fixture.Writes.Count) 'Repeated cleanup changed restored resources.'
        $cases++
    }
    $f = New-ResourceFixture -RebootExists $false; Initialize-SmokeResources $f.State
    Restore-SmokeResources $f.State -RuntimeExited $true
    Assert-ResourceTest ($f.Fixture.Writes.Count -eq 0) 'Absent Reboot task was created.'; $cases++

    $f = New-ResourceFixture; $f.Fixture.ReadDenied = $true
    Assert-ResourceThrows { Initialize-SmokeResources $f.State } 'access denied'
    Assert-ResourceTest (-not $f.State.Initialized -and $f.Fixture.Writes.Count -eq 0) 'Failed baseline permitted mutation.'; $cases++

    $f = New-ResourceFixture; $f.Fixture.Auto = New-FixtureAutostart
    Assert-ResourceThrows { Initialize-SmokeResources $f.State } 'pre-existing'
    Assert-ResourceTest ($f.Fixture.Writes.Count -eq 0) 'Pre-existing task was changed.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State
    Start-SmokeAutostartAttempt $f.State
    $f.Fixture.Auto = New-FixtureAutostart # Model registration succeeding before CLI reports failure.
    Restore-SmokeResources $f.State -RuntimeExited $true
    Assert-ResourceTest (-not $f.Fixture.Auto.Exists -and $f.Fixture.Writes.Contains('delete')) 'Partial creation was not cleaned.'; $cases++

    foreach ($field in @('Xml', 'Sddl')) {
        $f = New-ResourceFixture; Initialize-SmokeResources $f.State
        $f.Fixture.Reboot.Enabled = $false
        if ($field -eq 'Xml') { $f.Fixture.Reboot.Xml = '<Task><Foreign /></Task>' } else { $f.Fixture.Reboot.Sddl = 'foreign-security' }
        Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true } 'Reboot task definition or security changed'
        Assert-ResourceTest (-not $f.Fixture.Writes.Contains('enabled')) 'Foreign Reboot identity was modified.'; $cases++
    }

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State; Start-SmokeAutostartAttempt $f.State
    $f.Fixture.Auto = New-FixtureAutostart; Capture-SmokeAutostartOwnership $f.State
    $f.Fixture.Auto.Xml = '<Task><Foreign /></Task>'
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true } 'definition or security changed'
    Assert-ResourceTest (-not $f.Fixture.Writes.Contains('delete')) 'Changed autostart task was deleted.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State; Start-SmokeAutostartAttempt $f.State
    $f.Fixture.Auto = New-FixtureAutostart; $f.Fixture.Auto.Principal = 'S-1-foreign'
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true } 'does not belong'
    Assert-ResourceTest (-not $f.Fixture.Writes.Contains('delete')) 'Foreign principal task was deleted.'; $cases++

    $start = [DateTime]::SpecifyKind([datetime]'2026-10-20T10:00:00', [DateTimeKind]::Utc)
    $milliseconds = ([DateTimeOffset]$start).ToUnixTimeMilliseconds()
    $process = [pscustomobject]@{ Id = 321; StartTime = $start; HasExited = $true; ExitTime = $start.AddSeconds(2) }
    $ownedName = 'Wardoff-Layer2-Local-321-' + ($milliseconds + 100)
    $foreignName = 'Wardoff-Layer2-Local-999-' + ($milliseconds + 100)
    $f = New-ResourceFixture; $f.Fixture.Etw.Add('Wardoff-Layer2-Local-321-1'); Initialize-SmokeResources $f.State
    $f.Fixture.Etw.Add($ownedName); $f.Fixture.Etw.Add($foreignName)
    Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($process)
    Assert-ResourceTest (-not $f.Fixture.Etw.Contains($ownedName) -and $f.Fixture.Etw.Contains($foreignName) -and $f.Fixture.Etw.Contains('Wardoff-Layer2-Local-321-1')) 'ETW cleanup touched baseline/foreign state or left owned state.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State; $f.Fixture.Etw.Add($ownedName)
    $live = [pscustomobject]@{ Id = 321; StartTime = $start; HasExited = $false; ExitTime = $start.AddSeconds(2) }
    Capture-SmokeOwnedEtw $f.State -Process $live -ObservedAtUtc $start.AddSeconds(1)
    Assert-ResourceTest ($f.State.OwnedEtw.Contains($ownedName)) 'Live owned ETW session was not captured.'
    $live.HasExited = $true
    Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($live)
    $writes = $f.Fixture.Writes.Count; Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($live)
    Assert-ResourceTest ($f.State.OwnedEtw.Count -eq 0 -and $writes -eq $f.Fixture.Writes.Count) 'Repeated cleanup stopped an ETW session twice.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State
    $f.Fixture.Etw.Add('Wardoff-Layer2-Local-321-' + ($milliseconds + 5000))
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($process) } 'outside its lifetime'
    Assert-ResourceTest ($f.Fixture.Writes.Count -eq 0) 'PID reuse authorised trace deletion.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State
    $outOfWindow = 'Wardoff-Layer2-Local-321-' + ($milliseconds + 5000)
    $f.Fixture.Etw.Add($outOfWindow); $f.Fixture.Etw.Add($ownedName)
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($process) } 'outside its lifetime'
    Assert-ResourceTest (-not $f.Fixture.Etw.Contains($ownedName) -and $f.Fixture.Etw.Contains($outOfWindow)) 'An out-of-window session prevented valid owned ETW cleanup or was itself removed.'
    Assert-ResourceTest ($f.Fixture.Writes.Contains('stop:' + $ownedName) -and -not $f.Fixture.Writes.Contains('stop:' + $outOfWindow)) 'Mixed ETW cleanup did not stop only the valid owned session.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State
    $f.Fixture.Reboot.Enabled = $false; $f.Fixture.FailRestore = $true; $f.Fixture.Etw.Add($ownedName)
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($process) } 'restoration failed'
    Assert-ResourceTest (-not $f.Fixture.Etw.Contains($ownedName)) 'Task restoration error prevented ETW cleanup.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State; $f.Fixture.FailStop = $true
    $f.Fixture.Etw.Add($ownedName)
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $true -OwnedProcesses @($process) } 'trace stop failed'
    Assert-ResourceTest ($f.State.OwnedEtw.Count -eq 1) 'Failed trace cleanup was marked restored.'; $cases++

    $f = New-ResourceFixture; Initialize-SmokeResources $f.State; $f.Fixture.Reboot.Enabled = $false
    Assert-ResourceThrows { Restore-SmokeResources $f.State -RuntimeExited $false } 'every owned runtime has exited'
    Assert-ResourceTest ($f.Fixture.Writes.Count -eq 0) 'Resources were mutated while runtime remained alive.'; $cases++

    $adapterCases = & {
        $fixture = @{
            Calls = [Collections.Generic.List[object]]::new(); Timeout = $false; ExitCode = 0
            Stdout = "Logger Name`r`nWardoff-Layer2-Local-321-1  Running`r`nUnrelated  Running"
            Stderr = 'Wardoff-Layer2-Local-555-2  Must not be parsed'
        }
        function Get-Command {
            param($Name, $CommandType, $ErrorAction)
            if ($Name -ne 'logman.exe') { throw 'Unexpected fixture command lookup.' }
            return [pscustomobject]@{ Source = 'C:\inert-fixture\logman.exe' }
        }
        function Invoke-SmokeChildCommand {
            param($FilePath, [string[]]$Arguments, $WorkingDirectory, [int]$TimeoutMilliseconds)
            $fixture.Calls.Add([pscustomobject]@{
                FilePath = $FilePath; Arguments = @($Arguments)
                WorkingDirectory = $WorkingDirectory; TimeoutMilliseconds = $TimeoutMilliseconds
            })
            if ($fixture.Timeout) { throw [TimeoutException]::new('fixture logman timeout') }
            return [pscustomobject]@{ ExitCode = $fixture.ExitCode; StandardOutput = $fixture.Stdout; StandardError = $fixture.Stderr }
        }
        $names = @(Get-SmokeNativeEtwSessions)
        Assert-ResourceTest ($names.Count -eq 1 -and $names[0] -ceq 'Wardoff-Layer2-Local-321-1') 'ETW adapter parsed stderr or unrelated output.'
        Assert-ResourceTest (($fixture.Calls[0].Arguments -join ',') -ceq 'query,-ets') 'ETW query arguments changed.'
        Stop-SmokeNativeEtwSession -Name 'Wardoff-Layer2-Local-321-1'
        Assert-ResourceTest (($fixture.Calls[1].Arguments -join ',') -ceq 'stop,Wardoff-Layer2-Local-321-1,-ets') 'ETW stop did not target exactly its owned name.'
        foreach ($call in $fixture.Calls) {
            Assert-ResourceTest ($call.TimeoutMilliseconds -eq 10000 -and $call.FilePath -ceq 'C:\inert-fixture\logman.exe' -and
                [IO.Path]::GetFullPath($call.WorkingDirectory) -ceq [IO.Path]::GetFullPath($PSScriptRoot)) 'Native ETW adapter lost its deadline, command or working directory.'
        }
        $fixture.Timeout = $true
        Assert-ResourceThrows { Get-SmokeNativeEtwSessions } 'fixture logman timeout'
        Assert-ResourceThrows { Stop-SmokeNativeEtwSession -Name 'Wardoff-Layer2-Local-321-1' } 'fixture logman timeout'
        Assert-ResourceTest ($fixture.Calls[2].TimeoutMilliseconds -eq 10000 -and $fixture.Calls[3].TimeoutMilliseconds -eq 10000) 'Timeout calls lost the fixed deadline.'
        $fixture.Timeout = $false; $fixture.ExitCode = 37
        Assert-ResourceThrows { Get-SmokeNativeEtwSessions } 'Could not enumerate'
        Assert-ResourceThrows { Stop-SmokeNativeEtwSession -Name 'Wardoff-Layer2-Local-321-1' } 'Could not stop'
        $before = $fixture.Calls.Count
        Assert-ResourceThrows { Stop-SmokeNativeEtwSession -Name 'foreign-name' } 'Invalid owned'
        Assert-ResourceTest ($fixture.Calls.Count -eq $before) 'Invalid trace name reached the child command adapter.'
        return 7
    }
    $cases += $adapterCases

    Write-Output "Smoke resource fixtures passed ($cases cases; callbacks only, no native task/ETW/runtime access)."
    exit 0
} catch {
    Write-Error $_.Exception.Message
    exit 1
}
