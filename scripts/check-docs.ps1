<#
.SYNOPSIS
Checks local Markdown destinations with a lightweight syntax scan.
.DESCRIPTION
External URLs and anchors are ignored, anchor existence is not validated, and this is not a full Markdown parser.
#>
[CmdletBinding()]
param(
    [string]$RepoRoot
)

$ErrorActionPreference = 'Stop'
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $scriptDirectory
}
$resolvedRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
$errors = [System.Collections.Generic.List[string]]::new()
$utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)

function Add-DocError {
    param([string]$Message)
    $script:errors.Add($Message)
}

function Get-TargetPath {
    param([string]$Target)
    $value = $Target.Trim()
    if ($value.StartsWith('<')) {
        $endAngle = $value.IndexOf('>')
        if ($endAngle -ge 0) {
            $value = $value.Substring(0, $endAngle + 1)
        }
    }
    if ($value.StartsWith('<') -and $value.EndsWith('>')) {
        $value = $value.Substring(1, $value.Length - 2)
    }
    $titleMatch = [regex]::Match($value, '^(?<destination>.*?)(?:\s+["''][^"'']*["''])$')
    if ($titleMatch.Success) {
        $value = $titleMatch.Groups['destination'].Value
    }
    $value = $value.Trim()
    if ([string]::IsNullOrWhiteSpace($value) -or $value.StartsWith('#') -or $value.StartsWith('//')) {
        return $null
    }
    if ($value -match '^[A-Za-z][A-Za-z0-9+.-]*:') {
        return $null
    }
    $fragmentIndex = $value.IndexOf('#')
    if ($fragmentIndex -ge 0) {
        $value = $value.Substring(0, $fragmentIndex)
    }
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $null
    }
    try {
        return [Uri]::UnescapeDataString($value)
    } catch {
        return $value
    }
}

function Test-Target {
    param(
        [string]$SourcePath,
        [int]$LineNumber,
        [string]$Target
    )
    $relativeTarget = Get-TargetPath $Target
    if ($null -eq $relativeTarget) {
        return
    }
    if ([IO.Path]::IsPathRooted($relativeTarget)) {
        $candidate = $relativeTarget
    } else {
        $candidate = Join-Path (Split-Path -Parent $SourcePath) $relativeTarget
    }
    try {
        $fullCandidate = [IO.Path]::GetFullPath($candidate)
    } catch {
        Add-DocError ("{0}:{1}: malformed local link target '{2}'" -f $SourcePath, $LineNumber, $Target)
        return
    }
    if (-not (Test-Path -LiteralPath $fullCandidate)) {
        Add-DocError ("{0}:{1}: missing local link target '{2}'" -f $SourcePath, $LineNumber, $Target)
    }
}

function Get-InlineTargets {
    param([string]$Line)
    $matches = [regex]::Matches($Line, '!?(?<!\\)\[[^\]]*\]\(')
    foreach ($match in $matches) {
        $start = $match.Index + $match.Length
        $position = $start
        $angle = $false
        $depth = 0
        while ($position -lt $Line.Length) {
            $character = $Line[$position]
            if ($character -eq '<' -and $position -eq $start) { $angle = $true }
            if ($character -eq '(' -and -not $angle) { $depth++ }
            if ($character -eq ')' -and -not $angle) {
                if ($depth -eq 0) { break }
                $depth--
            }
            if ($character -eq '>' -and $angle) { $angle = $false }
            $position++
        }
        if ($position -lt $Line.Length) {
            $content = $Line.Substring($start, $position - $start).Trim()
            if ($content.StartsWith('<')) {
                $end = $content.IndexOf('>')
                if ($end -ge 0) { $content = $content.Substring(0, $end + 1) }
            } else {
                $titleMatch = [regex]::Match($content, '^(?<destination>.*?)(?:\s+["''][^"'']*["''])$')
                if ($titleMatch.Success) { $content = $titleMatch.Groups['destination'].Value }
            }
            [pscustomobject]@{ Target = $content }
        }
    }
}

try {
    # Git emits unquoted paths as UTF-8. Windows PowerShell otherwise decodes
    # them using the console code page, which corrupts non-ASCII filenames.
    $previousConsoleEncoding = [Console]::OutputEncoding
    try {
        [Console]::OutputEncoding = $utf8Strict
        $gitOutput = & git -C $resolvedRoot -c core.quotepath=false ls-files --cached --others --exclude-standard -- '*.md' 2>&1
        $gitExitCode = $LASTEXITCODE
    } finally {
        [Console]::OutputEncoding = $previousConsoleEncoding
    }
    if ($gitExitCode -ne 0) {
        throw "Unable to enumerate Markdown files with Git: $($gitOutput -join ' ')"
    }
    $paths = @($gitOutput | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | ForEach-Object { $_.ToString().Trim() } | Sort-Object -Unique)

    foreach ($relativePath in $paths) {
        $sourcePath = Join-Path $resolvedRoot ($relativePath -replace '/', '\')
        if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            Add-DocError ("{0}: Markdown file listed by Git is missing" -f $sourcePath)
            continue
        }
        try {
            $bytes = [IO.File]::ReadAllBytes($sourcePath)
            $text = $utf8Strict.GetString($bytes)
        } catch {
            Add-DocError ("{0}: invalid UTF-8 Markdown ({1})" -f $sourcePath, $_.Exception.Message)
            continue
        }
        if ($text.IndexOf([char]0) -ge 0) {
            Add-DocError ("{0}: Markdown contains embedded NUL" -f $sourcePath)
            continue
        }
        $lines = $text -split "`r`n|`n|`r"
        $inFence = $false
        $fenceCharacter = $null
        $fenceLength = 0
        for ($index = 0; $index -lt $lines.Count; $index++) {
            $line = $lines[$index]
            if (-not $inFence) {
                $openingFence = [regex]::Match($line, '^\s{0,3}(?<marker>`{3,}|~{3,})')
                if ($openingFence.Success) {
                    $inFence = $true
                    $fenceCharacter = $openingFence.Groups['marker'].Value.Substring(0, 1)
                    $fenceLength = $openingFence.Groups['marker'].Value.Length
                    continue
                }
            } else {
                $closingFence = [regex]::Match($line, '^\s{0,3}(?<marker>`{3,}|~{3,})\s*$')
                if ($closingFence.Success -and
                    $closingFence.Groups['marker'].Value.Substring(0, 1) -eq $fenceCharacter -and
                    $closingFence.Groups['marker'].Value.Length -ge $fenceLength) {
                    $inFence = $false
                    $fenceCharacter = $null
                    $fenceLength = 0
                }
                continue
            }
            $parseLine = [regex]::Replace($line, '`+[^`]*`+', '')
            foreach ($link in Get-InlineTargets $parseLine) {
                Test-Target $sourcePath ($index + 1) $link.Target
            }
            $reference = [regex]::Match($parseLine, '^\s{0,3}\[[^\]]+\]:\s*(?<target><[^>\r\n]*>(?:\s+["''][^"'']*["''])?|.+?)\s*$')
            if ($reference.Success) {
                Test-Target $sourcePath ($index + 1) $reference.Groups['target'].Value
            }
        }
    }
    if ($errors.Count -gt 0) {
        $errors | ForEach-Object { Write-Error $_ }
        exit 1
    }
    Write-Output ("Documentation checks passed ({0} Markdown files; local anchors are not validated)." -f $paths.Count)
    exit 0
} catch {
    Write-Error $_.Exception.Message
    exit 1
}
