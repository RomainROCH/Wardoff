[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$checker = Join-Path $PSScriptRoot '..\scripts\check-docs.ps1'
$entrypoint = Join-Path $PSScriptRoot '..\scripts\check.ps1'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('wardoff-doc-check-' + [Guid]::NewGuid().ToString('N'))

function Invoke-FixtureChecker {
    param([switch]$ExpectFailure)

    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $result = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $checker -RepoRoot $fixtureRoot 2>&1
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    $code = $LASTEXITCODE
    if ($ExpectFailure) {
        if ($code -eq 0) { throw "Expected checker failure, but it succeeded.`n$($result -join [Environment]::NewLine)" }
    } elseif ($code -ne 0) {
        throw "Expected checker success, exit code $code.`n$($result -join [Environment]::NewLine)"
    }
    return ($result -join [Environment]::NewLine)
}

function Assert-Contains {
    param([string]$Text, [string]$Expected)
    if ($Text.IndexOf($Expected, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
        throw "Expected output to contain '$Expected'.`n$Text"
    }
}

function Invoke-Entrypoint {
    param(
        [ValidateSet('Docs', 'Full')]
        [string]$Mode,
        [switch]$ExpectFailure
    )
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $result = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $entrypoint -Mode $Mode 2>&1
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    $code = $LASTEXITCODE
    if ($ExpectFailure -and $code -eq 0) { throw "Expected $Mode entrypoint failure, but it succeeded." }
    if (-not $ExpectFailure -and $code -ne 0) { throw "Expected $Mode entrypoint success, exit code $code.`n$($result -join [Environment]::NewLine)" }
    return ($result -join [Environment]::NewLine)
}

try {
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    Push-Location $fixtureRoot
    git init --quiet
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'README.md') -Encoding utf8 -Value @'
# Fixture

[existing](docs/guide%20file.md)
[reference][guide]
[remote](https://example.invalid/no-file)
[anchor](#section)
`[inline-code](missing-inline.md)`

````text
[fenced-code](missing-fenced.md)
```
[still-fenced](missing-fenced-2.md)
````

[guide]: <docs/guide%20file.md#section> "optional title"
[bare]: docs/guide%20file.md "optional title"
'@
    New-Item -ItemType Directory -Path (Join-Path $fixtureRoot 'docs') | Out-Null
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'docs\guide file.md') -Encoding utf8 -Value '# Guide'
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'docs\nötes.md') -Encoding utf8 -Value '# Notes'
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'ignored.md') -Encoding utf8 -Value '[missing](nope.md)'
    Add-Content -LiteralPath (Join-Path $fixtureRoot '.gitignore') -Encoding utf8 -Value 'ignored.md'
    git add README.md 'docs/guide file.md' 'docs/nötes.md' .gitignore

    $success = Invoke-FixtureChecker
    Assert-Contains $success 'Documentation checks passed'

    $cargoPath = Join-Path $fixtureRoot 'cargo.cmd'
    $cargoTrace = Join-Path $fixtureRoot 'cargo-trace.txt'
    Set-Content -LiteralPath $cargoPath -Encoding ascii -Value '@echo off', 'echo %*>>"%~dp0cargo-trace.txt"', 'exit /b 7'
    $oldPath = $env:PATH
    try {
        $env:PATH = $fixtureRoot + ';' + $oldPath
        $docsEntry = Invoke-Entrypoint -Mode Docs
        Assert-Contains $docsEntry 'Documentation checks completed.'
        if (Test-Path -LiteralPath $cargoTrace) { throw 'Docs mode unexpectedly invoked Cargo.' }
        $fullEntry = Invoke-Entrypoint -Mode Full -ExpectFailure
        Assert-Contains $fullEntry 'cargo fmt --all -- --check failed'
        $trace = Get-Content -LiteralPath $cargoTrace -Raw
        Assert-Contains $trace 'fmt --all -- --check'
        if ($trace -match '(?:^|\s)(?:check|test|clippy|build)(?:\s|$)') { throw "Full mode ran steps after the first failure: $trace" }
    } finally {
        $env:PATH = $oldPath
    }
    Remove-Item -LiteralPath $cargoPath, $cargoTrace -Force -ErrorAction SilentlyContinue

    Set-Content -LiteralPath (Join-Path $fixtureRoot 'untracked.md') -Encoding utf8 -Value '[missing](untracked-target.md)'
    $untrackedFailure = Invoke-FixtureChecker -ExpectFailure
    Assert-Contains $untrackedFailure 'untracked.md:1'

    Remove-Item -LiteralPath (Join-Path $fixtureRoot 'untracked.md')
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'broken.md') -Encoding utf8 -Value '[missing](does-not-exist.md)'
    $brokenFailure = Invoke-FixtureChecker -ExpectFailure
    Assert-Contains $brokenFailure 'broken.md:1'
    Assert-Contains $brokenFailure 'does-not-exist.md'

    Remove-Item -LiteralPath (Join-Path $fixtureRoot 'broken.md')
    Set-Content -LiteralPath (Join-Path $fixtureRoot 'broken-reference.md') -Encoding utf8 -Value @'
[reference][missing]

[missing]: missing-reference.md
'@
    $brokenReferenceFailure = Invoke-FixtureChecker -ExpectFailure
    Assert-Contains $brokenReferenceFailure 'broken-reference.md:3'
    Assert-Contains $brokenReferenceFailure 'missing-reference.md'

    Remove-Item -LiteralPath (Join-Path $fixtureRoot 'broken-reference.md')
    $invalidPath = Join-Path $fixtureRoot 'invalid.md'
    [IO.File]::WriteAllBytes($invalidPath, [byte[]](0x23, 0x20, 0xC3, 0x28))
    $invalidFailure = Invoke-FixtureChecker -ExpectFailure
    Assert-Contains $invalidFailure 'invalid.md'
    Assert-Contains $invalidFailure 'UTF-8'

    Remove-Item -LiteralPath $invalidPath
    $nulPath = Join-Path $fixtureRoot 'nul.md'
    [IO.File]::WriteAllBytes($nulPath, [byte[]](0x23, 0x20, 0x41, 0x00, 0x42))
    $nulFailure = Invoke-FixtureChecker -ExpectFailure
    Assert-Contains $nulFailure 'nul.md'
    Assert-Contains $nulFailure 'NUL'

    Write-Output 'Developer checks tests passed.'
} finally {
    Pop-Location -ErrorAction SilentlyContinue
    $resolvedFixture = $null
    if (Test-Path -LiteralPath $fixtureRoot) {
        $resolvedFixture = (Resolve-Path -LiteralPath $fixtureRoot).Path
    }
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
    $fixtureName = Split-Path -Leaf $resolvedFixture
    if ($resolvedFixture -and $resolvedFixture.StartsWith($tempRoot + '\', [StringComparison]::OrdinalIgnoreCase) -and
        $fixtureName.StartsWith('wardoff-doc-check-', [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
exit 0
