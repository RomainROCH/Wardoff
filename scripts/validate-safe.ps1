[CmdletBinding()]
param([switch]$PlanOnly, [string]$OutputDirectory)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$nativeTest = 'blocker::sleep::tests::activation_reports_an_acquired_request_before_returning'
$plan = @(
    @{ label = 'fmt'; tool = 'cargo'; arguments = @('fmt', '--all', '--', '--check'); seconds = 120 },
    @{ label = 'check'; tool = 'cargo'; arguments = @('check', '--locked'); seconds = 600 },
    @{ label = 'inventory'; tool = 'cargo'; arguments = @('test', '--locked', '--', '--list'); seconds = 600 },
    @{ label = 'test'; tool = 'cargo'; arguments = @('test', '--locked', '--', '--skip', $nativeTest, '--nocapture', '--test-threads=1'); seconds = 600 },
    @{ label = 'clippy'; tool = 'cargo'; arguments = @('clippy', '--locked', '--all-targets', '--all-features', '--', '-D', 'warnings'); seconds = 600 },
    @{ label = 'build'; tool = 'cargo'; arguments = @('build', '--release', '--locked'); seconds = 900 },
    @{ label = 'help'; tool = 'wardoff'; arguments = @('--help'); seconds = 8 },
    @{ label = 'version'; tool = 'wardoff'; arguments = @('--version'); seconds = 8 }
)
$excluded = @(
    @{ name = 'Native PowerRequest'; status = 'NOT_RUN'; test = $nativeTest },
    @{ name = 'System smoke'; status = 'NOT_RUN'; reason = 'Runtime and OS changes require separate qualification.' },
    @{ name = 'Portable CLI13'; status = 'NOT_RUN'; reason = 'Namespace variant qualification is separate.' },
    @{ name = 'Hardware'; status = 'NOT_RUN'; reason = 'Physical power transitions require separate qualification.' }
)
if ($PlanOnly) {
    [ordered]@{ schema = 1; mode = 'PLAN_ONLY'; scope = 'SAFE_ONLY'; executes_commands = $false; plan = $plan; exclusions = $excluded; release_qualified = $false } | ConvertTo-Json -Depth 8
    exit 0
}

