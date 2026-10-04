Set-StrictMode -Version Latest

function Get-SmokeCurrentSessionId {
    return [System.Diagnostics.Process]::GetCurrentProcess().SessionId
}

function Test-SmokeOwnedProcess {
    param(
        [Parameter(Mandatory = $true)]
        [object] $ObservedProcess,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]] $OwnedProcesses
    )

    foreach ($ownedProcess in $OwnedProcesses) {
        if ($null -eq $ownedProcess) {
            continue
        }
        if ($ownedProcess.HasExited) {
            continue
        }
        if ([int]$ObservedProcess.ProcessId -ne [int]$ownedProcess.Id) {
            continue
        }
        # The live Process object owns a kernel handle; an exited object cannot
        # authorize a reused PID to be treated as the smoke process.
        return $true
    }
    return $false
}

function Assert-SmokePreconditions {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]] $WardoffProcesses,
        [Parameter(Mandatory = $true)]
        [int] $CurrentSessionId,
        [Parameter(Mandatory = $true)]
        [string] $BinaryPath,
        [Parameter(Mandatory = $true)]
        [scriptblock] $AutostartTaskProbe,
        [AllowEmptyCollection()]
        [object[]] $OwnedProcesses = @()
    )

    $resolvedBinaryPath = [System.IO.Path]::GetFullPath($BinaryPath)
    $foreignProcess = @(
        $WardoffProcesses | Where-Object {
            (([int]$_.SessionId -eq $CurrentSessionId) -or
                ($_.ExecutablePath -and
                    [string]::Equals(
                        [System.IO.Path]::GetFullPath([string]$_.ExecutablePath),
                        $resolvedBinaryPath,
                        [System.StringComparison]::OrdinalIgnoreCase
                    ))) -and
            (-not (Test-SmokeOwnedProcess -ObservedProcess $_ -OwnedProcesses $OwnedProcesses))
        }
    )
    if ($foreignProcess.Count -gt 0) {
        $ids = ($foreignProcess | ForEach-Object { [int]$_.ProcessId }) -join ', '
        throw "Smoke test refused to start: a conflicting Wardoff runtime process already exists (session $CurrentSessionId or the same release binary in another session; PID(s): $ids). Use an isolated environment without an existing runtime."
    }

    $taskExists = & $AutostartTaskProbe
    if ([bool]$taskExists) {
        throw 'Smoke test refused to start: the \Wardoff autostart task already exists. Use an isolated environment without an existing autostart task.'
    }
}

function Stop-SmokeOwnedProcesses {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [object[]] $OwnedProcesses,
        [Parameter(Mandatory = $true)]
        [scriptblock] $StopAction,
        [scriptblock] $WaitAction = $null
    )

    foreach ($process in $OwnedProcesses) {
        if ($null -eq $process) {
            continue
        }
        try {
            if (-not $process.HasExited) {
                & $StopAction $process
                if ($null -ne $WaitAction) {
                    & $WaitAction $process
                }
            }
        }
        catch {
            if ($_.Exception.Message -notmatch 'cannot find|has exited') {
                Write-Warning "Could not stop owned Wardoff process $($process.Id): $($_.Exception.Message)"
            }
        }
    }
}

function Test-SmokeAutostartTaskIdentity {
    param(
        [Parameter(Mandatory = $true)][object] $ExpectedIdentity,
        [Parameter(Mandatory = $true)][object] $ActualIdentity
    )
    return [string]::Equals($ExpectedIdentity.Xml, $ActualIdentity.Xml, [System.StringComparison]::Ordinal) -and
        [string]::Equals($ExpectedIdentity.Path, $ActualIdentity.Path, [System.StringComparison]::OrdinalIgnoreCase) -and
        [string]::Equals($ExpectedIdentity.WorkingDirectory, $ActualIdentity.WorkingDirectory, [System.StringComparison]::OrdinalIgnoreCase)
}
