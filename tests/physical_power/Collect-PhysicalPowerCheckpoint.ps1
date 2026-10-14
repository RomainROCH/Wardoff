[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$KitDirectory,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_.-]{0,47}$')][string]$Label
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$provider = $null; $drive = $null
$kit = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($KitDirectory,[ref]$provider,[ref]$drive)
if ($provider.Name -ne 'FileSystem') { throw 'Only FileSystem paths are allowed' }
$manifestPath = Join-Path $kit 'manifest.json'
$binary = Join-Path $kit 'wardoff.exe'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Kit manifest is missing' }
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Kit Wardoff binary is missing' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.schema -ne 'wardoff-physical-power-validation-kit-v1') { throw 'Kit manifest schema mismatch' }
$actualSha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToUpperInvariant()
if ($actualSha256 -ne ([string]$manifest.wardoffSha256).ToUpperInvariant()) { throw 'Kit Wardoff hash mismatch' }
$checkpoint = Join-Path (Join-Path $kit 'checkpoints') $Label
if (Test-Path -LiteralPath $checkpoint) { throw 'Checkpoint already exists' }
New-Item -ItemType Directory -Path $checkpoint -Force:$false | Out-Null
function Save-PowerCfg {
    param([string]$Name, [string[]]$CommandArgs)
    $stdout = Join-Path $checkpoint ($Name + '.stdout.txt')
    $stderr = Join-Path $checkpoint ($Name + '.stderr.txt')
    $exitCode = 0
    $previous = $ErrorActionPreference
    try { $ErrorActionPreference = 'Continue'; & (Join-Path $env:SystemRoot 'System32\powercfg.exe') @CommandArgs 1> $stdout 2> $stderr; $exitCode = $LASTEXITCODE }
    catch { $_ | Out-File -LiteralPath $stderr -Encoding UTF8; $exitCode = 1 }
    finally { $ErrorActionPreference = $previous }
    return [ordered]@{ name=$Name; arguments=$CommandArgs; stdout=$stdout; stderr=$stderr; exitCode=$exitCode }
}
$captures = @(
    (Save-PowerCfg 'powercfg-a' @('/a')),
    (Save-PowerCfg 'powercfg-getactivescheme' @('/getactivescheme')),
    (Save-PowerCfg 'powercfg-query' @('/query','SCHEME_CURRENT')),
    (Save-PowerCfg 'powercfg-requests' @('/requests')),
    (Save-PowerCfg 'powercfg-requestsoverride' @('/requestsoverride'))
)
$eventStatus = @(); $events = @()
$start = [DateTime]::Parse($manifest.preparedUtc).ToUniversalTime()
foreach ($provider in @('Microsoft-Windows-Kernel-Power','Microsoft-Windows-Power-Troubleshooter')) {
    try {
        $providerEvents = @(Get-WinEvent -FilterHashtable @{ LogName='System'; ProviderName=$provider; StartTime=$start } -MaxEvents 100 -ErrorAction Stop |
            Select-Object -First 100 Id,RecordId,@{Name='TimeCreatedUtc';Expression={$_.TimeCreated.ToUniversalTime().ToString('o')}})
        $events += $providerEvents
        $eventStatus += [ordered]@{ provider=$provider; status='captured'; count=$providerEvents.Count; error=$null }
    } catch {
        $id = $_.FullyQualifiedErrorId
        if ($id -match 'NoMatchingEventsFound') {
            $eventStatus += [ordered]@{ provider=$provider; status='no-events'; count=0; error=$null }
        } else {
            $eventStatus += [ordered]@{ provider=$provider; status='error'; count=0; error=$_.Exception.GetType().Name }
        }
    }
}
ConvertTo-Json -InputObject @($events) -Depth 4 | Set-Content -LiteralPath (Join-Path $checkpoint 'power-events.json') -Encoding UTF8
$record = [ordered]@{ schema='wardoff-physical-power-checkpoint-v1'; utc=[DateTime]::UtcNow.ToString('o'); label=$Label; wardoffSha256=$actualSha256; captures=$captures; eventLog=$eventStatus; physicalTransitionObserved=$false }
$record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $checkpoint 'checkpoint.json') -Encoding UTF8
Write-Output ('Created passive checkpoint: ' + $checkpoint)
