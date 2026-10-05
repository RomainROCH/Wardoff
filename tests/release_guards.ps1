[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$guardPath = Join-Path $PSScriptRoot '..\scripts\release-guard.ps1'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('wardoff-release-guard-' + [Guid]::NewGuid().ToString('N'))
$repoFixture = Join-Path $fixtureRoot 'repo'
$packageFixture = Join-Path $fixtureRoot 'package'
$repository = 'RomainROCH/Wardoff'
$candidateCommit = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
$otherCommit = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
$tagName = 'v0.2.1'
$script:caseCount = 0

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Assert-Equal {
    param($Actual, $Expected, [string]$Message)
    if ($Actual -cne $Expected) { throw "$Message Expected '$Expected', received '$Actual'." }
}

function Assert-Throws {
    param([scriptblock]$Action)
    $caught = $false
    try { & $Action | Out-Null } catch { $caught = $true }
    if (-not $caught) { throw 'Expected the release guard to reject the fixture.' }
}

function Invoke-TestCase {
    param([string]$Name, [scriptblock]$Action)
    try { & $Action } catch { throw "Release guard test failed [$Name]: $($_.Exception.Message)" }
    $script:caseCount++
}

function Write-VersionFixture {
    param(
        [string]$TomlVersion = '0.2.1',
        [string]$LockVersion = '0.2.1',
        [string]$ManifestVersion = '0.2.1.0',
        [string]$Level = 'asInvoker',
        [string]$UiAccess = 'false',
        [string]$PackageName = 'wardoff',
        [string]$LockPackageName = 'wardoff'
    )
    New-Item -ItemType Directory -Path $repoFixture -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $repoFixture 'Cargo.toml') -Encoding utf8 -Value @"
[package]
name = "$PackageName"
version = "$TomlVersion"
edition = "2021"

[dependencies]
some_dependency = "9.8.7"
"@
    Set-Content -LiteralPath (Join-Path $repoFixture 'Cargo.lock') -Encoding utf8 -Value @"
version = 4

[[package]]
name = "some_dependency"
version = "9.8.7"

[[package]]
name = "$LockPackageName"
version = "$LockVersion"
dependencies = ["some_dependency"]
"@
    Set-Content -LiteralPath (Join-Path $repoFixture 'wardoff.manifest') -Encoding utf8 -Value @"
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="$ManifestVersion" processorArchitecture="*" name="wardoff" type="win32" />
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="$Level" uiAccess="$UiAccess" /></requestedPrivileges></security>
  </trustInfo>
</assembly>
"@
}

function New-CiRun {
    param(
        [long]$Id = 73,
        [string]$Commit = $candidateCommit,
        [string]$Branch = 'main',
        [string]$Event = 'push',
        [string]$Path = '.github/workflows/ci.yml',
        [string]$RepositoryName = $repository,
        [string]$Status = 'completed',
        [string]$Conclusion = 'success',
        [int]$Attempt = 1,
        [string]$UpdatedAt
    )
    if (-not $PSBoundParameters.ContainsKey('UpdatedAt')) {
        $UpdatedAt = [datetime]::new(2026, 1, 1, 0, 0, 0, [DateTimeKind]::Utc).AddSeconds($Id).ToString('yyyy-MM-ddTHH:mm:ssZ', [Globalization.CultureInfo]::InvariantCulture)
    }
    return [pscustomobject]@{
        id = $Id; head_sha = $Commit; head_branch = $Branch; event = $Event
        path = $Path; head_repository = [pscustomobject]@{ full_name = $RepositoryName }
        status = $Status; conclusion = $Conclusion; run_attempt = $Attempt; updated_at = $UpdatedAt
    }
}

function New-CiJob {
    param([long]$RunId = 73)
    return [pscustomobject]@{
        name = 'ci (windows-latest, stable)'; run_id = $RunId; head_sha = $candidateCommit; status = 'completed'; conclusion = 'success'
        steps = @('Full local checks', 'Developer tooling tests', 'Smoke safety tests', 'Release guard tests', 'Smoke tests') | ForEach-Object {
            [pscustomobject]@{ name = $_; status = 'completed'; conclusion = 'success' }
        }
    }
}

