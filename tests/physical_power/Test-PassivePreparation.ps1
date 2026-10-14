# Focused safety checks. Uses a text fixture and mocked diagnostics; never runs
# Wardoff, powercfg or an actual event-log query.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checks = 0
function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:checks++
}
function Assert-Throws([scriptblock]$Action, [string]$Pattern) {
    $caught = $null
    try { & $Action | Out-Null } catch { $caught = $_ }
    Assert-True ($null -ne $caught -and $caught.Exception.Message -match $Pattern) 'Expected refusal was not observed'
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('wardoff-passive-' + [Guid]::NewGuid().ToString('N'))
$originalLocation = Get-Location
$hadExitCode = Test-Path Variable:global:LASTEXITCODE
$originalExitCode = if ($hadExitCode) { $global:LASTEXITCODE } else { $null }
try {
    $cloneScripts = Join-Path $testRoot 'tests\physical_power'
    New-Item -ItemType Directory -Path $cloneScripts | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $testRoot 'docs') | Out-Null
    $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    foreach ($name in @('PHYSICAL_POWER_VALIDATION.md','MVP_VALIDATION_MATRIX.md')) {
        Copy-Item -LiteralPath (Join-Path $repo ('docs\' + $name)) -Destination (Join-Path $testRoot ('docs\' + $name))
    }
    $stub = Join-Path $testRoot 'fake-powercfg.ps1'
    @'
param([Parameter(ValueFromRemainingArguments=$true)][string[]]$CommandArgs)
$global:LASTEXITCODE = 0
if ($CommandArgs[0] -eq '/requests') {
    $global:LASTEXITCODE = 5
    Write-Error 'Fictitious diagnostic permission failure' -ErrorAction Continue
} else { Write-Output 'Fictitious read-only power setting' }
'@ | Set-Content -LiteralPath $stub -Encoding UTF8
    $allowed = @('Set-StrictMode','Test-Path','Get-FileHash','Join-Path','Split-Path','New-Item','Out-Null','Copy-Item','Out-File','ConvertTo-Json','Set-Content','Write-Output','Get-Content','ConvertFrom-Json','Get-WinEvent','Select-Object','Save-PowerCfg','Save-BaselinePowerCfg')
    $nativeExpression = '(Join-Path $env:SystemRoot ''System32\powercfg.exe'')'
    foreach ($name in @('Prepare-PhysicalPowerValidation.ps1','Collect-PhysicalPowerCheckpoint.ps1')) {
        $path = Join-Path $PSScriptRoot $name
        $tokens = $null; $errors = $null
        $ast = [Management.Automation.Language.Parser]::ParseFile($path,[ref]$tokens,[ref]$errors)
        Assert-True ($errors.Count -eq 0) 'PowerShell parse failure'
        foreach ($command in $ast.FindAll({param($node) $node -is [Management.Automation.Language.CommandAst]},$true)) {
            $commandName = $command.GetCommandName()
            if ($null -eq $commandName) {
                Assert-True ($command.Extent.Text.StartsWith('& ' + $nativeExpression) -or $command.Extent.Text.StartsWith('& $resolve ')) 'Unexpected dynamic command'
            } else { Assert-True ($allowed -contains $commandName) ('Unexpected command: ' + $commandName) }
        }
        $sourceText = Get-Content -LiteralPath $path -Raw
        Assert-True ($sourceText -notmatch '(?i)SetSuspendState|DllImport|Add-Type|Start-Process|Stop-Process|SetThreadExecutionState') 'Power-affecting operation detected'
        Assert-True (($sourceText.Split(@($nativeExpression),[StringSplitOptions]::None).Count - 1) -eq 1) 'Diagnostic mock replacement must be exact'
        $sourceText.Replace($nativeExpression, ("'" + $stub.Replace("'","''") + "'")) | Set-Content -LiteralPath (Join-Path $cloneScripts $name) -Encoding UTF8
    }
    # This function shadows the event reader only in this test's scope.
    function Get-WinEvent {
        [CmdletBinding()]
        param([hashtable]$FilterHashtable,[int]$MaxEvents)
        if ($MaxEvents -ne 100) { throw 'Event query must be bounded' }
        if ($FilterHashtable.ProviderName -eq 'Microsoft-Windows-Power-Troubleshooter') {
            $record = New-Object Management.Automation.ErrorRecord ([InvalidOperationException]::new('Fixture has no matching events')), 'NoMatchingEventsFound', ([Management.Automation.ErrorCategory]::ObjectNotFound), $null
            $PSCmdlet.ThrowTerminatingError($record)
        }
        [pscustomobject]@{Id=42;RecordId=123;TimeCreated=[DateTime]::UtcNow;Message='Fictitious omitted field';MachineName='Fictitious omitted field'}
    }
    Set-Location -LiteralPath $testRoot
    Set-Content -LiteralPath 'fixture.exe' -Value 'Text fixture; never execute' -Encoding UTF8
    $hash = (Get-FileHash -LiteralPath 'fixture.exe' -Algorithm SHA256).Hash
    $prepare = Join-Path $cloneScripts 'Prepare-PhysicalPowerValidation.ps1'
    $collect = Join-Path $cloneScripts 'Collect-PhysicalPowerCheckpoint.ps1'
    $parameters = @{WardoffExecutable='fixture.exe';ExpectedSha256=$hash;OutputDirectory='kit';ApplicationSourceCommit=('a' * 40);AcceptedIntegrationCommit=('b' * 40)}
    $invalid = $parameters.Clone(); $invalid.ExpectedSha256 = '0' * 64
    Assert-Throws { & $prepare @invalid } 'hash mismatch'
    Assert-True (-not (Test-Path -LiteralPath 'kit')) 'Invalid artifact created output'
    $invalid = $parameters.Clone(); $invalid.OutputDirectory = 'Variable:WardoffFixtureKit'
    Assert-Throws { & $prepare @invalid } 'Only FileSystem'
    $invalid = $parameters.Clone(); $invalid.WardoffExecutable = 'Variable:WardoffFixtureBinary'
    Assert-Throws { & $prepare @invalid } 'Only FileSystem'
    Assert-Throws { & $collect -KitDirectory 'Variable:WardoffFixtureKit' -Label 'provider' } 'Only FileSystem'
    Assert-True (-not (Test-Path -LiteralPath 'kit')) 'Non-filesystem path created output'
    & $prepare @parameters | Out-Null
    $kit = Join-Path $testRoot 'kit'
    Assert-True (Test-Path -LiteralPath (Join-Path $kit 'PHYSICAL_POWER_VALIDATION.md') -PathType Leaf) 'Copied validation matrix guide is missing'
    $manifest = Get-Content -LiteralPath (Join-Path $kit 'manifest.json') -Raw | ConvertFrom-Json
    $results = Get-Content -LiteralPath (Join-Path $kit 'results.json') -Raw | ConvertFrom-Json
    Assert-True ($manifest.wardoffSha256 -eq $hash) 'Artifact binding differs'
    Assert-True (-not $manifest.physicalTestsExecuted -and -not $results.physicalTestsExecuted) 'Preparation claimed execution'
    Assert-True ($results.cases.Count -eq 12 -and @($results.cases | Where-Object status -ne 'pending').Count -eq 0) 'Pending cases are incomplete'
    Assert-Throws { & $prepare @parameters } 'already exists'
    Set-Location -LiteralPath $kit
    & $collect -KitDirectory . -Label 'fixture' | Out-Null
    $record = Get-Content -LiteralPath 'checkpoints\fixture\checkpoint.json' -Raw | ConvertFrom-Json
    Assert-True (-not $record.physicalTransitionObserved) 'Checkpoint claimed a transition'
    Assert-True (($record.captures | Where-Object name -eq 'powercfg-requests').exitCode -eq 5) 'Native diagnostic failure was lost'
    Assert-True ($record.eventLog[0].count -eq 1 -and $record.eventLog[1].status -eq 'no-events') ('Provider isolation failed: ' + ($record.eventLog | ConvertTo-Json -Compress))
    $eventsText = Get-Content -LiteralPath 'checkpoints\fixture\power-events.json' -Raw
    Assert-True ($eventsText.TrimStart().StartsWith('[') -and $eventsText -notmatch 'Message|MachineName') 'Event export is not a sanitized array'
    $vectors = @($record.captures | ForEach-Object { $_.arguments -join ' ' })
    Assert-True (($vectors -join '|') -eq '/a|/getactivescheme|/query SCHEME_CURRENT|/requests|/requestsoverride') 'Diagnostics include unexpected arguments'
    Assert-Throws { & $collect -KitDirectory . -Label 'fixture' } 'already exists'
    Assert-Throws { & $collect -KitDirectory . -Label '../escape' } 'pattern'
    Add-Content -LiteralPath 'wardoff.exe' -Value 'fixture corruption'
    Assert-Throws { & $collect -KitDirectory . -Label 'corrupt' } 'hash mismatch'
    Assert-True (-not (Test-Path -LiteralPath 'checkpoints\corrupt')) 'Corrupted artifact created checkpoint'
    Write-Output ('PASS: ' + $checks + ' passive checks; all diagnostics mocked; no physical tests executed')
} finally {
    Set-Location -LiteralPath $originalLocation.Path
    if ($hadExitCode) { $global:LASTEXITCODE = $originalExitCode } else { Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue }
    # Delete only the fresh directory owned by this test, inside the OS temp dir.
    $resolvedTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    if (-not $resolvedTest.StartsWith($resolvedTemp,[StringComparison]::OrdinalIgnoreCase) -or (Split-Path -Leaf $resolvedTest) -notmatch '^wardoff-passive-[0-9a-f]{32}$') { throw 'Unsafe test cleanup path' }
    if (Test-Path -LiteralPath $resolvedTest) { Remove-Item -LiteralPath $resolvedTest -Recurse -Force }
}
