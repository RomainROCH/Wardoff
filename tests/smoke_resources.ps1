# Resource cleanup for an exclusive disposable Windows smoke session.
# Dot-sourcing defines functions only; native adapters run only when invoked.
# CLI/Task Scheduler operations cannot lock out unrelated concurrent actors.
Set-StrictMode -Version Latest

function Get-SmokeNativeTaskSnapshot {
    param([Parameter(Mandatory = $true)][string] $TaskPath)
    $scheduler = $folder = $task = $definition = $principal = $actions = $action = $null
    try {
        $scheduler = New-Object -ComObject 'Schedule.Service'
        $scheduler.Connect()
        $separator = $TaskPath.LastIndexOf('\')
        if ($separator -lt 0) { throw 'Expected an absolute task path.' }
        $folderPath = if ($separator -eq 0) { '\' } else { $TaskPath.Substring(0, $separator) }
        try {
            $folder = $scheduler.GetFolder($folderPath)
            $task = $folder.GetTask($TaskPath.Substring($separator + 1))
        } catch {
            if ($_.Exception.GetBaseException().HResult -in @(-2147024894, -2147024893, -2147023728)) {
                return [pscustomobject]@{ Exists = $false }
            }
            throw
        }
        $definition = $task.Definition
        $principal = $definition.Principal
        $principalId = [string]$principal.UserId
        if ([string]::IsNullOrWhiteSpace($principalId)) { $principalId = [string]$principal.GroupId }
        if ($principalId -notmatch '^S-\d-') {
            $principalId = ([Security.Principal.NTAccount]::new($principalId)).Translate([Security.Principal.SecurityIdentifier]).Value
        }
        $actions = $definition.Actions
        $actionPath = $workingDirectory = ''
        if ($actions.Count -eq 1) {
            $action = $actions.Item(1)
            if ($action.Type -eq 0) {
                $actionPath = [string]$action.Path
                $workingDirectory = [string]$action.WorkingDirectory
            }
        }
        return [pscustomobject]@{
            Exists = $true; Xml = [string]$task.Xml; Enabled = [bool]$task.Enabled
            Sddl = [string]$task.GetSecurityDescriptor(7); Principal = $principalId
            ActionPath = $actionPath; WorkingDirectory = $workingDirectory
        }
    } finally {
        foreach ($value in @($action, $actions, $principal, $definition, $task, $folder, $scheduler)) {
            if ($null -ne $value) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($value) }
        }
    }
}

function Set-SmokeNativeTaskEnabled {
    param([string] $TaskPath, [bool] $Enabled)
    $scheduler = $folder = $task = $null
    try {
        $scheduler = New-Object -ComObject 'Schedule.Service'; $scheduler.Connect()
        $separator = $TaskPath.LastIndexOf('\')
        $folder = $scheduler.GetFolder($(if ($separator -eq 0) { '\' } else { $TaskPath.Substring(0, $separator) }))
        $task = $folder.GetTask($TaskPath.Substring($separator + 1))
        $task.Enabled = $Enabled
    } finally {
        foreach ($value in @($task, $folder, $scheduler)) {
            if ($null -ne $value) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($value) }
        }
    }
}

function Remove-SmokeNativeTask {
    param([string] $TaskPath)
    if ($TaskPath -cne '\Wardoff') { throw 'Only the owned Wardoff autostart task may be deleted.' }
    $scheduler = $folder = $null
    try {
        $scheduler = New-Object -ComObject 'Schedule.Service'; $scheduler.Connect()
        $folder = $scheduler.GetFolder('\'); $folder.DeleteTask('Wardoff', 0)
    } finally {
        foreach ($value in @($folder, $scheduler)) {
            if ($null -ne $value) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($value) }
        }
    }
}

function Get-SmokeNativeEtwSessions {
    $tool = @(Get-Command logman.exe -CommandType Application -ErrorAction Stop)[0].Source
    $result = Invoke-SmokeChildCommand -FilePath $tool -Arguments @('query', '-ets') `
        -WorkingDirectory $PSScriptRoot -TimeoutMilliseconds 10000
    if ($result.ExitCode -ne 0) {
        throw ('Could not enumerate ETW sessions; smoke cannot establish a baseline. logman query -ets exit={0}; stdout={1}; stderr={2}' -f $result.ExitCode, $result.StandardOutput, $result.StandardError)
    }
    foreach ($line in @($result.StandardOutput -split '\r?\n')) {
        if ([string]$line -match '^\s*(Wardoff-Layer2-Local-\S+)(?:\s|$)') { $Matches[1] }
    }
}

function Stop-SmokeNativeEtwSession {
    param([string] $Name)
    if ($Name -cnotmatch '^Wardoff-Layer2-Local-[1-9][0-9]*-[0-9]+$') { throw 'Invalid owned ETW session name.' }
    $tool = @(Get-Command logman.exe -CommandType Application -ErrorAction Stop)[0].Source
    $result = Invoke-SmokeChildCommand -FilePath $tool -Arguments @('stop', $Name, '-ets') `
        -WorkingDirectory $PSScriptRoot -TimeoutMilliseconds 10000
    if ($result.ExitCode -ne 0) { throw "Could not stop owned ETW session '$Name'." }
}

function New-SmokeNativeResourceAdapters {
    return @{
        ReadTask = { param($path) Get-SmokeNativeTaskSnapshot -TaskPath $path }
        SetTaskEnabled = { param($path, $enabled) Set-SmokeNativeTaskEnabled -TaskPath $path -Enabled $enabled }
        DeleteTask = { param($path) Remove-SmokeNativeTask -TaskPath $path }
        ListEtw = { Get-SmokeNativeEtwSessions }
        StopEtw = { param($name) Stop-SmokeNativeEtwSession -Name $name }
        Principal = {
            $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
            try { $identity.User.Value } finally { $identity.Dispose() }
        }
    }
}

function Copy-SmokeTaskSnapshot {
    param([object] $Snapshot)
    if ($null -eq $Snapshot) { throw 'Task probe returned no snapshot.' }
    if (-not [bool]$Snapshot.Exists) { return [pscustomobject]@{ Exists = $false } }
    return [pscustomobject]@{
        Exists = $true; Xml = [string]$Snapshot.Xml; Enabled = [bool]$Snapshot.Enabled
        Sddl = [string]$Snapshot.Sddl; Principal = [string]$Snapshot.Principal
        ActionPath = [string]$Snapshot.ActionPath; WorkingDirectory = [string]$Snapshot.WorkingDirectory
    }
}

function Get-SmokeTaskDefinitionIdentity {
    param([object] $Snapshot)
    $document = [Xml.XmlDocument]::new(); $document.XmlResolver = $null
    $document.LoadXml($Snapshot.Xml)
    foreach ($node in @($document.SelectNodes('/*[local-name()="Task"]/*[local-name()="Settings"]/*[local-name()="Enabled"]'))) {
        [void]$node.ParentNode.RemoveChild($node)
    }
    return $document.OuterXml
}

function Test-SmokeTaskIdentity {
    param([object] $Expected, [object] $Actual, [switch] $IgnoreEnabled)
    if ([bool]$Expected.Exists -ne [bool]$Actual.Exists) { return $false }
    if (-not $Expected.Exists) { return $true }
    $expectedXml = if ($IgnoreEnabled) { Get-SmokeTaskDefinitionIdentity $Expected } else { $Expected.Xml }
    $actualXml = if ($IgnoreEnabled) { Get-SmokeTaskDefinitionIdentity $Actual } else { $Actual.Xml }
    return [string]::Equals($expectedXml, $actualXml, [StringComparison]::Ordinal) -and
        [string]::Equals($Expected.Sddl, $Actual.Sddl, [StringComparison]::Ordinal) -and
        [string]::Equals($Expected.Principal, $Actual.Principal, [StringComparison]::OrdinalIgnoreCase) -and
        ($IgnoreEnabled -or $Expected.Enabled -eq $Actual.Enabled)
}

function ConvertTo-SmokeAutostartIdentityPath {
    param([AllowEmptyString()][string] $Path)
    # Only equate canonical local DOS paths with their extended-length spelling.
    # Extended paths bypass Win32 parsing; do not collapse dots, spaces or devices.
    # https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
    if ([string]::IsNullOrWhiteSpace($Path)) { throw 'Empty autostart identity path.' }
    $local = if ($Path.StartsWith('\\?\', [StringComparison]::Ordinal)) { $Path.Substring(4) } else { $Path }
    if ($local -cnotmatch '^[A-Za-z]:\\' -or $local.Contains('/')) {
        throw 'Autostart identity requires a canonical local drive path.'
    }
    $tail = $local.Substring(3)
    if ($tail -match '[\x00-\x1f<>:"|?*]') { throw 'Ambiguous autostart identity path characters.' }
    if ($tail.Length -gt 0) {
        foreach ($segment in $tail.Split('\')) {
            if ($segment.Length -eq 0 -or $segment -match '[ .]$' -or
                $segment -match '^(?i:CON|PRN|AUX|NUL|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(?:\.|$)') {
                throw 'Ambiguous autostart identity path segment.'
            }
        }
    }
    $full = [IO.Path]::GetFullPath($local)
    if (-not [string]::Equals($full, $local, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Autostart identity path must already be canonical.'
    }
    return $full
}

function Get-SmokeAutostartIdentityComparison {
    param([object] $State, [object] $Task)
    $fields = [Collections.Generic.List[object]]::new()
    foreach ($field in @('ActionPath', 'WorkingDirectory', 'Principal')) {
        $expected = switch ($field) {
            'ActionPath' { $State.BinaryPath }
            'WorkingDirectory' { $State.WorkingDirectory }
            'Principal' { $State.ExpectedPrincipal }
        }
        $actual = [string]$Task.$field
        $normalizedExpected = $normalizedActual = $null
        $errorText = $null
        try {
            if ($field -eq 'Principal') {
                $normalizedExpected = $expected; $normalizedActual = $actual
            } else {
                $normalizedExpected = ConvertTo-SmokeAutostartIdentityPath $expected
                $normalizedActual = ConvertTo-SmokeAutostartIdentityPath $actual
            }
        } catch { $errorText = $_.Exception.Message }
        $fields.Add([pscustomobject]@{
            Field = $field; Expected = $expected; Actual = $actual
            NormalizedExpected = $normalizedExpected; NormalizedActual = $normalizedActual
            Matches = ($null -eq $errorText -and [string]::Equals($normalizedExpected, $normalizedActual, [StringComparison]::OrdinalIgnoreCase))
            Error = $errorText
        })
    }
    return [pscustomobject]@{
        Matches = (@($fields | Where-Object { -not $_.Matches }).Count -eq 0)
        Fields = $fields.ToArray()
    }
}

function New-SmokeResourceState {
    param([Parameter(Mandatory = $true)][string] $BinaryPath,
          [Parameter(Mandatory = $true)][string] $WorkingDirectory,
          [hashtable] $Adapters = (New-SmokeNativeResourceAdapters))
    foreach ($name in @('ReadTask', 'SetTaskEnabled', 'DeleteTask', 'ListEtw', 'StopEtw', 'Principal')) {
        if (-not $Adapters.ContainsKey($name) -or $Adapters[$name] -isnot [scriptblock]) { throw "Missing resource adapter $name." }
    }
    return [pscustomobject]@{
        BinaryPath = [IO.Path]::GetFullPath($BinaryPath); WorkingDirectory = [IO.Path]::GetFullPath($WorkingDirectory)
        Adapters = $Adapters; Initialized = $false; ExpectedPrincipal = ''
        Reboot = $null; Autostart = $null; AutostartAttempted = $false; OwnedAutostart = $null
        AutostartIdentityChecks = [Collections.Generic.List[object]]::new()
        BaselineEtw = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        OwnedEtw = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        ProtocolRequiresExclusiveDisposableSession = $true
    }
}

function Initialize-SmokeResources {
    param([Parameter(Mandatory = $true)][object] $State)
    if ($State.Initialized) { throw 'Resource baseline already initialized.' }
    $State.Reboot = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Microsoft\Windows\UpdateOrchestrator\Reboot')
    $State.Autostart = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Wardoff')
    if ($State.Autostart.Exists) { throw 'Refusing smoke with a pre-existing Wardoff autostart task.' }
    $State.ExpectedPrincipal = [string](& $State.Adapters.Principal)
    if ([string]::IsNullOrWhiteSpace($State.ExpectedPrincipal)) { throw 'Current principal could not be established.' }
    foreach ($name in @(& $State.Adapters.ListEtw)) { [void]$State.BaselineEtw.Add([string]$name) }
    $State.Initialized = $true
}

function Start-SmokeAutostartAttempt {
    param([Parameter(Mandatory = $true)][object] $State)
    if (-not $State.Initialized) { throw 'Missing resource baseline.' }
    $task = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Wardoff')
    if ($task.Exists) { throw 'Refusing to overwrite an existing Wardoff autostart task.' }
    $State.OwnedAutostart = $null
    $State.AutostartAttempted = $true
}

function Capture-SmokeAutostartOwnership {
    param([Parameter(Mandatory = $true)][object] $State)
    if (-not $State.Initialized -or -not $State.AutostartAttempted) { throw 'No owned autostart creation attempt.' }
    $task = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Wardoff')
    if (-not $task.Exists) { return }
    $comparison = Get-SmokeAutostartIdentityComparison -State $State -Task $task
    $State.AutostartIdentityChecks.Add($comparison)
    if (-not $comparison.Matches) {
        throw ('Autostart task identity does not belong to this smoke run. Comparison=' + ($comparison | ConvertTo-Json -Depth 5 -Compress))
    }
    if ($null -ne $State.OwnedAutostart -and -not (Test-SmokeTaskIdentity $State.OwnedAutostart $task)) {
        throw 'Owned autostart task definition or security changed.'
    }
    $State.OwnedAutostart = $task
}

function Capture-SmokeOwnedEtw {
    param([Parameter(Mandatory = $true)][object] $State,
          [Parameter(Mandatory = $true)][object] $Process,
          [datetime] $ObservedAtUtc = [DateTime]::UtcNow)
    if (-not $State.Initialized) { throw 'Missing resource baseline.' }
    $start = ([DateTimeOffset]$Process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    $endTime = if ($Process.HasExited) { $Process.ExitTime.ToUniversalTime() } else { $ObservedAtUtc.ToUniversalTime() }
    $end = ([DateTimeOffset]$endTime).ToUnixTimeMilliseconds()
    if ($end -lt $start) { throw 'Process lifetime clock moved backwards; ETW ownership is unavailable.' }
    $errors = [Collections.Generic.List[string]]::new()
    foreach ($name in @(& $State.Adapters.ListEtw)) {
        if ($State.BaselineEtw.Contains([string]$name)) { continue }
        try {
            if ([string]$name -cmatch '^Wardoff-Layer2-Local-([1-9][0-9]*)-([0-9]+)$' -and [long]$Matches[1] -eq [long]$Process.Id) {
                $created = [long]$Matches[2]
                if ($created -lt $start -or $created -gt $end) {
                    $errors.Add('Unattributed ETW session shares an owned PID outside its lifetime.')
                    continue
                }
                [void]$State.OwnedEtw.Add([string]$name)
            }
        } catch {
            $errors.Add('An ETW session could not be classified against the owned process lifetime.')
        }
    }
    if ($errors.Count -gt 0) { throw ($errors -join '; ') }
}

function Restore-SmokeResources {
    param([Parameter(Mandatory = $true)][object] $State,
          [Parameter(Mandatory = $true)][bool] $RuntimeExited,
          [object[]] $OwnedProcesses = @())
    if (-not $State.Initialized) { return }
    if (-not $RuntimeExited -or @($OwnedProcesses | Where-Object { -not $_.HasExited }).Count -gt 0) {
        throw 'Refusing resource mutation until every owned runtime has exited.'
    }
    $errors = [Collections.Generic.List[string]]::new()
    foreach ($process in $OwnedProcesses) {
        try { Capture-SmokeOwnedEtw -State $State -Process $process } catch { $errors.Add($_.Exception.Message) }
    }
    try {
        $path = '\Microsoft\Windows\UpdateOrchestrator\Reboot'
        $actual = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask $path)
        if (-not (Test-SmokeTaskIdentity $State.Reboot $actual -IgnoreEnabled)) { throw 'Reboot task definition or security changed; refusing restoration.' }
        if ($State.Reboot.Exists -and $actual.Enabled -ne $State.Reboot.Enabled) {
            & $State.Adapters.SetTaskEnabled $path $State.Reboot.Enabled
        }
        $actual = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask $path)
        if (-not (Test-SmokeTaskIdentity $State.Reboot $actual -IgnoreEnabled) -or
            ($State.Reboot.Exists -and $actual.Enabled -ne $State.Reboot.Enabled)) { throw 'Reboot task restoration did not verify.' }
    } catch { $errors.Add($_.Exception.Message) }
    try {
        $actual = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Wardoff')
        if ($actual.Exists) {
            if (-not $State.AutostartAttempted) { throw 'Unowned autostart task appeared during smoke; left untouched.' }
            Capture-SmokeAutostartOwnership -State $State
            $actual = Copy-SmokeTaskSnapshot (& $State.Adapters.ReadTask '\Wardoff')
            if (-not (Test-SmokeTaskIdentity $State.OwnedAutostart $actual)) { throw 'Autostart task changed before cleanup; left untouched.' }
            & $State.Adapters.DeleteTask '\Wardoff'
        }
        if ((& $State.Adapters.ReadTask '\Wardoff').Exists) { throw 'Owned autostart task remains after cleanup.' }
        $State.OwnedAutostart = $null
    } catch { $errors.Add($_.Exception.Message) }
    $names = @($State.OwnedEtw)
    try {
        $active = @(& $State.Adapters.ListEtw)
        foreach ($name in $names) {
            if ($active -contains $name) {
                try { & $State.Adapters.StopEtw $name } catch { $errors.Add($_.Exception.Message) }
            }
        }
        $remaining = @(& $State.Adapters.ListEtw)
        foreach ($name in $names) {
            if ($remaining -contains $name) { $errors.Add("Owned ETW session remains: $name") }
            else { [void]$State.OwnedEtw.Remove($name) }
        }
    } catch { $errors.Add($_.Exception.Message) }
    if ($errors.Count -gt 0) { throw ('Smoke resource cleanup failed: ' + ($errors -join '; ')) }
}
