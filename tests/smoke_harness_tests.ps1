[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'smoke_helpers.ps1')
$source=Join-Path $PSScriptRoot 'smoke_test.ps1'
$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($source,[ref]$null,[ref]$errors)
if($errors){throw ($errors.Message -join '; ')}
function Load-Function([string]$Name){
    $defs=@($ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $Name},$true))
    if($defs.Count -ne 1){throw "Expected one $Name"}
    . ([scriptblock]::Create(($defs[0].Extent.Text -replace ('^function '+[regex]::Escape($Name)),('function script:'+ $Name))))
}
foreach($name in @('Assert-SmokePreflight','Assert-SmokeOwnedRuntime','Invoke-ExternalCommand','Stop-RepoWardoffProcesses','Add-TestResult')){Load-Function $name}
function Assert-True([bool]$v,[string]$message){if(-not $v){throw $message}}
function Assert-Throws([scriptblock]$action,[string]$text){
    $caught=$null;try{&$action}catch{$caught=$_.Exception.Message}
    Assert-True ($null -ne $caught -and $caught -match [regex]::Escape($text)) "Expected failure: $text; observed $caught"
}
function Fake-Process([int]$Identity,[bool]$Exited=$false){
    $p=[pscustomobject]@{Id=$Identity;HasExited=$Exited;Stops=0;Disposed=$false}
    $p|Add-Member ScriptMethod Kill {param($tree) $this.Stops++;$this.HasExited=$true}
    $p|Add-Member ScriptMethod WaitForExit {param($ms) return $this.HasExited}
    $p|Add-Member ScriptMethod Dispose {$this.Disposed=$true}
    return $p
}
# These are explicit fakes. Native entrypoint body is never invoked.
$binaryPath=[IO.Path]::GetFullPath((Join-Path $env:TEMP 'wardoff.exe'))
$script:OwnedProcesses=[Collections.Generic.List[object]]::new()
$script:BackgroundProcess=$null
$script:Lease=[Diagnostics.Stopwatch]::StartNew()
$script:RuntimeEnvironment=@{APPDATA='fixture-app';LOCALAPPDATA='fixture-local'}
$fixture=@{Processes=@();ProcessFailure=$false;Task=$false;NamespaceConflict=$false;NamespaceCalls=0;Commands=0;Arguments=@();Captures=0}
$script:ResourceState=@{Adapters=@{ReadTask={param($path) [pscustomobject]@{Exists=$fixture.Task}}};AutostartIdentityChecks=[Collections.Generic.List[object]]::new()}
function Get-WardoffProcessesByName {if($fixture.ProcessFailure){throw 'fixture enumeration failed'};return $fixture.Processes}
function Get-SmokeCurrentSessionId {return 7}
function Assert-SmokeNamespaceUnused {$fixture.NamespaceCalls++;if($fixture.NamespaceConflict){throw 'fixture namespace conflict'}}
function Invoke-SmokeChildCommand {
    param($FilePath,$Arguments,$WorkingDirectory,$Environment,$TimeoutMilliseconds,$OnStarted,[switch]$RetainProcess)
    $fixture.Commands++;$fixture.Arguments=$Arguments
    Assert-True ($Environment.APPDATA -ceq 'fixture-app') 'Child environment lost its isolated AppData.'
    if($RetainProcess){&$OnStarted (Fake-Process 456 $true)}
    [pscustomobject]@{ExitCode=0;StandardOutput='fixture';StandardError='';Output='fixture';LaunchedProcessId=456}
}
function Capture-SmokeOwnedEtw {param($State,$Process) $fixture.Captures++}
$cases=0
$canary=$false;try{Assert-Throws {} 'canary'}catch{$canary=$true};Assert-True $canary 'Assertion canary was accepted.'
Assert-SmokePreflight -Initial;Assert-True ($fixture.NamespaceCalls -eq 1) 'Initial preflight did not check namespace.';$cases++
$fixture.ProcessFailure=$true;Assert-Throws {Assert-SmokePreflight -Initial} 'fixture enumeration failed';$fixture.ProcessFailure=$false;$cases++
$fixture.Processes=@([pscustomobject]@{ProcessId=200;SessionId=7;ExecutablePath='C:\foreign\wardoff.exe'})
Assert-Throws {Assert-SmokePreflight -Initial} 'conflicting Wardoff runtime';$fixture.Processes=@();$cases++
$fixture.Task=$true;Assert-Throws {Assert-SmokePreflight -Initial} 'autostart task already exists';$fixture.Task=$false;$cases++
$fixture.NamespaceConflict=$true;Assert-Throws {Assert-SmokePreflight -Initial} 'fixture namespace conflict';$fixture.NamespaceConflict=$false;$cases++
Assert-Throws {Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--block')} 'owned runtime is not alive'
Assert-True ($fixture.Commands -eq 0) 'An absent runtime reached the command backend.';$cases++
$runtime=Fake-Process 100;$script:BackgroundProcess=$runtime;$script:OwnedProcesses.Add($runtime)
$fixture.Processes=@([pscustomobject]@{ProcessId=100;SessionId=7;ExecutablePath=$binaryPath})
$r=Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--block')
Assert-True ($fixture.Commands -eq 1 -and $fixture.Arguments -contains '--wardoff-detached-runtime') 'Fallback runtime was not kept inside the owned CLI process.'
Assert-True ($script:OwnedProcesses.Count -eq 2) 'CLI process handle was not retained.';$cases++
$runtime.HasExited=$true
Assert-Throws {Invoke-ExternalCommand -FilePath $binaryPath -Arguments @('--allow')} 'owned runtime is not alive'
Assert-True ($fixture.Commands -eq 1) 'A dead runtime reached the mutation backend.';$cases++
$runtime.HasExited=$false;$fixture.Processes=@();$foreign=Fake-Process 300
Stop-RepoWardoffProcesses
Assert-True ($runtime.Stops -eq 1 -and $foreign.Stops -eq 0 -and $fixture.Captures -eq 2) 'Cleanup did not remain within owned handles.';$cases++
# Execute the actual outer finally with fakes, including two cleanup errors.
$main=@($ast.EndBlock.Statements|Where-Object {$_ -is [Management.Automation.Language.TryStatementAst]})[-1]
Assert-True ($null -ne $main.Finally) 'Missing main cleanup finally.'
$cleanup=@{Stopped=0;Restored=0;Receipt=''}
function Stop-RepoWardoffProcesses {$cleanup.Stopped++;throw 'fixture stop failure'}
function Restore-SmokeResources {param($State,$RuntimeExited,$OwnedProcesses) $cleanup.Restored++;throw 'fixture restore failure'}
function Set-Content {param([Parameter(ValueFromPipeline)]$Value,$LiteralPath,$Encoding) process{$cleanup.Receipt=[string]$Value}}
$script:Results=[Collections.Generic.List[object]]::new();$script:BinaryLock=$null
$script:RuntimeOutput=$null;$script:RuntimeError=$null
$runRoot=$env:TEMP;$ExpectedBinarySha256=('a'*64)
$RequireAutostartLifecycle=$false;$script:AutostartLifecyclePassed=$false
$script:ResourceState.AutostartIdentityChecks.Add([pscustomobject]@{Matches=$true;Fields=@(
    [pscustomobject]@{Field='ActionPath';Expected='C:\fixture\wardoff.exe';Actual='\\?\C:\fixture\wardoff.exe';NormalizedExpected='C:\fixture\wardoff.exe';NormalizedActual='C:\fixture\wardoff.exe';Matches=$true;Error=$null}
)})
$text=$main.Finally.Extent.Text
& ([scriptblock]::Create($text.Substring(1,$text.Length-2)))
Assert-True ($cleanup.Stopped -eq 1 -and $cleanup.Restored -eq 1) 'Stop failure prevented resource restoration.'
Assert-True (@($script:Results|Where-Object Status -eq 'FAIL').Count -eq 2) 'Cleanup errors were swallowed.'
Assert-True ($runtime.Disposed -and $cleanup.Receipt -match 'Resource restoration') 'Finalization skipped handle disposal or result evidence.';$cases++
$identityEvidence=($cleanup.Receipt|ConvertFrom-Json).autostart_identity_checks
Assert-True ($identityEvidence.Count -eq 1 -and $identityEvidence[0].Fields[0].Expected -ceq 'C:\fixture\wardoff.exe' -and
    $identityEvidence[0].Fields[0].Actual -ceq '\\?\C:\fixture\wardoff.exe' -and $identityEvidence[0].Fields[0].NormalizedActual -ceq 'C:\fixture\wardoff.exe') 'Outer cleanup lost raw or normalized identity evidence.'
$RequireAutostartLifecycle=$true;$script:Results=[Collections.Generic.List[object]]::new()
& ([scriptblock]::Create($text.Substring(1,$text.Length-2)))
Assert-True (@($script:Results|Where-Object Description -eq 'Required positive autostart lifecycle').Count -eq 1) 'Missing positive coverage was accepted.';$cases++
[ordered]@{status='PASS_MOCK_HARNESS_TESTS';cases=$cases;assertion_canary=$true;real_harness_invoked=$false;native_process_task_etw_calls=$false} | ConvertTo-Json