function Require([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Digest([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-New([string]$Path, [string]$Text) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
}
function Field($Value, [string]$Name) {
    if ($null -ne $Value -and $null -ne $Value.PSObject.Properties[$Name]) { return $Value.$Name }
    return $null
}

$run = $null
$commands = [Collections.Generic.List[object]]::new()
$receipt = [ordered]@{ schema = 1; status = 'BLOCKED'; scope = 'SAFE_ONLY'; release_qualified = $false; started_utc = [DateTime]::UtcNow.ToString('o'); plan = $plan; exclusions = $excluded; commands = $commands; errors = @() }
$before = $null
$productBefore = $null
$product = Join-Path $repo 'target\release\wardoff.exe'
$frozen = $null
$frozenBefore = $null
$appRoots = @()
$exitCode = 1

# Every child is created here. Timeout cleanup targets only its owned PID tree.
function Invoke-Owned([string]$Tool, [string[]]$Arguments, [string]$Label, [int]$Seconds, [switch]$ProductCommand) {
    $stem = '{0:d3}-{1}' -f $commands.Count, $Label
    $stdout = Join-Path $run ($stem + '.stdout.log')
    $stderr = Join-Path $run ($stem + '.stderr.log')
    $record = [ordered]@{ label = $Label; tool = $Tool; arguments = $Arguments; timeout_seconds = $Seconds; watchdog_triggered = $false; native_exit_code = $null; reaped = $false }
    $commands.Add($record)
    $process = [Diagnostics.Process]::new()
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Tool
    # Arguments are the fixed, whitespace-free tokens defined by this script.
    Require (-not ($Arguments | Where-Object { $_ -match '[\s"]' })) 'Unexpected command token.'
    $info.Arguments = $Arguments -join ' '
    $info.WorkingDirectory = $repo
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $info.EnvironmentVariables['CARGO_TARGET_DIR'] = Join-Path $repo 'target'
    $info.EnvironmentVariables['RUSTUP_AUTO_INSTALL'] = '0'
    $info.EnvironmentVariables['GIT_OPTIONAL_LOCKS'] = '0'
    $info.EnvironmentVariables.Remove('CARGO_BUILD_TARGET')
    foreach ($name in @('WARDOFF_REQUEST_LIMITS_CHILD', 'WARDOFF_STATUS_REPLY_CHILD', 'WARDOFF_IPC_BASELINE')) { $info.EnvironmentVariables.Remove($name) }
    if ($ProductCommand) { $info.EnvironmentVariables['LOCALAPPDATA'] = $appRoots[0]; $info.EnvironmentVariables['APPDATA'] = $appRoots[1] }
    $process.StartInfo = $info
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $outTask = $null; $errTask = $null
    try {
        Require ($process.Start()) 'Process did not start.'
        $record.pid = $process.Id
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($Seconds * 1000)) {
            $record.watchdog_triggered = $true
            $kill = [Diagnostics.ProcessStartInfo]::new()
            $kill.FileName = Join-Path $env:SystemRoot 'System32\taskkill.exe'
            $kill.Arguments = '/PID {0} /T /F' -f $process.Id
            $kill.UseShellExecute = $false; $kill.CreateNoWindow = $true
            $killer = [Diagnostics.Process]::Start($kill)
            try { Require ($killer.WaitForExit(10000)) 'Owned-tree cleanup timed out.'; $record.cleanup_exit_code = $killer.ExitCode } finally { $killer.Dispose() }
        }
        Require ($process.WaitForExit(10000)) 'Owned child was not reaped.'
        $record.reaped = $true
        $record.native_exit_code = $process.ExitCode
        Require ($outTask.Wait(10000) -and $errTask.Wait(10000)) 'Owned output handles did not close.'
    } finally {
        if ($null -ne $outTask -and -not $record.reaped -and -not $process.HasExited) {
            $process.Kill()
            $record.reaped = $process.WaitForExit(10000)
        }
        $record.elapsed_seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
        $outText = ''; $errText = ''
        if ($null -ne $outTask -and $outTask.IsCompleted) { $outText = $outTask.GetAwaiter().GetResult() }
        if ($null -ne $errTask -and $errTask.IsCompleted) { $errText = $errTask.GetAwaiter().GetResult() }
        Write-New $stdout $outText; Write-New $stderr $errText
        $record.stdout = @{ path = [IO.Path]::GetFileName($stdout); bytes = (Get-Item -LiteralPath $stdout).Length; sha256 = Digest $stdout }
        $record.stderr = @{ path = [IO.Path]::GetFileName($stderr); bytes = (Get-Item -LiteralPath $stderr).Length; sha256 = Digest $stderr }
        $process.Dispose()
    }
    Require (-not $record.watchdog_triggered -and $record.reaped -and $record.native_exit_code -eq 0) ('Command failed: ' + $Label)
    return [pscustomobject]@{ stdout = $outText; stderr = $errText }
}

function Source-State([string]$Label) {
    $status = (Invoke-Owned $git @('status', '--porcelain=v1', '--untracked-files=all') ($Label + '-status') 30).stdout
    Require ([string]::IsNullOrWhiteSpace($status)) 'Source checkout must be clean, including untracked files.'
    $head = (Invoke-Owned $git @('rev-parse', 'HEAD') ($Label + '-head') 30).stdout.Trim()
    $tree = (Invoke-Owned $git @('rev-parse', 'HEAD^{tree}') ($Label + '-tree') 30).stdout.Trim()
    $branch = (Invoke-Owned $git @('rev-parse', '--abbrev-ref', 'HEAD') ($Label + '-branch') 30).stdout.Trim()
    $names = (Invoke-Owned $git @('ls-files', '-z', '--cached') ($Label + '-files') 30).stdout.Split([char]0) | Where-Object { $_ -ne '' } | Sort-Object
    $hashes = [ordered]@{}
    foreach ($name in $names) {
        $path = [IO.Path]::GetFullPath((Join-Path $repo $name))
        Require ($path.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) 'Tracked path escaped checkout.'
        $hashes[$name] = Digest $path
    }
    return [ordered]@{ clean = $true; head = $head; tree = $tree; branch = $branch; source_file_count = $hashes.Count; source_hashes = $hashes }
}

try {
    Require ($env:OS -eq 'Windows_NT') 'Safe validation requires Windows.'
    if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repo 'target\validation-safe' }
    $root = [IO.Path]::GetFullPath($OutputDirectory)
    $run = Join-Path $root ('run-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N'))
    Require (-not (Test-Path -LiteralPath $run)) 'Run output already exists.'
    [void][IO.Directory]::CreateDirectory($run)
    $receipt.output_directory = $run
    $cargo = @(Get-Command cargo -CommandType Application -ErrorAction Stop)[0].Source
    $git = @(Get-Command git -CommandType Application -ErrorAction Stop)[0].Source
    $receipt.tools = @{ cargo = $cargo; git = $git }
    $before = Source-State 'before'; $receipt.source_before = $before
    $event = $null
    if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_EVENT_PATH)) { $event = Get-Content -Raw -LiteralPath $env:GITHUB_EVENT_PATH | ConvertFrom-Json }
    $pr = Field $event 'pull_request'
    $receipt.event = @{ name = $env:GITHUB_EVENT_NAME; sha = $env:GITHUB_SHA; ref = $env:GITHUB_REF; pr_head = (Field (Field $pr 'head') 'sha'); pr_base = (Field (Field $pr 'base') 'sha') }
    if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_SHA)) { Require ($before.head -eq $env:GITHUB_SHA) 'Checkout HEAD differs from event SHA.' }
    $toml = Get-Content -Raw -LiteralPath (Join-Path $repo 'Cargo.toml')
    $package = [regex]::Match($toml, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
    $versions = [regex]::Matches($package, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$')
    Require ($versions.Count -eq 1) 'Expected one numeric Cargo package version.'
    $version = $versions[0].Groups[1].Value
    $lock = Get-Content -Raw -LiteralPath (Join-Path $repo 'Cargo.lock')
    $packages = @([regex]::Matches($lock, '(?ms)^\[\[package\]\]\s*(.*?)(?=^\[\[package\]\]|\z)') | Where-Object { $_.Groups[1].Value -match '(?m)^name = "wardoff"\s*$' })
    Require ($packages.Count -eq 1 -and $packages[0].Groups[1].Value -match ('(?m)^version = "' + [regex]::Escape($version) + '"\s*$')) 'Cargo.lock package version differs.'
    $manifest = [Xml.XmlDocument]::new(); $manifest.XmlResolver = $null
    $manifest.Load((Join-Path $repo 'wardoff.manifest'))
    Require ($manifest.assembly.assemblyIdentity.version -eq ($version + '.0')) 'Source manifest version differs.'
    Require ($manifest.assembly.trustInfo.security.requestedPrivileges.requestedExecutionLevel.level -eq 'asInvoker') 'Unexpected elevation manifest.'
    Require ($manifest.assembly.trustInfo.security.requestedPrivileges.requestedExecutionLevel.uiAccess -ceq 'false') 'Unexpected UI access manifest.'
    $receipt.version = $version
    foreach ($step in $plan | Where-Object { $_.tool -eq 'cargo' }) {
        $result = Invoke-Owned $cargo $step.arguments $step.label $step.seconds
        if ($step.label -eq 'inventory') {
            $matching = @($result.stdout -split '\r?\n' | Where-Object { $_ -match ([regex]::Escape($nativeTest) + '.*: test$') })
            Require ($matching.Count -eq 1 -and $matching[0] -ceq ($nativeTest + ': test')) 'Native exclusion must appear exactly once under its full test name.'
            $receipt.native_exclusion_inventory_verified = $true
        }
    }
    Require (Test-Path -LiteralPath $product -PathType Leaf) 'Release build produced no ordinary executable.'
    $buildBytes = [IO.File]::ReadAllBytes($product)
    $productBefore = Digest $product
    $binaryDirectory = Join-Path $run 'binary'
    [void][IO.Directory]::CreateDirectory($binaryDirectory)
    $frozen = Join-Path $binaryDirectory 'wardoff.exe'
    $stream = [IO.File]::Open($frozen, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($buildBytes, 0, $buildBytes.Length) } finally { $stream.Dispose() }
    $frozenBefore = Digest $frozen
    Require ($frozenBefore -eq $productBefore) 'Frozen executable differs from the release build.'
    $receipt.binary = [ordered]@{ source_build_path = $product; frozen_path = $frozen; bytes = $buildBytes.Length; source_sha256_before = $productBefore; frozen_sha256_before = $frozenBefore; source_head = $before.head; source_tree = $before.tree; package_version = $version; pe_metadata_qualified = $false }
    $appRoots = @((Join-Path $run 'localappdata'), (Join-Path $run 'appdata'))
    foreach ($path in $appRoots) { [void][IO.Directory]::CreateDirectory($path) }
    $help = Invoke-Owned $frozen @('--help') 'help' 8 -ProductCommand
    Require ($help.stdout -match '(?i)wardoff' -and [string]::IsNullOrWhiteSpace($help.stderr)) 'Unexpected help output.'
    $reported = Invoke-Owned $frozen @('--version') 'version' 8 -ProductCommand
    Require ($reported.stdout.Trim() -ceq ('wardoff ' + $version) -and [string]::IsNullOrWhiteSpace($reported.stderr)) 'Unexpected version output.'
    $receipt.status = 'SAFE_PASS'
    $exitCode = 0
} catch {
    $receipt.errors += $_.Exception.Message
} finally {
    try {
        if ($null -ne $before) {
            $after = Source-State 'after'; $receipt.source_after = $after
            Require (($before | ConvertTo-Json -Depth 8 -Compress) -ceq ($after | ConvertTo-Json -Depth 8 -Compress)) 'Source changed during validation.'
            $receipt.source_stable = $true
        }
        if ($null -ne $productBefore -and $receipt.Contains('binary')) {
            $receipt.binary.source_sha256_after = Digest $product
            $receipt.binary.frozen_sha256_after = Digest $frozen
            Require ($receipt.binary.source_sha256_after -eq $productBefore -and $receipt.binary.frozen_sha256_after -eq $frozenBefore) 'Build or frozen executable changed during CLI probes.'
        }
        foreach ($path in $appRoots) { Require (@(Get-ChildItem -LiteralPath $path -Force -Recurse).Count -eq 0) 'CLI probes created application-data files.' }
        if ($appRoots.Count -eq 2) { $receipt.isolated_application_data_empty = $true }
    } catch { $receipt.errors += $_.Exception.Message; $exitCode = 1 }
    if ($receipt.errors.Count -gt 0) { $receipt.status = 'BLOCKED'; $exitCode = 1 }
    try {
        if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) {
            $summary = @(('Wardoff Safe validation: **' + $receipt.status + '**.'), '', 'Scope: **SAFE_ONLY**. `release_qualified=false`.', '', '| Excluded qualification | Status |', '| --- | --- |')
            $summary += @($excluded | ForEach-Object { '| ' + $_.name + ' | NOT_RUN |' })
            [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, (($summary -join [Environment]::NewLine) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
        }
    } catch { $receipt.errors += $_.Exception.Message; $receipt.status = 'BLOCKED'; $exitCode = 1 }
    $receipt.finished_utc = [DateTime]::UtcNow.ToString('o')
    if ($null -ne $run) { Write-New (Join-Path $run 'safe-validation.json') ($receipt | ConvertTo-Json -Depth 16) }
    [ordered]@{ status = $receipt.status; release_qualified = $false; receipt = $(if ($null -ne $run) { Join-Path $run 'safe-validation.json' } else { $null }); errors = $receipt.errors } | ConvertTo-Json -Depth 4
}
exit $exitCode
