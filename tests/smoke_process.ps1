# Dot-sourcing defines functions only. Child execution requires PowerShell 7.
# This helper retains the Process handle until exit/cleanup and never selects a
# process by executable name or path. Callbacks must return promptly and must
# not dispose the supplied Process. By default it is disposed before return.
# With RetainProcess the OnStarted callback takes ownership and must dispose it
# during the caller's final cleanup, including after a command failure.
# Kill(entireProcessTree: true) is best effort for a live owned parent; this is
# not an atomic Windows job boundary for descendants escaping after parent exit.
# Run the native smoke only in its separately authorized, exclusive test VM.

function Invoke-SmokeChildCommand {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $FilePath,
        [AllowEmptyCollection()][string[]] $Arguments = @(),
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $WorkingDirectory,
        [hashtable] $Environment = @{},
        [ValidateRange(1, 3600000)][int] $TimeoutMilliseconds = 10000,
        [scriptblock] $OnStarted = $null,
        [scriptblock] $OnPoll = $null,
        [switch] $RetainProcess,
        [ValidateRange(1, 16777216)][int] $MaxOutputCharacters = 1048576
    )

    if ($PSVersionTable.PSVersion.Major -lt 7) {
        throw 'Invoke-SmokeChildCommand requires PowerShell 7.'
    }
    if ($RetainProcess -and $null -eq $OnStarted) {
        throw 'RetainProcess requires an OnStarted callback that owns and eventually disposes the Process.'
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = [Diagnostics.ProcessStartInfo]::new()
    $process.StartInfo.FileName = $FilePath
    $process.StartInfo.WorkingDirectory = $WorkingDirectory
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $process.StartInfo.ArgumentList.Add([string]$argument)
    }
    foreach ($key in $Environment.Keys) {
        if ($null -eq $Environment[$key]) {
            [void]$process.StartInfo.Environment.Remove([string]$key)
        }
        else {
            $process.StartInfo.Environment[[string]$key] = [string]$Environment[$key]
        }
    }

    $started = $false
    $handedOff = $false
    $processId = $null
    $streams = @()
    $failure = $null
    $result = $null
    $cleanupErrors = [Collections.Generic.List[string]]::new()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $process.Start()) { throw 'The smoke child did not start.' }
        $started = $true
        $processId = $process.Id
        # Keep the handle returned for this launch, including through cleanup.
        $null = $process.Handle
        if ($null -ne $OnStarted) {
            $handedOff = $true
            $null = & $OnStarted $process
        }
        foreach ($reader in @($process.StandardOutput, $process.StandardError)) {
            $stream = @{
                Reader = $reader
                Buffer = [char[]]::new(4096)
                Text = [Text.StringBuilder]::new()
                Done = $false
                Pending = $null
            }
            $stream.Pending = $reader.ReadAsync($stream.Buffer, 0, $stream.Buffer.Length)
            $streams += $stream
        }
        while ($true) {
            foreach ($stream in $streams) {
                if (-not $stream.Done -and $stream.Pending.IsCompleted) {
                    $read = $stream.Pending.GetAwaiter().GetResult()
                    if ($read -eq 0) {
                        $stream.Done = $true
                    }
                    else {
                        if ($stream.Text.Length + $read -gt $MaxOutputCharacters) {
                            throw 'Smoke child output exceeded the per-stream capture limit.'
                        }
                        [void]$stream.Text.Append($stream.Buffer, 0, $read)
                        $stream.Pending = $stream.Reader.ReadAsync($stream.Buffer, 0, $stream.Buffer.Length)
                    }
                }
            }
            if ($clock.ElapsedMilliseconds -ge $TimeoutMilliseconds) {
                throw [TimeoutException]::new("Smoke child timed out after $TimeoutMilliseconds ms (including output drain).")
            }
            $process.Refresh()
            if ($process.HasExited -and $streams[0].Done -and $streams[1].Done) { break }
            if ($null -ne $OnPoll) { $null = & $OnPoll $process }
            Start-Sleep -Milliseconds 10
        }

        # HasExited can observe an exit code before the process wait handle is
        # signaled. Reap within the original deadline instead of using a zero wait.
        $remaining = [Math]::Max(1, $TimeoutMilliseconds - [int]$clock.ElapsedMilliseconds)
        if (-not $process.WaitForExit([int]$remaining)) {
            throw [TimeoutException]::new('Smoke child exit did not complete within its deadline.')
        }
        $stdout = $streams[0].Text.ToString()
        $stderr = $streams[1].Text.ToString()
        $combined = @($stdout.Trim(), $stderr.Trim()) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
        $result = [pscustomobject]@{
            ExitCode = $process.ExitCode
            StandardOutput = $stdout
            StandardError = $stderr
            Output = ($combined -join [Environment]::NewLine)
            LaunchedProcessId = $processId
        }
    }
    catch {
        $failure = $_.Exception
    }
    finally {
        if ($started) {
            try {
                $process.Refresh()
                if (-not $process.HasExited) {
                    $process.Kill($true)
                }
                if (-not $process.WaitForExit(2000)) {
                    $cleanupErrors.Add('The owned child did not exit within the two-second reap deadline.')
                }
            }
            catch {
                $cleanupErrors.Add("Owned child cleanup failed: $($_.Exception.Message)")
            }
        }
        foreach ($stream in $streams) {
            try { $stream.Reader.Dispose() }
            catch { $cleanupErrors.Add("Owned stream cleanup failed: $($_.Exception.Message)") }
        }
        if (-not $RetainProcess -or -not $handedOff) { $process.Dispose() }
        $clock.Stop()
    }

    if ($cleanupErrors.Count -gt 0) {
        $prefix = if ($null -ne $failure) { $failure.Message + ' ' } else { '' }
        $failure = [InvalidOperationException]::new($prefix + ($cleanupErrors -join ' '), $failure)
    }
    if ($null -ne $failure) {
        $failure.Data['SmokeChildProcessId'] = $processId
        if ($streams.Count -eq 2) {
            $failure.Data['StandardOutput'] = $streams[0].Text.ToString()
            $failure.Data['StandardError'] = $streams[1].Text.ToString()
        }
        throw $failure
    }
    return $result
}
