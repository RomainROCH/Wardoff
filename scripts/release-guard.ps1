# Pure release policy. Network reads and binary execution belong to the driver.
Set-StrictMode -Version Latest

function Test-ReleasePositiveInteger {
    param($Value)
    return (($Value -is [int] -or $Value -is [long] -or $Value -is [uint32] -or $Value -is [uint64]) -and $Value -gt 0)
}

function Get-ReleaseVersion {
    param([string] $RepoRoot, [string] $TagName)
    if ($TagName -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') {
        throw 'Release tag must be a stable vX.Y.Z without leading zeroes.'
    }
    $version = $TagName.Substring(1)
    foreach ($part in $version.Split('.')) {
        if ([decimal]$part -gt 65535) { throw 'Release version exceeds the Windows version range.' }
    }
    $toml = Get-Content -LiteralPath (Join-Path $RepoRoot 'Cargo.toml') -Raw
    $package = [regex]::Matches($toml, '(?ms)^\[package\]\s*\r?\n(.*?)(?=^\[|\z)')
    if ($package.Count -ne 1 -or $package[0].Groups[1].Value -notmatch '(?m)^name\s*=\s*"wardoff"\s*$') {
        throw 'Cargo.toml must declare the wardoff package.'
    }
    $versions = [regex]::Matches($package[0].Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if ($versions.Count -ne 1 -or $versions[0].Groups[1].Value -cne $version) {
        throw 'Tag and Cargo.toml package version differ.'
    }
    $lock = Get-Content -LiteralPath (Join-Path $RepoRoot 'Cargo.lock') -Raw
    $packages = @([regex]::Matches($lock, '(?ms)^\[\[package\]\]\s*\r?\n(.*?)(?=^\[\[package\]\]|\z)') |
        Where-Object { $_.Groups[1].Value -match '(?m)^name\s*=\s*"wardoff"\s*$' })
    if ($packages.Count -ne 1) { throw 'Cargo.lock must contain exactly one wardoff package.' }
    $locked = [regex]::Matches($packages[0].Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if ($locked.Count -ne 1 -or $locked[0].Groups[1].Value -cne $version) {
        throw 'Tag and Cargo.lock package version differ.'
    }
    [xml]$manifest = Get-Content -LiteralPath (Join-Path $RepoRoot 'wardoff.manifest') -Raw
    $identity = $manifest.SelectNodes("/*[local-name()='assembly']/*[local-name()='assemblyIdentity']")
    $level = $manifest.SelectNodes("//*[local-name()='requestedExecutionLevel']")
    if ($identity.Count -ne 1 -or $identity[0].GetAttribute('name') -cne 'wardoff' -or
        $identity[0].GetAttribute('version') -cne ($version + '.0') -or $level.Count -ne 1 -or
        $level[0].GetAttribute('level') -cne 'asInvoker' -or $level[0].GetAttribute('uiAccess') -cne 'false') {
        throw 'Manifest must match the package version and retain asInvoker/uiAccess=false.'
    }
    return $version
}

function Resolve-ReleaseTagCommit {
    param([string] $TagName, [AllowEmptyCollection()][string[]] $RemoteRefs)
    if ($TagName -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Invalid release tag.' }
    $refName = 'refs/tags/' + $TagName
    $base = @(); $peeled = @()
    foreach ($line in $RemoteRefs) {
        if ($line -notmatch '^([0-9a-fA-F]{40})\s+(\S+)$') { throw 'Malformed remote tag response.' }
        $sha = $Matches[1]; $ref = $Matches[2]
        if ($ref -ceq $refName) { $base += $sha }
        elseif ($ref -ceq ($refName + '^{}')) { $peeled += $sha }
        else { throw 'Unexpected remote tag reference.' }
    }
    if ($base.Count -ne 1 -or $peeled.Count -gt 1) { throw 'Remote tag is absent or ambiguous.' }
    if ($peeled.Count -eq 1) { return $peeled[0].ToLowerInvariant() }
    return $base[0].ToLowerInvariant()
}

function Select-ReleaseCiRun {
    param([AllowEmptyCollection()][object[]] $CiRuns, [string] $Repository, [string] $CandidateCommit)
    $matching = @($CiRuns | Where-Object {
        $_.head_sha -eq $CandidateCommit -and $_.head_branch -ceq 'main' -and $_.event -ceq 'push' -and
        $_.head_repository.full_name -ceq $Repository -and
        $_.path -cin @('.github/workflows/ci.yml', '.github/workflows/ci.yml@refs/heads/main')
    })
    if ($matching.Count -eq 0) { throw 'No CI push/main run exists for the exact candidate commit.' }
    foreach ($run in $matching) {
        if (-not (Test-ReleasePositiveInteger $run.id) -or -not (Test-ReleasePositiveInteger $run.run_attempt)) {
            throw 'CI run identity or attempt is invalid.'
        }
        $updated = [datetimeoffset]::MinValue
        if ($run.updated_at -isnot [string] -or $run.updated_at -notmatch '(Z|\+00:00)$' -or
            -not [datetimeoffset]::TryParse($run.updated_at, [ref]$updated)) { throw 'CI update timestamp is invalid or not UTC.' }
    }
    $selected = $matching | Sort-Object @{Expression = { [datetimeoffset]$_.updated_at }; Descending = $true}, @{Expression = { $_.id }; Descending = $true}, @{Expression = { $_.run_attempt }; Descending = $true} | Select-Object -First 1
    if ($selected.status -cne 'completed' -or $selected.conclusion -cne 'success') {
        throw 'The latest candidate CI run has not completed successfully.'
    }
    return $selected
}

function Assert-ReleaseDraftAvailable {
    param([string] $TagName, [AllowEmptyCollection()][object[]] $Releases)
    if (@($Releases | Where-Object { $_.tag_name -ceq $TagName }).Count -gt 0) {
        throw 'A release or draft already exists for this tag; never overwrite it.'
    }
}

function Assert-ReleaseCandidate {
    param([string] $RepoRoot, [string] $Repository, [string] $TagName,
        [string] $CandidateCommit, [string] $RemoteTagCommit, [bool] $MainContainsCandidate,
        [AllowEmptyCollection()][object[]] $CiRuns, [AllowEmptyCollection()][object[]] $CiJobs,
        [AllowEmptyCollection()][object[]] $ExistingReleases)
    if ($Repository -cne 'RomainROCH/Wardoff') { throw 'Release repository is not authorized.' }
    if ($CandidateCommit -notmatch '^[0-9a-fA-F]{40}$' -or $RemoteTagCommit -notmatch '^[0-9a-fA-F]{40}$' -or
        $CandidateCommit -ne $RemoteTagCommit) { throw 'Remote tag does not identify the tested commit.' }
    if (-not $MainContainsCandidate) { throw 'Candidate commit is not reachable from main.' }
    $version = Get-ReleaseVersion -RepoRoot $RepoRoot -TagName $TagName
    $run = Select-ReleaseCiRun -CiRuns $CiRuns -Repository $Repository -CandidateCommit $CandidateCommit
    if ($CiJobs.Count -eq 0) { throw 'CI jobs are missing.' }
    foreach ($job in $CiJobs) {
        if ($job.run_id -ne $run.id -or $job.head_sha -ne $CandidateCommit -or -not (Test-ReleasePositiveInteger $job.run_id) -or
            $job.status -cne 'completed' -or $job.conclusion -cne 'success') { throw 'A candidate CI job is not successful.' }
    }
    $windows = @($CiJobs | Where-Object { $_.name -ceq 'ci (windows-latest, stable)' })
    if ($windows.Count -ne 1) { throw 'Required Windows CI job is missing or ambiguous.' }
    foreach ($name in @('Full local checks', 'Developer tooling tests', 'Smoke safety tests', 'Release guard tests', 'Smoke tests')) {
        $step = @($windows[0].steps | Where-Object { $_.name -ceq $name })
        if ($step.Count -ne 1 -or $step[0].status -cne 'completed' -or $step[0].conclusion -cne 'success') {
            throw "Required CI step is absent, skipped or failed: $name."
        }
    }
    Assert-ReleaseDraftAvailable -TagName $TagName -Releases $ExistingReleases
    return [pscustomobject]@{ Tag = $TagName; Version = $version; Commit = $CandidateCommit.ToLowerInvariant(); CiRunId = $run.id; CiAttempt = $run.run_attempt }
}

function Assert-ReleaseArtifact {
    param([string] $PackageDirectory, [string] $CandidateCommit, [string] $TagName,
        [string] $ExpectedSha256, $ExpectedBuildRunId)
    if ($CandidateCommit -notmatch '^[0-9a-fA-F]{40}$' -or $ExpectedSha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        $TagName -cnotmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -or
        -not (Test-ReleasePositiveInteger $ExpectedBuildRunId)) { throw 'Artifact expectations are invalid.' }
    $binary = Join-Path $PackageDirectory 'wardoff.exe'
    $info = Get-Content -LiteralPath (Join-Path $PackageDirectory 'build-info.json') -Raw | ConvertFrom-Json
    $hash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    $checksum = @(Get-Content -LiteralPath (Join-Path $PackageDirectory 'SHA256SUMS.txt'))
    if ($checksum.Count -ne 1 -or $checksum[0] -cnotmatch '^([0-9a-fA-F]{64})  wardoff\.exe$' -or
        $Matches[1] -ne $ExpectedSha256) { throw 'Package checksum file differs from the checked binary.' }
    if ($hash -ne $ExpectedSha256 -or $info.sha256 -ne $ExpectedSha256 -or
        $info.source_commit -ne $CandidateCommit -or $info.tag -cne $TagName -or
        $info.version -cne $TagName.Substring(1) -or $info.target -cne 'x86_64-pc-windows-msvc' -or
        $info.unsigned -isnot [bool] -or -not $info.unsigned -or
        -not (Test-ReleasePositiveInteger $info.build_run_id) -or $info.build_run_id -ne $ExpectedBuildRunId -or
        -not (Test-ReleasePositiveInteger $info.ci_run_id)) { throw 'Artifact hash or build metadata differs from the validated candidate.' }
    return $info
}

function Assert-ReleaseTransferInputs {
    param([string] $ArtifactId, [string] $Sha256)
    if ($ArtifactId -cnotmatch '^[1-9][0-9]*$' -or $Sha256 -cnotmatch '^[0-9a-fA-F]{64}$') {
        throw 'Exact artifact ID and binary SHA256 are required before downloading.'
    }
}
