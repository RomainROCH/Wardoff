<#
.SYNOPSIS
Runs the repository's lightweight documentation and Cargo validation checks.
.DESCRIPTION
The Docs mode performs only the local Markdown scan; it does not validate anchors or parse full Markdown.
#>
[CmdletBinding()]
param(
    [ValidateSet('Fast', 'Full', 'Docs')]
    [string]$Mode = 'Fast'
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$originalLocation = (Get-Location).Path

function Invoke-CheckStep {
    param(
        [string]$Label,
        [scriptblock]$Action
    )
    Write-Output ("==> {0}" -f $Label)
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw ("{0} failed with exit code {1}." -f $Label, $LASTEXITCODE)
    }
    Write-Output ("PASS: {0}" -f $Label)
}

try {
    Set-Location -LiteralPath $repoRoot
    $docsScript = Join-Path $PSScriptRoot 'check-docs.ps1'
    Invoke-CheckStep 'Markdown documentation' { & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $docsScript -RepoRoot $repoRoot }
    if ($Mode -eq 'Docs') {
        Write-Output 'Documentation checks completed.'
        exit 0
    }

    Invoke-CheckStep 'cargo fmt --all -- --check' { & cargo fmt --all -- --check }
    Invoke-CheckStep 'cargo check --locked' { & cargo check --locked }
    Invoke-CheckStep 'cargo test --locked' { & cargo test --locked }
    if ($Mode -eq 'Full') {
        Invoke-CheckStep 'cargo clippy --locked --all-targets --all-features -- -D warnings' { & cargo clippy --locked --all-targets --all-features -- -D warnings }
        Invoke-CheckStep 'cargo build --release --locked' { & cargo build --release --locked }
    }
    Write-Output ("{0} checks completed." -f $Mode)
    exit 0
} catch {
    Write-Error $_.Exception.Message
    exit 1
} finally {
    Set-Location -LiteralPath $originalLocation
}
