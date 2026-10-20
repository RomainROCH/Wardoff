# Exact-binary native smoke protocol

The harness correction is validated with simulations and inert child processes.
Its Task Scheduler and ETW adapters have **not** run against Windows resources.
The production binary retained from `06b95136b28b4de75e779632c5b7f501c5b16f7e`
already exists; this change does not rebuild or requalify it.

## Checks that are safe to run locally

Use PowerShell 7. These tests do not launch Wardoff or access Task Scheduler,
ETW, power settings or physical transitions. Process tests launch only inert
PowerShell children owned by the test.

```powershell
pwsh -NoProfile -NonInteractive -File tests/smoke_harness_tests.ps1
pwsh -NoProfile -NonInteractive -File tests/smoke_resources_tests.ps1
pwsh -NoProfile -NonInteractive -File tests/smoke_process_tests.ps1
```

## Proposed target and remaining authorization

Use an offline, disposable Windows 11 x64 guest cloned from the documented
QEMU/q35/WHPX lab, with a rollback snapshot named
`wardoff-smoke-06b9513-before-native`. That snapshot has not been created here.
Use one exclusive interactive test account with a linked UAC token and
PowerShell 7 already installed. Do not use the LaFramboise host session.

Before execution, authorization must name that guest and allow Block/Allow,
SYSTEM/DISPLAY PowerRequests, ETW start/stop, shutdown-abort polling, the possible
Reboot-task Enabled change and restoration, creation/deletion of the test-owned
`\Wardoff` task, and forced termination of owned test processes. The harness
does not authorize these effects. No native test has been approved by this
document, and no publication authorization follows from a native result.

The guest must have no Wardoff runtime, session mutex/pipes or `\Wardoff` task,
no pending shutdown and no concurrent test actor. Keep its existing idle sleep
and display policy disabled throughout this short test; if the prepared snapshot
does not meet that prerequisite, stop rather than silently changing its policy.
Do not initiate shutdown, reboot, Sleep or Hibernate. Host physical behavior is
outside this protocol.

## Procedure after authorization

1. Restore the guest snapshot, keep networking disconnected and close other
   interactive sessions. Record the guest build, UAC/elevation and whether the
   real UpdateOrchestrator Reboot task exists and is enabled. An absent task
   remains a coverage gap; do not create a replacement system task to turn a
   skip into a pass.
2. Copy the retained production executable, without rebuilding, into the
   guest's protected `C:\Program Files\WardoffSmoke\06b9513\wardoff.exe`.
   Keep the directory's normal protected permissions. This preparatory copy
   requires approval inside the disposable guest; the harness performs no
   installation or ACL change. Verify SHA256:
   `0e3a4fe985e5945c78e65fd78006d38698117a8c98f8459d111308008b027c4f`.
   Do not use the old checkout's `target/release/wardoff.exe`: it contains the
   separate CLI namespace variant.
3. Run the harness from the reviewed harness commit in an elevated interactive
   PowerShell 7 session with a linked UAC token:

   ```powershell
   pwsh -NoProfile -NonInteractive -File tests/smoke_test.ps1 `
     -BinaryPath 'C:\Program Files\WardoffSmoke\06b9513\wardoff.exe' `
     -ExpectedBinarySha256 '0e3a4fe985e5945c78e65fd78006d38698117a8c98f8459d111308008b027c4f' `
     -OutputDirectory 'C:\WardoffSmokeEvidence\elevated' `
     -IsolatedTarget -RequireAutostartLifecycle
   ```

   The switch describes a prerequisite; it is not an approval mechanism.
   `-RequireAutostartLifecycle` makes missing positive on/off coverage fail.
   A trusted-location refusal alone is reported separately and cannot satisfy it.
4. Retain the result JSON and isolated application logs. Verify process exit,
   original Reboot-task Enabled/XML/security, absence of the test-created
   autostart task, absence of owned ETW sessions and unchanged binary hash.
   Repeat from the snapshot without elevation and without
   `-RequireAutostartLifecycle` only if non-elevated coverage is also authorized;
   it does not replace the elevated positive lifecycle run.
5. Revert the guest snapshot after collecting private evidence. A timeout,
   task/ETW identity conflict, incomplete cleanup or skip of a required check
   is a failed or incomplete run, not permission to delete a foreign resource.
   If COM/RPC stops responding or the harness itself is killed, rely on guest
   rollback; PowerShell finally cannot guarantee restoration after its own
   termination. Do not continue on the host to obtain a green result.

## Isolation and scope

The harness refuses known process, task and namespace conflicts. It retains
kernel handles for its processes, avoids runtime detachment, isolates AppData,
bounds CLI/logman children, and restores only resources whose identities still
match. Reboot restoration changes only Enabled, never XML or security. ETW
cleanup uses exact new PID/timestamp names within retained process lifetimes.

These guards do not atomically exclude another actor racing production CLI
forwarding or Task Scheduler, and `Kill(true)` is not a Windows job boundary
after a parent has already exited. Stable guest time and an exclusive disposable
session are required. Native results would establish only the observed runtime
and cleanup cases for these exact bytes; they would not establish physical
S3/display behavior, Hibernate, Modern Standby, universal shutdown prevention,
or a complete release qualification. Existing CI/release gates stay blocked.
