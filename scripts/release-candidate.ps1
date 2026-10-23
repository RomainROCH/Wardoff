<#
.SYNOPSIS
Validates release provenance or packages a previously checked Windows binary.
.DESCRIPTION
Validate performs only Git/GitHub reads. Package never compiles or starts the
protection runtime. This script never creates a tag, release or asset remotely.
#>
[CmdletBinding()]
param(
    [ValidateSet('Validate', 'Package')][string] $Mode = 'Validate',
    [string] $TagName = $env:RELEASE_TAG,
    [string] $Repository = $env:GITHUB_REPOSITORY,
    [string] $CandidateCommit = $env:GITHUB_SHA,
    [string] $ContextPath,
    [string] $PackageDirectory,
    [string] $ExpectedSha256
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'release-guard.ps1')

function Invoke-ReleaseGit {
    param([string[]] $GitArgs)
    $result = @(& git -C $repoRoot @GitArgs)
    if ($LASTEXITCODE -ne 0) { throw 'Release Git read failed.' }
    return $result
}

function Read-ReleaseApiPages {
    param([string] $Endpoint)
    $json = & gh api --method GET --paginate --slurp $Endpoint
    if ($LASTEXITCODE -ne 0) { throw 'Release GitHub read failed; no fallback is allowed.' }
    return (ConvertFrom-ReleaseApiJson -Json ($json -join "`n"))
}

$head = @(Invoke-ReleaseGit @('rev-parse', 'HEAD'))[0]
if ($CandidateCommit -notmatch '^[0-9a-fA-F]{40}$' -or $head -ne $CandidateCommit) {
    throw 'Checkout is not the requested candidate commit.'
}
if (@(Invoke-ReleaseGit @('status', '--porcelain=v1')).Count -ne 0) { throw 'Release checkout is dirty.' }
if ([string]::IsNullOrWhiteSpace($ContextPath)) { throw 'A release context path is required.' }

if ($Mode -eq 'Validate') {
    # Validate names/versions before interpolating them in a Git or API argument.
    $null = Get-ReleaseVersion -RepoRoot $repoRoot -TagName $TagName
    if ($Repository -cne 'RomainROCH/Wardoff') { throw 'Release repository is not authorized.' }
    $remote = 'https://github.com/' + $Repository + '.git'
    $null = Invoke-ReleaseGit @('fetch', '--no-tags', $remote, '+refs/heads/main:refs/remotes/release/main')
    & git -C $repoRoot merge-base --is-ancestor $head refs/remotes/release/main
    $ancestryExit = $LASTEXITCODE
    if ($ancestryExit -notin @(0, 1)) { throw 'Main ancestry is indeterminate.' }
    $remoteRefs = Invoke-ReleaseGit @('ls-remote', '--exit-code', $remote, ('refs/tags/' + $TagName), ('refs/tags/' + $TagName + '^{}'))
    $tagCommit = Resolve-ReleaseTagCommit -TagName $TagName -RemoteRefs $remoteRefs
    $runsEndpoint = "repos/$Repository/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=$head&per_page=100"
    $runs = @(foreach ($page in @(Read-ReleaseApiPages $runsEndpoint)) { $page.workflow_runs })
    $ci = Select-ReleaseCiRun -CiRuns $runs -Repository $Repository -CandidateCommit $head
    $jobs = @(foreach ($page in @(Read-ReleaseApiPages "repos/$Repository/actions/runs/$($ci.id)/attempts/$($ci.run_attempt)/jobs?per_page=100")) { $page.jobs })
    $releases = @(foreach ($page in @(Read-ReleaseApiPages "repos/$Repository/releases?per_page=100")) { $page })
    $runs = @(foreach ($page in @(Read-ReleaseApiPages $runsEndpoint)) { $page.workflow_runs })
    $recheckedCi = Select-ReleaseCiRun -CiRuns $runs -Repository $Repository -CandidateCommit $head
    if ($recheckedCi.id -ne $ci.id -or $recheckedCi.run_attempt -ne $ci.run_attempt -or $recheckedCi.updated_at -cne $ci.updated_at) {
        throw 'CI evidence changed during release validation; retry only after it settles.'
    }
    $context = Assert-ReleaseCandidate -RepoRoot $repoRoot -Repository $Repository -TagName $TagName -CandidateCommit $head -RemoteTagCommit $tagCommit -MainContainsCandidate ($ancestryExit -eq 0) -CiRuns $runs -CiJobs $jobs -ExistingReleases $releases
    $context | ConvertTo-Json | Set-Content -LiteralPath $ContextPath -Encoding utf8
    Write-Output "PASS: $TagName at $head; main CI run $($context.CiRunId)."
    exit 0
}