function Invoke-CandidateFixture {
    param([hashtable]$Overrides = @{})
    $arguments = @{
        RepoRoot = $repoFixture; Repository = $repository; TagName = $tagName
        CandidateCommit = $candidateCommit; RemoteTagCommit = $candidateCommit
        MainContainsCandidate = $true; CiRuns = @((New-CiRun)); CiJobs = @((New-CiJob))
        ExistingReleases = @()
    }
    foreach ($key in $Overrides.Keys) { $arguments[$key] = $Overrides[$key] }
    return Assert-ReleaseCandidate @arguments
}

function Write-ArtifactFixture {
    New-Item -ItemType Directory -Path $packageFixture -Force | Out-Null
    # Deliberately not an executable. These tests only validate bytes and metadata.
    [IO.File]::WriteAllBytes((Join-Path $packageFixture 'wardoff.exe'), [Text.Encoding]::ASCII.GetBytes('release guard fixture - never execute'))
    $hash = (Get-FileHash -LiteralPath (Join-Path $packageFixture 'wardoff.exe') -Algorithm SHA256).Hash
    $info = [pscustomobject]@{
        source_commit = $candidateCommit; tag = $tagName; version = '0.2.1'
        target = 'x86_64-pc-windows-msvc'; unsigned = $true
        build_run_id = 91; ci_run_id = 73; sha256 = $hash
    }
    Write-ArtifactInfo -Info $info
    Set-Content -LiteralPath (Join-Path $packageFixture 'SHA256SUMS.txt') -Encoding ascii -Value "$hash  wardoff.exe"
    return $info
}

function Write-ArtifactInfo {
    param($Info)
    $Info | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $packageFixture 'build-info.json') -Encoding utf8
}

function Invoke-ArtifactFixture {
    param([string]$ExpectedSha256, [hashtable]$Overrides = @{})
    $arguments = @{
        PackageDirectory = $packageFixture; CandidateCommit = $candidateCommit; TagName = $tagName
        ExpectedSha256 = $ExpectedSha256; ExpectedBuildRunId = 91
    }
    foreach ($key in $Overrides.Keys) { $arguments[$key] = $Overrides[$key] }
    Assert-ReleaseArtifact @arguments | Out-Null
}

