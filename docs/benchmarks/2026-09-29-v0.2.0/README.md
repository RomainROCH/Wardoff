# Wardoff v0.2.0 resource benchmark — 2026-09-29

This directory preserves the report, protocol, retained measurement script, raw
samples, executable identity, system information, and analysis for this campaign.
Use this dated evidence when citing resource usage; do not generalize it to other
versions, hardware, or protection configurations.

## Mean process resource usage

| Metric | Allow | Block |
|---|---:|---:|
| Resident memory / Working Set (MiB) | 12.57 | 13.95 |
| Private committed memory (MiB) | 1.96 | 3.25 |
| CPU (% of total machine capacity) | 0.0024% | 0.0050% |

**Allow:** application running with protection disabled. **Block:** normal
background protection enabled. Both use the visible tray interface. Private
committed memory and Working Set are different measures and must not be added.

The machine was an Intel Core i7-6700HQ (4 cores / 8 logical processors), 16 GiB
installed RAM, Windows 11 IoT Enterprise LTSC build 26100.9445, Balanced power
plan. Wardoff ran elevated. The UpdateOrchestrator Reboot task was absent, so its
protection layer was inactive. The other four layers reported active in Block.
No real shutdown attempt was part of the workload.

## Preserved evidence

- [Detailed report and protocol (French)](RAPPORT.md): means, medians, nearest-rank
  P95, maxima, individual repetitions, sampling method, and limitations.
- [Retained collection script](Measure-Wardoff.ps1): one-second target intervals,
  monotonic elapsed time, process and system CPU, private memory, Working Set,
  threads, handles and process I/O. Its bytes match the original local archive.
- [Raw data](results/): nine CSV files containing 2,700 samples; runtime state
  snapshots, phase log, completion status and Update-layer evidence.
- [System and measurement metadata](results/metadata.json) and
  [source provenance](provenance.json).
- [Binary identity](binary.json): version, release URL, source tag and SHA-256
  verified against the downloaded executable and GitHub release asset digest.
- [Analysis script](analyze.py), [statistics](summary.csv), and
  [validation results](validation.json).
- [File integrity manifest](SHA256SUMS.txt): SHA-256 of every other file in this
  directory. The scoped `.gitattributes` preserves their bytes across platforms.

The executable is not duplicated in Git. Retrieve it from the URL in
`binary.json` and require the recorded SHA-256 before using it. The preliminary
pilot, redundant console output, and full GitHub API response are not part of
this evidence set. All final-run CSV and state files are preserved unchanged.

No collector-script checksum was recorded at run startup. The archived copy
matches the initial local archive, and the later manifest protects its bytes;
this is not cryptographic attestation that those bytes executed. A future
collector should record its own digest at startup. This historical collector
and the original run metadata have not been retroactively modified to claim it.

## Audit without rerunning the experiment

From this directory, use Python 3 (standard library only):

```powershell
python verify.py
```

The verifier checks manifest coverage and every file digest, then reruns the
analysis in a temporary directory and compares the generated statistics,
validation and detailed report. It does not launch Wardoff or alter the stored
evidence. Checks require nine phases of 300 samples, each phase within 1.25
seconds of its target duration, no sample interval above 1.25 seconds, consistent
elapsed-time totals, and consistent runtime states. These tolerances are for
this recorded campaign, not a universal acceptance rule for future experiments.

To conduct a **new** experiment, copy `Measure-Wardoff.ps1` and the verified
executable into a separate new directory, then run the script in an elevated
PowerShell session. Do not overwrite this dated evidence set. The collector is
an exact historical script; its version/hash and source metadata are pinned to
this campaign and must be reviewed for a different build.

## Interpretation and retention

There were three 300-second repetitions per scenario, with 30 seconds of
excluded warmup and rotated scenario order: 45 minutes measured in total.
CPU is normalized across all eight logical processors and uses actual elapsed
time. A zero median/P95 reflects timer quantization and does not mean zero CPU
work. Maxima represent approximately one-second intervals, not instantaneous
peaks. The desktop was not isolated; baseline system CPU averaged 9.31%.

The figures describe Wardoff's process, not the complete causal cost of Windows
services or kernel activity. Startup, actual shutdown blocking, sustained
foreground workloads, battery use, and long-term leaks were not measured.

Keep the whole directory together. Corrections to the report or analysis must
be explicit Git changes followed by manifest regeneration; never silently edit
raw measurements. A future campaign belongs in a new dated directory. Local
files, a ZIP, and a local commit alone are not an off-machine backup: completion
requires the evidence commit on an authorized remote, with its SHA read back.
