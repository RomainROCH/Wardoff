Set-StrictMode -Version Latest

function Invoke-SmokeReleaseBuild {
    param([bool] $UseExistingBinary, [string] $BinaryPath, [scriptblock] $BuildAction)
    if ($UseExistingBinary) {
        if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
            throw "Expected an existing release binary at $BinaryPath; compilation is disabled."
        }
        return
    }
    $result = & $BuildAction
    if ($result.ExitCode -ne 0) {
        throw "cargo build --release --locked failed with exit code $($result.ExitCode)."
    }
}

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
