[CmdletBinding()]
param(
    [ValidateSet('Integration', 'Release')]
    [string] $Scope = 'Integration',
    [ValidateSet('success', 'failure', 'cancelled', 'skipped', 'not_run')]
    [string] $SafeConclusion = 'not_run',
    [string] $OutputDirectory = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot 'target/qualification'
}

# A candidate's JSON or an acknowledgement is not trusted execution evidence.
# No override is exposed: a reviewed importer of trusted runs is still required.
$source = [ordered]@{ commit = 'UNAVAILABLE'; tree = 'UNAVAILABLE'; clean = $false }
try {
    $git = (Get-Command git -CommandType Application -ErrorAction Stop).Source
    $commit = & $git --no-optional-locks -C $repoRoot rev-parse HEAD 2>$null
    if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[a-f0-9]{40}$') { throw 'Unavailable commit.' }
    $tree = & $git --no-optional-locks -C $repoRoot rev-parse 'HEAD^{tree}' 2>$null
    if ($LASTEXITCODE -ne 0 -or $tree -notmatch '^[a-f0-9]{40}$') { throw 'Unavailable tree.' }
    $status = @(& $git --no-optional-locks -C $repoRoot status --porcelain=v1 2>$null)
    if ($LASTEXITCODE -ne 0) { throw 'Unavailable checkout state.' }
    $source = [ordered]@{ commit = [string]$commit; tree = [string]$tree; clean = ($status.Count -eq 0) }
}
catch {
    # Qualification remains blocked when provenance cannot be read.
}

$safeStatus = if ($SafeConclusion -eq 'success') { 'PASS_SAFE_ONLY' } else { $SafeConclusion.ToUpperInvariant() }
$requirements = @(
    [ordered]@{ check = 'Safe'; status = $safeStatus; reason = 'A Safe step outcome does not qualify Full or a release.' },
    [ordered]@{ check = 'Native PowerRequest'; status = 'NOT_RUN'; reason = 'Excluded; requires explicit authorization for the named test on the chosen Windows host.' },
    [ordered]@{ check = 'System smoke'; status = 'NOT_RUN'; reason = 'Excluded; process ownership, exact task restoration and residual-state checks must be prepared first.' },
    [ordered]@{ check = 'Portable isolated CLI'; status = 'NOT_RUN'; reason = 'The external 13-case proof is bound to its earlier source and is not a portable CI job.' },
    [ordered]@{ check = 'Hardware/manual acceptance'; status = 'NOT_RUN'; reason = 'No physical sleep/resume or other required manual acceptance is established by Safe checks.' },
    [ordered]@{ check = 'Authenticated qualification provenance'; status = 'NOT_IMPLEMENTED'; reason = 'A trusted run/artifact importer must bind repository, workflow revision, run/attempt, commit, tree and tested binary hash.' }
)
if ($Scope -eq 'Release') {
    $requirements += [ordered]@{ check = 'Candidate version and exact package'; status = 'NOT_QUALIFIED'; reason = 'No new release version or exact qualified package is selected. Local development 0.2.0 must not replace the public asset.' }
}

$report = [ordered]@{
    schema = 1
    scope = $Scope
    status = 'BLOCKED'
    qualified = $false
    source = $source
    event = [ordered]@{ name = $env:GITHUB_EVENT_NAME; sha = $env:GITHUB_SHA; ref = $env:GITHUB_REF }
    safe_step_conclusion = $SafeConclusion
    requirements = $requirements
    native_tests_executed = $false
    system_smoke_executed = $false
    publication_performed = $false
}
$runDirectory = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) ('qualification-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($runDirectory)
$encoding = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText((Join-Path $runDirectory 'qualification.json'), (($report | ConvertTo-Json -Depth 8) + "`n"), $encoding)
$lines = @("## $Scope qualification: BLOCKED", '', 'Safe is a limited validation profile. Missing required evidence remains a failing check.', '', '| Check | Status | Reason |', '| --- | --- | --- |')
foreach ($requirement in $requirements) {
    $lines += '| ' + $requirement.check + ' | ' + $requirement.status + ' | ' + $requirement.reason + ' |'
}
$lines += @('', 'See docs/VALIDATION_GATES.md. No publication job is enabled. A candidate-provided PASS file cannot clear this gate.')
$summary = ($lines -join "`n") + "`n"
[IO.File]::WriteAllText((Join-Path $runDirectory 'summary.md'), $summary, $encoding)
if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) {
    [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $summary, $encoding)
}
Write-Output $summary
Write-Output ("Qualification receipt: " + (Join-Path $runDirectory 'qualification.json'))
exit 1
