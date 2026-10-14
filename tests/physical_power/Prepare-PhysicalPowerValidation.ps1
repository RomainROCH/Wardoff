[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$WardoffExecutable,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9A-Fa-f]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$OutputDirectory,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9A-Fa-f]{40}$')][string]$ApplicationSourceCommit,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9A-Fa-f]{40}$')][string]$AcceptedIntegrationCommit
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$resolve = {
    param([string]$Path)
    $provider = $null; $drive = $null
    $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path,[ref]$provider,[ref]$drive)
    if ($provider.Name -ne 'FileSystem') { throw 'Only FileSystem paths are allowed' }
    return $resolved
}
$source = & $resolve $WardoffExecutable
$kit = & $resolve $OutputDirectory
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Wardoff executable is missing' }
if (Test-Path -LiteralPath $kit) { throw 'Output directory already exists' }
$sourceSha256 = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToUpperInvariant()
if ($sourceSha256 -ne $ExpectedSha256.ToUpperInvariant()) { throw 'Wardoff executable hash mismatch' }
$collector = Join-Path $PSScriptRoot 'Collect-PhysicalPowerCheckpoint.ps1'
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$guide = Join-Path $repo 'docs\PHYSICAL_POWER_VALIDATION.md'
$matrix = Join-Path $repo 'docs\MVP_VALIDATION_MATRIX.md'
if (-not (Test-Path -LiteralPath $collector -PathType Leaf)) { throw 'Collector script is missing' }
if (-not (Test-Path -LiteralPath $guide -PathType Leaf)) { throw 'Physical power guide is missing' }
if (-not (Test-Path -LiteralPath $matrix -PathType Leaf)) { throw 'Validation matrix is missing' }
$preparedUtc = [DateTime]::UtcNow.ToString('o')
New-Item -ItemType Directory -Path $kit -Force:$false | Out-Null
New-Item -ItemType Directory -Path (Join-Path $kit 'baseline') -Force:$false | Out-Null
New-Item -ItemType Directory -Path (Join-Path $kit 'checkpoints') -Force:$false | Out-Null
Copy-Item -LiteralPath $source -Destination (Join-Path $kit 'wardoff.exe')
Copy-Item -LiteralPath $collector -Destination (Join-Path $kit 'Collect-PhysicalPowerCheckpoint.ps1')
Copy-Item -LiteralPath $guide -Destination (Join-Path $kit 'PHYSICAL_POWER_VALIDATION.md')
Copy-Item -LiteralPath $matrix -Destination (Join-Path $kit 'MVP_VALIDATION_MATRIX.md')
$actualSha256 = (Get-FileHash -LiteralPath (Join-Path $kit 'wardoff.exe') -Algorithm SHA256).Hash.ToUpperInvariant()
if ($actualSha256 -ne $ExpectedSha256.ToUpperInvariant()) { throw 'Copied Wardoff executable hash mismatch' }
function Save-BaselinePowerCfg {
    param([string]$Name, [string[]]$CommandArgs)
    $stdout = Join-Path (Join-Path $kit 'baseline') ($Name + '.stdout.txt')
    $stderr = Join-Path (Join-Path $kit 'baseline') ($Name + '.stderr.txt')
    $exitCode = 0
    $previous = $ErrorActionPreference
    try { $ErrorActionPreference = 'Continue'; & (Join-Path $env:SystemRoot 'System32\powercfg.exe') @CommandArgs 1> $stdout 2> $stderr; $exitCode = $LASTEXITCODE }
    catch { $_ | Out-File -LiteralPath $stderr -Encoding UTF8; $exitCode = 1 }
    finally { $ErrorActionPreference = $previous }
    return [ordered]@{ name=$Name; arguments=$CommandArgs; stdout=$stdout; stderr=$stderr; exitCode=$exitCode }
}
$baseline = @(
    (Save-BaselinePowerCfg 'powercfg-a' @('/a')),
    (Save-BaselinePowerCfg 'powercfg-getactivescheme' @('/getactivescheme')),
    (Save-BaselinePowerCfg 'powercfg-query' @('/query','SCHEME_CURRENT'))
)
$baseline | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $kit 'baseline\baseline.json') -Encoding UTF8
$cases = @()
foreach ($number in 1..8) { $cases += [ordered]@{ id=('S3-' + $number); status='pending'; observed=$null; notes=$null } }
$cases += [ordered]@{ id='Hibernate-explicit'; status='pending'; observed=$null; notes=$null }
$cases += [ordered]@{ id='Hibernate-resume'; status='pending'; observed=$null; notes=$null }
$cases += [ordered]@{ id='Modern-Standby-AC'; status='pending'; observed=$null; notes=$null }
$cases += [ordered]@{ id='Modern-Standby-DC'; status='pending'; observed=$null; notes=$null }
$manifest = [ordered]@{
    schema = 'wardoff-physical-power-validation-kit-v1'
    preparedUtc = $preparedUtc
    wardoffExecutable = (Join-Path $kit 'wardoff.exe')
    wardoffSha256 = $actualSha256
    applicationSourceCommit = $ApplicationSourceCommit
    acceptedIntegrationCommit = $AcceptedIntegrationCommit
    baseline = $baseline
    physicalTestsExecuted = $false
    cases = $cases
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $kit 'manifest.json') -Encoding UTF8
$results = [ordered]@{ schema='wardoff-physical-power-validation-results-v1'; physicalTestsExecuted=$false; cases=$cases }
$results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $kit 'results.json') -Encoding UTF8
Write-Output ('Prepared passive physical power kit: ' + $kit)