try {
    Invoke-TestCase 'exception assertion rejects a non-throwing action' {
        $canaryCaught = $false
        try { Assert-Throws {} } catch { $canaryCaught = $true }
        Assert-True $canaryCaught 'Assert-Throws accepted an action that did not throw.'
    }
    if (-not (Test-Path -LiteralPath $guardPath -PathType Leaf)) {
        throw 'Release guard implementation is missing: scripts/release-guard.ps1.'
    }
    . $guardPath
    foreach ($functionName in @('Get-ReleaseVersion', 'Resolve-ReleaseTagCommit', 'Select-ReleaseCiRun', 'Assert-ReleaseDraftAvailable', 'Assert-ReleaseCandidate', 'Assert-ReleaseArtifact')) {
        if (-not (Get-Command -Name $functionName -CommandType Function -ErrorAction SilentlyContinue)) {
            throw "Release guard function is missing: $functionName."
        }
    }
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    Write-VersionFixture

    Invoke-TestCase 'matching package lock and namespaced manifest' {
        Assert-Equal (Get-ReleaseVersion -RepoRoot $repoFixture -TagName $tagName) '0.2.1' 'Matching versions were rejected.'
    }
    foreach ($invalidTag in @('', '0.2.1', 'V0.2.1', 'v00.2.1', 'v0.02.1', 'v0.2.01', 'v0.2', 'v0.2.1.0', 'v0.2.1-beta.1', 'v0.2.1+build', ' v0.2.1', 'v0.2.1 ', "v0.2.1`n")) {
        Invoke-TestCase "strict semantic tag rejects [$invalidTag]" {
            Assert-Throws { Get-ReleaseVersion -RepoRoot $repoFixture -TagName $invalidTag }
        }
    }
    Invoke-TestCase 'zero components remain valid' {
        Write-VersionFixture -TomlVersion '0.0.0' -LockVersion '0.0.0' -ManifestVersion '0.0.0.0'
        Assert-Equal (Get-ReleaseVersion -RepoRoot $repoFixture -TagName 'v0.0.0') '0.0.0' 'Zero components were rejected.'
    }
    foreach ($versions in @(
        @{ TomlVersion = '0.2.0' }, @{ LockVersion = '0.2.0' }, @{ ManifestVersion = '0.2.0.0' },
        @{ ManifestVersion = '0.2.1.1' }, @{ ManifestVersion = '0.2.1' }, @{ Level = 'requireAdministrator' }, @{ UiAccess = 'true' },
        @{ PackageName = 'other' }, @{ LockPackageName = 'other' }
    )) {
        Invoke-TestCase ('version fixture rejects ' + ($versions.Keys -join ',')) {
            Write-VersionFixture @versions
            Assert-Throws { Get-ReleaseVersion -RepoRoot $repoFixture -TagName $tagName }
        }
    }
    foreach ($missingFile in @('Cargo.toml', 'Cargo.lock', 'wardoff.manifest')) {
        Invoke-TestCase "missing version file $missingFile" {
            Write-VersionFixture
            Remove-Item -LiteralPath (Join-Path $repoFixture $missingFile)
            Assert-Throws { Get-ReleaseVersion -RepoRoot $repoFixture -TagName $tagName }
        }
    }
    Invoke-TestCase 'malformed manifest' {
        Write-VersionFixture
        Set-Content -LiteralPath (Join-Path $repoFixture 'wardoff.manifest') -Encoding utf8 -Value '<assembly><broken>'
        Assert-Throws { Get-ReleaseVersion -RepoRoot $repoFixture -TagName $tagName }
    }
    Write-VersionFixture

    Invoke-TestCase 'lightweight remote tag' {
        Assert-Equal (Resolve-ReleaseTagCommit -TagName $tagName -RemoteRefs @("$candidateCommit`trefs/tags/$tagName")) $candidateCommit 'Lightweight tag SHA differs.'
    }
    Invoke-TestCase 'annotated remote tag uses peeled commit' {
        $refs = @("$otherCommit`trefs/tags/$tagName", "$candidateCommit`trefs/tags/$tagName^{}")
        Assert-Equal (Resolve-ReleaseTagCommit -TagName $tagName -RemoteRefs $refs) $candidateCommit 'Annotated tag was not peeled.'
    }
    foreach ($remoteRefs in @(
        @{ Name = 'missing'; Refs = @() },
        @{ Name = 'different tag'; Refs = @("$candidateCommit`trefs/tags/v0.2.10") },
        @{ Name = 'short hash'; Refs = @("aaaa`trefs/tags/$tagName") },
        @{ Name = 'non hexadecimal hash'; Refs = @('gggggggggggggggggggggggggggggggggggggggg refs/tags/v0.2.1') },
        @{ Name = 'malformed line'; Refs = @("$candidateCommit refs/tags/$tagName extra") },
        @{ Name = 'conflicting duplicate'; Refs = @("$candidateCommit`trefs/tags/$tagName", "$otherCommit`trefs/tags/$tagName") }
    )) {
        Invoke-TestCase ('remote tag rejects ' + $remoteRefs.Name) {
            Assert-Throws { Resolve-ReleaseTagCommit -TagName $tagName -RemoteRefs $remoteRefs.Refs }
        }
    }

    Invoke-TestCase 'exact successful main push CI' {
        $run = Select-ReleaseCiRun -CiRuns @((New-CiRun)) -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 73 'The expected CI run was not selected.'
    }
    Invoke-TestCase 'main workflow path suffix' {
        $run = Select-ReleaseCiRun -CiRuns @((New-CiRun -Path '.github/workflows/ci.yml@refs/heads/main')) -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 73 'The authorized workflow suffix was rejected.'
    }
    foreach ($runArguments in @(
        @{ Commit = $otherCommit }, @{ Branch = 'dev' }, @{ Event = 'pull_request' },
        @{ Path = '.github/workflows/release.yml' }, @{ Path = '.github/workflows/ci.yml@refs/heads/dev' },
        @{ Path = '.github/workflows/ci.yml@refs/heads/main/extra' }, @{ RepositoryName = 'other/Wardoff' },
        @{ Attempt = 0 }, @{ Status = 'in_progress'; Conclusion = '' }, @{ Conclusion = 'failure' },
        @{ Conclusion = 'cancelled' }, @{ Conclusion = 'skipped' }
    )) {
        Invoke-TestCase ('CI rejects ' + ($runArguments.Keys -join ',')) {
            Assert-Throws { Select-ReleaseCiRun -CiRuns @((New-CiRun @runArguments)) -Repository $repository -CandidateCommit $candidateCommit }
        }
    }
    Invoke-TestCase 'CI evidence absent' {
        Assert-Throws { Select-ReleaseCiRun -CiRuns @() -Repository $repository -CandidateCommit $candidateCommit }
    }
    foreach ($invalidRunId in @(0, -1, $true, '73')) {
        Invoke-TestCase "CI run ID rejects invalid value [$invalidRunId]" {
            $run = New-CiRun
            $run.id = $invalidRunId
            Assert-Throws { Select-ReleaseCiRun -CiRuns @($run) -Repository $repository -CandidateCommit $candidateCommit }
        }
    }
    Invoke-TestCase 'newest eligible run is selected by update timestamp' {
        $run = Select-ReleaseCiRun -CiRuns @((New-CiRun -Id 9), (New-CiRun -Id 101), (New-CiRun -Id 88)) -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 101 'The newest CI run was not selected.'
    }
    Invoke-TestCase 'recent failed rerun of lower ID invalidates higher ID green' {
        $runs = @(
            (New-CiRun -Id 74 -UpdatedAt '2026-10-05T01:00:00Z'),
            (New-CiRun -Id 70 -Attempt 2 -Conclusion 'failure' -UpdatedAt '2026-10-05T02:00:00Z')
        )
        Assert-Throws { Select-ReleaseCiRun -CiRuns $runs -Repository $repository -CandidateCommit $candidateCommit }
    }
    Invoke-TestCase 'recent running rerun of lower ID invalidates higher ID green' {
        $runs = @(
            (New-CiRun -Id 74 -UpdatedAt '2026-10-05T01:00:00Z'),
            (New-CiRun -Id 70 -Attempt 2 -Status 'in_progress' -Conclusion '' -UpdatedAt '2026-10-05T02:00:00Z')
        )
        Assert-Throws { Select-ReleaseCiRun -CiRuns $runs -Repository $repository -CandidateCommit $candidateCommit }
    }
    Invoke-TestCase 'recent successful rerun of lower ID is selected' {
        $runs = @(
            (New-CiRun -Id 74 -UpdatedAt '2026-10-05T01:00:00Z'),
            (New-CiRun -Id 70 -Attempt 2 -UpdatedAt '2026-10-05T02:00:00Z')
        )
        $run = Select-ReleaseCiRun -CiRuns $runs -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 70 'The most recently updated rerun was not selected.'
        Assert-Equal $run.run_attempt 2 'The expected rerun attempt was not selected.'
    }
    Invoke-TestCase 'equal update timestamps use run ID as tie breaker' {
        $runs = @((New-CiRun -Id 70 -UpdatedAt '2026-10-05T01:00:00Z'), (New-CiRun -Id 74 -UpdatedAt '2026-10-05T01:00:00Z'))
        $run = Select-ReleaseCiRun -CiRuns $runs -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 74 'Equal timestamps were not ordered by run ID.'
    }
    Invoke-TestCase 'equal update timestamp and ID use attempt as tie breaker' {
        $runs = @((New-CiRun -Id 74 -Attempt 1 -UpdatedAt '2026-10-05T01:00:00Z'), (New-CiRun -Id 74 -Attempt 2 -UpdatedAt '2026-10-05T01:00:00Z'))
        $run = Select-ReleaseCiRun -CiRuns $runs -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.run_attempt 2 'Equal timestamps and IDs were not ordered by attempt.'
    }
    Invoke-TestCase 'CI run without update timestamp is rejected' {
        $run = New-CiRun
        $run.PSObject.Properties.Remove('updated_at')
        Assert-Throws { Select-ReleaseCiRun -CiRuns @($run) -Repository $repository -CandidateCommit $candidateCommit }
    }
    foreach ($invalidUpdatedAt in @('', 'not-a-timestamp', '2026-13-05T01:00:00Z', '2026-10-05T01:00:00')) {
        Invoke-TestCase "CI run rejects malformed update timestamp [$invalidUpdatedAt]" {
            Assert-Throws { Select-ReleaseCiRun -CiRuns @((New-CiRun -UpdatedAt $invalidUpdatedAt)) -Repository $repository -CandidateCommit $candidateCommit }
        }
    }
    foreach ($latestConclusion in @('failure', 'cancelled', 'skipped')) {
        Invoke-TestCase "newer $latestConclusion run invalidates old green" {
            Assert-Throws { Select-ReleaseCiRun -CiRuns @((New-CiRun -Id 70), (New-CiRun -Id 74 -Conclusion $latestConclusion)) -Repository $repository -CandidateCommit $candidateCommit }
        }
    }
    Invoke-TestCase 'newer running CI invalidates old green' {
        Assert-Throws { Select-ReleaseCiRun -CiRuns @((New-CiRun -Id 70), (New-CiRun -Id 74 -Status 'in_progress' -Conclusion '')) -Repository $repository -CandidateCommit $candidateCommit }
    }
    Invoke-TestCase 'unrelated newer commit does not replace exact SHA evidence' {
        $run = Select-ReleaseCiRun -CiRuns @((New-CiRun -Id 70), (New-CiRun -Id 74 -Commit $otherCommit -Conclusion 'failure')) -Repository $repository -CandidateCommit $candidateCommit
        Assert-Equal $run.id 70 'Unrelated SHA replaced exact candidate evidence.'
    }

    Invoke-TestCase 'no existing release' { Assert-ReleaseDraftAvailable -TagName $tagName -Releases @() }
    foreach ($draftState in @($true, $false)) {
        Invoke-TestCase "existing release rejected draft=$draftState" {
            Assert-Throws { Assert-ReleaseDraftAvailable -TagName $tagName -Releases @([pscustomobject]@{ tag_name = $tagName; draft = $draftState; assets = @() }) }
        }
    }
    Invoke-TestCase 'release for other tag allowed' {
        Assert-ReleaseDraftAvailable -TagName $tagName -Releases @([pscustomobject]@{ tag_name = 'v0.2.0'; draft = $false; assets = @([pscustomobject]@{ name = 'wardoff.exe' }) })
    }

    Invoke-TestCase 'complete candidate returns bound metadata' {
        $candidate = Invoke-CandidateFixture
        Assert-Equal $candidate.Tag $tagName 'Candidate tag differs.'
        Assert-Equal $candidate.Version '0.2.1' 'Candidate version differs.'
        Assert-Equal $candidate.Commit $candidateCommit 'Candidate commit differs.'
        Assert-Equal $candidate.CiRunId 73 'Candidate CI run differs.'
    }
    foreach ($overrides in @(
        @{ Repository = 'other/Wardoff' }, @{ CandidateCommit = 'aaaa' },
        @{ CandidateCommit = 'gggggggggggggggggggggggggggggggggggggggg' },
        @{ RemoteTagCommit = 'aaaa' }, @{ RemoteTagCommit = $otherCommit },
        @{ MainContainsCandidate = $false }, @{ CiRuns = @() }, @{ CiJobs = @() }
    )) {
        Invoke-TestCase ('candidate rejects ' + ($overrides.Keys -join ',')) {
            Assert-Throws { Invoke-CandidateFixture -Overrides $overrides }
        }
    }
    Invoke-TestCase 'moved annotated tag invalidates tested commit' {
        $movedTag = Resolve-ReleaseTagCommit -TagName $tagName -RemoteRefs @("$candidateCommit`trefs/tags/$tagName", "$otherCommit`trefs/tags/$tagName^{}")
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ RemoteTagCommit = $movedTag } }
    }
    Invoke-TestCase 'candidate rejects CI job for another commit' {
        $job = New-CiJob
        $job.head_sha = $otherCommit
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ CiJobs = @($job) } }
    }
    Invoke-TestCase 'candidate rejects CI job without commit SHA' {
        $job = New-CiJob
        $job.PSObject.Properties.Remove('head_sha')
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ CiJobs = @($job) } }
    }
    foreach ($jobProblem in @('wrong run', 'wrong job', 'failure', 'running')) {
        Invoke-TestCase "candidate rejects $jobProblem" {
            $job = New-CiJob
            switch ($jobProblem) {
                'wrong run' { $job.run_id = 74 }
                'wrong job' { $job.name = 'ci (ubuntu-latest, stable)' }
                'failure' { $job.conclusion = 'failure' }
                'running' { $job.status = 'in_progress' }
            }
            Assert-Throws { Invoke-CandidateFixture -Overrides @{ CiJobs = @($job) } }
        }
    }
    foreach ($stepName in @('Full local checks', 'Developer tooling tests', 'Smoke safety tests', 'Release guard tests', 'Smoke tests')) {
        foreach ($stepProblem in @('missing', 'failure', 'skipped', 'running')) {
            Invoke-TestCase "candidate rejects $stepName $stepProblem" {
                $job = New-CiJob
                if ($stepProblem -eq 'missing') {
                    $job.steps = @($job.steps | Where-Object { $_.name -cne $stepName })
                } else {
                    $step = $job.steps | Where-Object { $_.name -ceq $stepName }
                    if ($stepProblem -eq 'running') { $step.status = 'in_progress' } else { $step.conclusion = $stepProblem }
                }
                Assert-Throws { Invoke-CandidateFixture -Overrides @{ CiJobs = @($job) } }
            }
        }
    }
    Invoke-TestCase 'other failed CI job invalidates candidate' {
        $failedJob = [pscustomobject]@{ name = 'additional checks'; run_id = 73; status = 'completed'; conclusion = 'failure'; steps = @() }
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ CiJobs = @((New-CiJob), $failedJob) } }
    }
    Invoke-TestCase 'existing public candidate release cannot be overwritten' {
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ ExistingReleases = @([pscustomobject]@{ tag_name = $tagName; draft = $false; assets = @([pscustomobject]@{ name = 'wardoff.exe' }) }) } }
    }
    Invoke-TestCase 'existing draft candidate release cannot be overwritten' {
        Assert-Throws { Invoke-CandidateFixture -Overrides @{ ExistingReleases = @([pscustomobject]@{ tag_name = $tagName; draft = $true; assets = @() }) } }
    }
    Invoke-TestCase 'candidate enforces the version fixture guard' {
        Write-VersionFixture -LockVersion '0.2.0'
        Assert-Throws { Invoke-CandidateFixture }
        Write-VersionFixture
    }

    Invoke-TestCase 'exact artifact hash and metadata' {
        $info = Write-ArtifactFixture
        Invoke-ArtifactFixture -ExpectedSha256 $info.sha256
    }
    foreach ($propertyCase in @(
        @{ Field = 'source_commit'; Value = $otherCommit }, @{ Field = 'tag'; Value = 'v0.2.0' },
        @{ Field = 'version'; Value = '0.2.0' }, @{ Field = 'target'; Value = 'x86_64-pc-windows-gnu' },
        @{ Field = 'unsigned'; Value = $false }, @{ Field = 'unsigned'; Value = 'true' },
        @{ Field = 'build_run_id'; Value = 92 }, @{ Field = 'build_run_id'; Value = 0 },
        @{ Field = 'build_run_id'; Value = -1 }, @{ Field = 'build_run_id'; Value = $true },
        @{ Field = 'build_run_id'; Value = '91' }, @{ Field = 'ci_run_id'; Value = 0 },
        @{ Field = 'ci_run_id'; Value = -1 }, @{ Field = 'ci_run_id'; Value = $true },
        @{ Field = 'ci_run_id'; Value = '73' }, @{ Field = 'sha256'; Value = ('0' * 64) }
    )) {
        Invoke-TestCase ('artifact rejects ' + $propertyCase.Field + '=' + $propertyCase.Value) {
            $info = Write-ArtifactFixture
            $expectedHash = $info.sha256
            $info.($propertyCase.Field) = $propertyCase.Value
            Write-ArtifactInfo -Info $info
            Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $expectedHash }
        }
    }
    foreach ($fieldName in @('source_commit', 'tag', 'version', 'target', 'unsigned', 'build_run_id', 'ci_run_id', 'sha256')) {
        Invoke-TestCase "artifact rejects missing metadata $fieldName" {
            $info = Write-ArtifactFixture
            $expectedHash = $info.sha256
            $info.PSObject.Properties.Remove($fieldName)
            Write-ArtifactInfo -Info $info
            Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $expectedHash }
        }
    }
    Invoke-TestCase 'artifact bytes changed after verification' {
        $info = Write-ArtifactFixture
        [IO.File]::WriteAllBytes((Join-Path $packageFixture 'wardoff.exe'), [byte[]](1, 2, 3))
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
    }
    Invoke-TestCase 'trusted output hash differs from artifact and metadata' {
        $info = Write-ArtifactFixture
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 ('0' * 64) }
    }
    Invoke-TestCase 'expected build run differs' {
        $info = Write-ArtifactFixture
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 -Overrides @{ ExpectedBuildRunId = 92 } }
    }
    Invoke-TestCase 'malformed artifact metadata' {
        $info = Write-ArtifactFixture
        Set-Content -LiteralPath (Join-Path $packageFixture 'build-info.json') -Encoding utf8 -Value '{broken'
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
    }
    foreach ($missingArtifact in @('wardoff.exe', 'build-info.json', 'SHA256SUMS.txt')) {
        Invoke-TestCase "artifact rejects missing $missingArtifact" {
            $info = Write-ArtifactFixture
            Remove-Item -LiteralPath (Join-Path $packageFixture $missingArtifact)
            Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
        }
    }
    Invoke-TestCase 'checksum file hash changed independently' {
        $info = Write-ArtifactFixture
        Set-Content -LiteralPath (Join-Path $packageFixture 'SHA256SUMS.txt') -Encoding ascii -Value ((('0' * 64) + '  wardoff.exe'))
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
    }
    Invoke-TestCase 'checksum file names another asset' {
        $info = Write-ArtifactFixture
        Set-Content -LiteralPath (Join-Path $packageFixture 'SHA256SUMS.txt') -Encoding ascii -Value "$($info.sha256)  other.exe"
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
    }
    Invoke-TestCase 'checksum file contains multiple lines' {
        $info = Write-ArtifactFixture
        Set-Content -LiteralPath (Join-Path $packageFixture 'SHA256SUMS.txt') -Encoding ascii -Value @("$($info.sha256)  wardoff.exe", "$($info.sha256)  other.exe")
        Assert-Throws { Invoke-ArtifactFixture -ExpectedSha256 $info.sha256 }
    }

    Invoke-TestCase 'valid transfer inputs produce no output' {
        $result = @(Assert-ReleaseTransferInputs -ArtifactId '123' -Sha256 ('a' * 64))
        Assert-Equal $result.Count 0 'The transfer input guard produced unexpected output.'
    }
    foreach ($invalidArtifactId in @('', '0', '-1', 'not-an-id', '1.5', '1e3', ' 73', '73 ')) {
        Invoke-TestCase "transfer guard rejects artifact ID [$invalidArtifactId]" {
            Assert-Throws { Assert-ReleaseTransferInputs -ArtifactId $invalidArtifactId -Sha256 ('a' * 64) }
        }
    }
    foreach ($invalidTransferHash in @('', 'abc', ('0' * 63), ('0' * 65), ('g' * 64))) {
        Invoke-TestCase "transfer guard rejects malformed hash [$invalidTransferHash]" {
            Assert-Throws { Assert-ReleaseTransferInputs -ArtifactId '123' -Sha256 $invalidTransferHash }
        }
    }

    Invoke-TestCase 'PowerShell parser accepts guard and tests' {
        foreach ($path in @($guardPath, $PSCommandPath)) {
            $syntaxErrors = $null
            [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$syntaxErrors) | Out-Null
            if ($syntaxErrors) { throw (($syntaxErrors | ForEach-Object { $_.Message }) -join [Environment]::NewLine) }
        }
    }
    Write-Output "Release guard tests passed ($script:caseCount cases)."
} finally {
    # Remove only this test's generated directory after checking its resolved boundary.
    if (Test-Path -LiteralPath $fixtureRoot) {
        $resolvedFixture = (Resolve-Path -LiteralPath $fixtureRoot).Path
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
        $fixtureName = Split-Path -Leaf $resolvedFixture
        if (-not ($resolvedFixture.StartsWith($tempRoot + '\', [StringComparison]::OrdinalIgnoreCase) -and
            $fixtureName.StartsWith('wardoff-release-guard-', [StringComparison]::OrdinalIgnoreCase))) {
            throw 'Refusing to remove a release fixture outside the temporary test boundary.'
        }
        Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
    }
}
exit 0
