# Idle-power validation record

Validation window: 2026-10-03 through 2026-10-04.

This record covers the functional candidate at commit `4041db0` and the retained Windows release artifact with SHA-256 `8B9545310891A73899115F65474AE86BCC5BF78F09526FF6D4246CED1D1B06A9`. The disposable Windows 11 VM used QEMU 11.1.2 with q35, WHPX, one Westmere-v2 CPU, 3072 MiB, and no network device. The original power plan was preserved.

The record combines the earlier lifecycle checks and the repaired-firmware transition run using the same release artifact. Raw measurements and failed diagnostic controls are retained locally; this table reports their observed results.

| Requested case | Result | Observation |
| --- | --- | --- |
| 1. Block prevents automatic idle sleep | Pass in VM | After resume, Windows remained running with 228.625 seconds of measured keyboard/mouse inactivity, beyond the configured 120-second sleep timeout. The same runtime remained in Block. |
| 2. Block prevents automatic display timeout | Pass in VM | The native console-display state remained on during the same idle observation, beyond the configured 60-second display timeout. |
| 3. Start-menu Sleep proceeds in Block | Pass in VM | Actual S3 was observed at 01:27:37 UTC and wake at 01:27:41 UTC. No synthetic suspend notification or post-wake reset was used. |
| 4. Wake reacquires Block protection | Pass in VM | Before any post-wake UI input, the same runtime remained in Block with exactly one system/display request pair. Native resume notification and successful reacquisition were recorded at 01:27:42.837 UTC; the later idle observation confirmed effective protection. |
| 5. Allow permits automatic display-off and sleep | Pass in VM | Allow completed at 01:38:33.534 UTC. Native display-off occurred at 01:39:35.240 UTC, followed by suspend; actual S3 was observed at 01:39:40 UTC. No UI input or explicit Sleep command occurred during this interval. The existing inactivity interval already exceeded the sleep timeout. After wake, the same runtime remained in Allow with no requests. |
| 6. Requests appear in Block and disappear in Allow | Pass in VM | Both request categories contained the exact Wardoff Block reason, and both became empty in Allow while the runtime stayed alive. |
| 7. Quit directly from Block releases requests | Pass in VM | A visible tray Quit from Block stopped the owned runtime, returned status to inactive, and left no Wardoff request. |
| 8. Abrupt termination from Block releases requests | Pass in VM | The smoke test killed the owned Block-mode runtime directly and confirmed no residual Wardoff request. |

The disposable guest fixture used 60-second display and 120-second sleep timeouts with hybrid sleep disabled. A matched OS-only firmware comparison reproduced lost PCI mappings after S3 with the baseline BIOS. Resetting the cached ECAM access path before PCI restoration recovered PCI BARs, AHCI, guest agents, and the display in the patched control. This was a VM firmware repair, separate from Wardoff's implementation.

Supporting checks passed: 42 Rust tests with zero failures, formatting, the default GNU release build, and ordinary Clippy. The two Clippy diagnostics were reproduced on the pre-feature baseline. Guest smoke testing recorded 18 passes, zero failures, and three skips: two non-elevated cases and the absent Update Orchestrator Reboot task. The new task probe fails on unexpected scheduler errors.

A completed independent source review found no actionable issue in the integration diff. A policy-rejected OpenCode review is excluded from acceptance evidence. Additional MSVC-target release attempts failed on local SDK autodetection; the retained tested Windows release artifact has an exact code/Cargo/tests binding to the candidate above.

Physical host S3 and monitor behavior, Hibernate, and Modern Standby remain **not physically validated**. Hibernate and Modern Standby were not exercised in the VM either. An unattended wake initially left the console display off; ordinary secure-desktop input restored it. This record does not claim a universal Sleep/Hibernate veto or a new presence-aware wake policy.

Cleanup verified the original plan active with identical settings, the temporary fixture deleted, the test runtimes and observers stopped, no remaining Wardoff request, and no remaining temporary lab tasks.