$context = Get-Content -LiteralPath $ContextPath -Raw | ConvertFrom-Json
$version = Get-ReleaseVersion -RepoRoot $repoRoot -TagName $context.Tag
if ($context.Commit -ne $head -or $context.Tag -cne $TagName -or $context.Version -cne $version -or
    -not (Test-ReleasePositiveInteger $context.CiRunId)) { throw 'Release context differs from the checkout.' }
if ($env:GITHUB_RUN_ID -notmatch '^[1-9][0-9]*$') { throw 'Build run ID is missing.' }
if ($ExpectedSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'The pre-smoke binary hash is required.' }
. (Join-Path $repoRoot 'tests/smoke_helpers.ps1')
$binary = Join-Path $repoRoot 'target/release/wardoff.exe'
$hash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
if ($hash -ne $ExpectedSha256) { throw 'Smoke changed the release binary.' }
$versionOutput = @(& $binary --version)
if ($LASTEXITCODE -ne 0 -or ($versionOutput -join "`n").Trim() -cne "wardoff $version") { throw 'Release executable version differs.' }
$helpOutput = @(& $binary --help)
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace(($helpOutput -join "`n"))) { throw 'Release help failed.' }
[xml]$manifest = Get-EmbeddedManifestContent -Path $binary
$identity = $manifest.SelectNodes("/*[local-name()='assembly']/*[local-name()='assemblyIdentity']")
$level = $manifest.SelectNodes("//*[local-name()='requestedExecutionLevel']")
if ($identity.Count -ne 1 -or $identity[0].GetAttribute('version') -cne ($version + '.0') -or
    $level.Count -ne 1 -or $level[0].GetAttribute('level') -cne 'asInvoker' -or $level[0].GetAttribute('uiAccess') -cne 'false') {
    throw 'Embedded release manifest differs from the source contract.'
}
$null = Assert-ReleaseWindowsVersion -FileVersionInfo (Get-Item -LiteralPath $binary).VersionInfo -Version $version
$rustVersion = (& rustc -Vv) -join "`n"
if ($LASTEXITCODE -ne 0 -or $rustVersion -notmatch 'host: x86_64-pc-windows-msvc') { throw 'Release must use the MSVC host toolchain.' }
if (Test-Path -LiteralPath $PackageDirectory) { throw 'Package destination already exists.' }
$null = New-Item -ItemType Directory -Path $PackageDirectory
Copy-Item -LiteralPath $binary -Destination (Join-Path $PackageDirectory 'wardoff.exe')
$info = [ordered]@{
    tag = $TagName; source_commit = $head; version = $version; target = 'x86_64-pc-windows-msvc'
    rustc = $rustVersion; unsigned = $true; sha256 = $hash; build_run_id = [long]$env:GITHUB_RUN_ID
    run_url = "https://github.com/$Repository/actions/runs/$env:GITHUB_RUN_ID"
    ci_run_id = $context.CiRunId; ci_attempt = $context.CiAttempt
    tests = @('Full local checks', 'Developer tooling tests', 'Smoke safety tests', 'Release guard tests', 'Smoke tests with the same binary')
    limits = @('Review explicit smoke skips in the run log.', 'Physical S3/monitor, Hibernate and Modern Standby acceptance remains incomplete.', 'Unsigned; community support only.')
}
$info | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $PackageDirectory 'build-info.json') -Encoding utf8
"$hash  wardoff.exe" | Set-Content -LiteralPath (Join-Path $PackageDirectory 'SHA256SUMS.txt') -Encoding ascii
$null = Assert-ReleaseArtifact -PackageDirectory $PackageDirectory -CandidateCommit $head -TagName $TagName -ExpectedSha256 $ExpectedSha256 -ExpectedBuildRunId ([long]$env:GITHUB_RUN_ID)
Write-Output "PASS: exact unsigned MSVC package, SHA256 $hash."
