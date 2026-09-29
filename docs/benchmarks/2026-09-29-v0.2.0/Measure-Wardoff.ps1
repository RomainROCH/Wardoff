param([int]$Seconds=300,[int]$Warmup=30,[int]$Repeats=3,[string]$OutputName='results')
$ErrorActionPreference='Stop'
[Threading.Thread]::CurrentThread.CurrentCulture=[Globalization.CultureInfo]::InvariantCulture
$out=Join-Path $PSScriptRoot $OutputName
New-Item -ItemType Directory -Force $out | Out-Null
$exe=Join-Path $PSScriptRoot 'wardoff.exe'
$expected='96D1700C492B1689E6CB3A18847481EB9A203E30DB4D4BA273D4B458ED6A84BC'
if((Get-FileHash $exe).Hash -ne $expected){throw 'Executable hash mismatch'}
if(Get-Process wardoff -ErrorAction SilentlyContinue){throw 'Existing Wardoff process: preserve it and stop.'}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class BenchNative {
 [DllImport("kernel32.dll",SetLastError=true)] public static extern bool GetSystemTimes(out ulong idle,out ulong kernel,out ulong user);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool PostThreadMessage(uint thread,uint msg,UIntPtr w,IntPtr l);
 [DllImport("kernel32.dll",SetLastError=true)] public static extern bool GetProcessIoCounters(IntPtr h,out IO v);
 [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);
 [StructLayout(LayoutKind.Sequential)] public struct IO {public ulong ReadOps,WriteOps,OtherOps,ReadBytes,WriteBytes,OtherBytes;}
 public static ulong[] Times(){ulong i,k,u;if(!GetSystemTimes(out i,out k,out u))throw new Exception("GetSystemTimes failed");return new ulong[]{i,k,u};}
 public static IO Counters(IntPtr h){IO v;if(!GetProcessIoCounters(h,out v))throw new Exception("GetProcessIoCounters failed");return v;}
}
'@
function Save-Json($value,$path){$value | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 $path}
function Status { $text= & $exe --status; if($LASTEXITCODE -ne 0){throw 'Status failed'}; return ($text | ConvertFrom-Json) }
function Stop-Owned($p){
 if($null -eq $p){return}; $p.Refresh(); if($p.HasExited){throw 'Runtime exited unexpectedly'}
 $main=$p.Threads | Sort-Object StartTime | Select-Object -First 1
 if(-not [BenchNative]::PostThreadMessage([uint32]$main.Id,0x12,[UIntPtr]::Zero,[IntPtr]::Zero)){throw 'WM_QUIT failed'}
 if(-not $p.WaitForExit(20000)){throw 'Graceful exit timed out; process preserved'}
 if(Get-Process wardoff -ErrorAction SilentlyContinue){throw 'Unexpected surviving Wardoff process'}
}
$owned=$null
try {
 $admin=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
 $cpu=Get-CimInstance Win32_Processor
 $os=Get-CimInstance Win32_OperatingSystem
 $logical=($cpu | Measure-Object NumberOfLogicalProcessors -Sum).Sum
 $meta=[ordered]@{startedUtc=[DateTime]::UtcNow.ToString('o');seconds=$Seconds;warmupSeconds=$Warmup;repeats=$Repeats;admin=$admin;cpu=@($cpu | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors);os=($os | Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize);logicalProcessors=$logical;powerPlan=(powercfg /getactivescheme | Out-String).Trim();binarySha256=$expected;version='0.2.0';releaseTag='v0.2.0';releaseCommit='9f6c0375c851d0a5b933da0bd69bafc75b215cf5';mainCommit='aa40e8982273fa66b86b806ec1424a547affdbd3';cpuDefinition='100 * process CPU seconds / actual elapsed seconds / logical processors';ramUnit='MiB = 1048576 bytes';ioDefinition='Process I/O includes file/device/network operations, not physical disk traffic';p95Definition='nearest rank';collectorPid=$PID}
 Save-Json $meta (Join-Path $out 'metadata.json')
 # Keep system awake equally in every condition; no change to display or power plan.
 [void][BenchNative]::SetThreadExecutionState([uint32]2147483649)
 $orders=@(@('baseline','allow','block'),@('allow','block','baseline'),@('block','baseline','allow'))
 $collector=Get-Process -Id $PID
 for($r=1;$r -le $Repeats;$r++){
  foreach($scenario in $orders[($r-1)%3]){
   $label="run-$r-$scenario"
   "$(Get-Date -Format o) START $label" | Add-Content (Join-Path $out 'progress.log')
   if($scenario -ne 'baseline'){
    Start-Process -FilePath $exe -ArgumentList '--allow' -WindowStyle Hidden
    Start-Sleep -Seconds 3
    $ps=@(Get-Process wardoff -ErrorAction SilentlyContinue)
    if($ps.Count -ne 1){throw 'Expected exactly one runtime'}
    $owned=$ps[0]
    if($owned.Path -ne $exe){throw 'Unexpected executable path'}
    if($scenario -eq 'block'){ & $exe --block | Out-Null; if($LASTEXITCODE -ne 0){throw 'Block command failed'} }
   }
   Start-Sleep -Seconds $Warmup
   if($owned){$s=Status; if($s.state -ne $scenario){throw 'Wrong runtime state'}; Save-Json $s (Join-Path $out "$label-start.json")}
   elseif(Get-Process wardoff -ErrorAction SilentlyContinue){throw 'Baseline contaminated'}
   $rows=[Collections.Generic.List[object]]::new()
   $watch=[Diagnostics.Stopwatch]::StartNew()
   $lastT=$watch.Elapsed.TotalSeconds; $lastSys=[BenchNative]::Times()
   $collector.Refresh(); $lastCollector=$collector.TotalProcessorTime.TotalSeconds
   $lastCpu=0.; $lastIo=$null
   if($owned){$owned.Refresh();$lastCpu=$owned.TotalProcessorTime.TotalSeconds;$lastIo=[BenchNative]::Counters($owned.Handle)}
   for($i=1;$i -le $Seconds;$i++){
    $wait=1000*($i-$watch.Elapsed.TotalSeconds); if($wait -gt 0){Start-Sleep -Milliseconds ([int]$wait)}
    $now=$watch.Elapsed.TotalSeconds;$dt=$now-$lastT;$sys=[BenchNative]::Times()
    $total=($sys[1]-$lastSys[1])+($sys[2]-$lastSys[2]);$idle=$sys[0]-$lastSys[0]
    $collector.Refresh();$cc=$collector.TotalProcessorTime.TotalSeconds
    $row=[ordered]@{run=$r;scenario=$scenario;sample=$i;utc=[DateTime]::UtcNow.ToString('o');elapsed_s=$now;interval_s=$dt;system_cpu_pct=100*($total-$idle)/$total;collector_cpu_pct=100*($cc-$lastCollector)/$dt/$logical;pid='';process_cpu_s='';cpu_pct='';private_mib='';working_set_mib='';threads='';handles='';io_read_bytes_s='';io_write_bytes_s=''}
    if($owned){
     $owned.Refresh();if($owned.HasExited){throw 'Runtime died during measurement'}
     $pc=$owned.TotalProcessorTime.TotalSeconds;$io=[BenchNative]::Counters($owned.Handle)
     $row.pid=$owned.Id;$row.process_cpu_s=$pc-$lastCpu;$row.cpu_pct=100*($pc-$lastCpu)/$dt/$logical
     $row.private_mib=$owned.PrivateMemorySize64/1MB;$row.working_set_mib=$owned.WorkingSet64/1MB
     $row.threads=$owned.Threads.Count;$row.handles=$owned.HandleCount
     $row.io_read_bytes_s=($io.ReadBytes-$lastIo.ReadBytes)/$dt;$row.io_write_bytes_s=($io.WriteBytes-$lastIo.WriteBytes)/$dt
     $lastCpu=$pc;$lastIo=$io
    }
    $rows.Add([pscustomobject]$row);$lastT=$now;$lastSys=$sys;$lastCollector=$cc
   }
   $rows | Export-Csv -NoTypeInformation -Encoding UTF8 (Join-Path $out "$label.csv")
   if($owned){$s=Status;Save-Json $s (Join-Path $out "$label-end.json");if($s.state -ne $scenario){throw 'State changed'};Stop-Owned $owned;$owned=$null}
   elseif(Get-Process wardoff -ErrorAction SilentlyContinue){throw 'Baseline contaminated'}
   "$(Get-Date -Format o) COMPLETE $label samples=$($rows.Count)" | Add-Content (Join-Path $out 'progress.log')
  }
 }
 'SUCCESS' | Set-Content (Join-Path $out 'status.txt')
} catch {
 $_ | Out-String | Set-Content (Join-Path $out 'error.txt')
 'FAILED' | Set-Content (Join-Path $out 'status.txt')
 throw
} finally {
 if($owned){Stop-Owned $owned}
 [void][BenchNative]::SetThreadExecutionState([uint32]2147483648)
}
