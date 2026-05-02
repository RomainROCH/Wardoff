# Security audit

- Date: 2026-05-02
- Auditor: automated internal review
- Scope: full source review of src/
- Wardoff version: f41d6fa02505875daf436a44ae6c7e6b0c58ce19

## Summary

Wardoff is not yet safe to distribute even as a source-first MVP for technical early adopters because the current dev build still has several local-attacker blockers: writable-path elevated autostart, log-path reparse-point abuse, named-pipe squatting/hijacking, global mutex squatting, and spoofable forced-shutdown teardown; after those are fixed and the remaining trust-model questions are documented or hardened, the codebase looks close enough for a narrowly scoped early-adopter release.

## Findings

### [CRITICAL] Highest-runlevel autostart can execute from a user-writable installation path

- **Location:** `src/autostart.rs:49-52`, `src/autostart.rs:155-167`, `src/autostart.rs:285-303`, `src/autostart.rs:310-323`
- **Attack surface:** Task Scheduler manipulation; privilege escalation risks
- **Description:** When autostart is enabled, Wardoff registers a highest-runlevel scheduled task that points at the current executable path and uses the executable parent as working directory. If Wardoff is launched from a user-writable location, an attacker who can replace the binary or plant loadable files in that directory before next logon can turn the scheduled task into elevated code execution.
- **Exploitability:** Any local attacker with write access to the chosen install directory can wait for the next interactive logon and obtain elevated execution in the victim context.
- **Risk for early-adopter release:** **blocker**
- **Recommended fix:** Treat this as a release blocker. Only allow highest-runlevel autostart from a trusted admin-writable install location, verify ownership/ACLs before registration, and consider refusing registration from user-profile paths entirely.

### [CRITICAL] Structured log writer follows attacker-controlled reparse points

- **Location:** `src/logger/mod.rs:80-87`, `src/logger/mod.rs:237-250`, `src/logger/mod.rs:266-324`
- **Attack surface:** Log file handling
- **Description:** The structured logger writes under `%LOCALAPPDATA%\Wardoff\logs` and rotates logs with ordinary create/append/rename/remove filesystem calls. The code does not defend against junctions, symlinks, or other reparse-point tricks, so an elevated Wardoff process can be redirected into creating, appending, renaming, or deleting files outside the intended log directory.
- **Exploitability:** A local attacker who can prepare the log path before or during execution can abuse the elevated writer for file-system side effects outside the log tree.
- **Risk for early-adopter release:** **blocker**
- **Recommended fix:** Treat this as a release blocker. Resolve and validate every path component without following unsafe reparse points, create the log directory with trusted ACL checks, and use handles opened with reparse-point-aware protections instead of path-based rename/remove flows.

### [HIGH] Named-pipe squatting or hijacking is possible before Wardoff starts

- **Location:** `src/ipc.rs:112-145`, `src/ipc.rs:331-361`, `src/ipc.rs:536-601`, `src/ipc.rs:646-707`
- **Attack surface:** Named pipe security
- **Description:** Wardoff uses fixed pipe names (`\\.\pipe\WardoffControl` and `\\.\pipe\WardoffStatus`) but does not first claim them through a stronger first-instance protocol or authenticate the peer it connects to. A local process can pre-create either pipe before Wardoff starts, causing the real server to fail startup or causing clients to connect to the wrong server.
- **Exploitability:** Any local process running before Wardoff can squat the pipe names and cause denial of service, spoofed replies, or wrong-server connections.
- **Risk for early-adopter release:** **blocker**
- **Recommended fix:** Treat this as a release blocker. Add a robust first-instance claim for pipe ownership, authenticate the server/client identity, and fail closed when the peer is not the expected Wardoff instance.

### [HIGH] Same-user medium-integrity clients can control an elevated runtime over the control pipe

- **Location:** `src/ipc.rs:45-47`, `src/ipc.rs:743-785`, `src/main.rs:281-314`, `src/main.rs:436-469`, `src/main.rs:550-560`, `src/main.rs:596-655`
- **Attack surface:** Named pipe security; privilege elevation boundary
- **Description:** The control pipe ACL intentionally allows the current interactive user to read and write to the control channel, and the elevated runtime accepts mode changes and autostart changes from that pipe. That means a same-user medium-integrity process can drive security-sensitive behavior in the elevated instance.
- **Exploitability:** Any process running as the same Windows user, but without elevation, can send control requests to the elevated runtime.
- **Risk for early-adopter release:** **blocker unless explicitly intended and documented**
- **Recommended fix:** If this trust model is intentional, document it plainly in architecture and security docs so users understand that non-elevated same-user processes are trusted operators of the elevated service. Otherwise, harden the pipe to require elevation or stronger peer authentication before release.

### [HIGH] Hidden session window accepts spoofable `WM_ENDSESSION` and tears down protection

- **Location:** `src/blocker/shutdown.rs:226-240`, `src/blocker/shutdown.rs:311-327`, `src/blocker/shutdown.rs:342-354`, `src/blocker/mod.rs:177-183`, `src/main.rs:1062-1070`, `src/main.rs:1166-1174`
- **Attack surface:** Tray/window message handling
- **Description:** The Layer 1 session window treats `WM_ENDSESSION` as authoritative and immediately begins forced-shutdown cleanup, which switches the coordinator into Allow mode and tears down protections. The code does not authenticate the sender or distinguish genuine session shutdown from a spoofed same-integrity message.
- **Exploitability:** UIPI may block some low-to-high integrity sends, but same-integrity or already-elevated peer processes remain relevant and can potentially force protective teardown locally.
- **Risk for early-adopter release:** **blocker**
- **Recommended fix:** Treat this as a release blocker. Narrow which windows receive session-ending signals, add stronger validation around shutdown state, and test whether alternate notification paths or sender checks can prevent spoofed cleanup.

### [HIGH] Scheduled-task principal identity is derived from spoofable environment variables

- **Location:** `src/autostart.rs:51-52`, `src/autostart.rs:57-64`, `src/autostart.rs:229-243`, `src/autostart.rs:325-333`
- **Attack surface:** Task Scheduler manipulation
- **Description:** The scheduled task principal is built from `USERDOMAIN` and `USERNAME` environment variables instead of the process token. Environment-variable spoofing is a poor source of identity for a highest-runlevel task definition and can cause the wrong principal to be recorded or trusted.
- **Exploitability:** A local attacker who can influence the environment of the registering process may steer task registration toward an unintended user string, with security-sensitive consequences depending on Task Scheduler behavior.
- **Risk for early-adopter release:** **fix before release**
- **Recommended fix:** Derive the account identity from the current process token or SID-to-name resolution, not from environment variables.

### [MEDIUM] Status pipe access control is left to the process default DACL

- **Location:** `src/ipc.rs:570-581`
- **Attack surface:** Named pipe security
- **Description:** Unlike the control pipe, the status pipe is created with `None` security attributes, so access falls back to the process default DACL. That may be acceptable, but it is not explicit hardening for a well-known IPC endpoint.
- **Exploitability:** Local access depends on the runtime token's default DACL and deployment context, which makes security posture less predictable.
- **Risk for early-adopter release:** **fix before release**
- **Recommended fix:** Apply an explicit, reviewed security descriptor to the status pipe instead of inheriting the default DACL.

### [MEDIUM] Single-instance control path can be blocked by unbounded `read_line` behavior

- **Location:** `src/ipc.rs:346-435`, `src/ipc.rs:541-545`, `src/ipc.rs:604-619`
- **Attack surface:** Named pipe security
- **Description:** The control server accepts one client at a time and then blocks on `BufRead::read_line` without a size cap or timeout. A client that connects and never completes a line, or that sends a very large line, can stall acceptance of other clients and consume memory.
- **Exploitability:** Any local client that can connect to the pipe can mount a denial-of-service attack against the single control server instance.
- **Risk for early-adopter release:** **fix before release or immediately after**
- **Recommended fix:** Add line-size caps, read timeouts or overlapped/threaded accept handling, and fail fast on oversized requests.

### [MEDIUM] Global mutex is squattable and cross-session denial-of-service prone

- **Location:** `src/instance.rs:27-45`, `src/instance.rs:69-78`, `src/instance.rs:94-103`
- **Attack surface:** Singleton/mutex
- **Description:** Wardoff uses a predictable global mutex name, `Global\WardoffInstance`, without a hardened security descriptor or ownership verification. Another process can create or hold that object first and make Wardoff believe a primary instance already exists.
- **Exploitability:** Any local process able to create the global mutex first can interfere with startup across sessions and integrity levels.
- **Risk for early-adopter release:** **blocker**
- **Recommended fix:** Treat this as a release blocker. Harden the mutex security descriptor, verify expected ownership where possible, and consider pairing the mutex with a stronger authenticated primary-instance handshake.

### [MEDIUM] ETW session naming is predictable and collisions disable Layer 2

- **Location:** `src/blocker/local.rs:81-83`, `src/blocker/local.rs:553-559`, `src/blocker/local.rs:587-603`
- **Attack surface:** ETW session
- **Description:** The Layer 2 ETW session name is derived from the process ID and a timestamp, which is predictable enough for collision attempts in a local attacker model. If the name already exists, the layer stays inactive.
- **Exploitability:** A local attacker able to race or pre-create colliding session names can disable this defensive layer.
- **Risk for early-adopter release:** **fix after release or in the next security release**
- **Recommended fix:** Use stronger random session names and retry on collision instead of treating `ERROR_ALREADY_EXISTS` as a terminal disablement.

### [MEDIUM] ETW buffering and lost-event handling can blind local monitoring under load

- **Location:** `src/blocker/local.rs:337-364`, `src/blocker/local.rs:669-679`
- **Attack surface:** ETW session
- **Description:** Layer 2 uses a small real-time ETW buffer configuration and does not appear to surface lost-event telemetry as a hard failure. Under event pressure, that can reduce visibility into local shutdown launches.
- **Exploitability:** This is primarily a robustness issue; an attacker who can create enough event pressure may increase the chance that shutdown-related process starts are missed.
- **Risk for early-adopter release:** **acceptable only as defense-in-depth debt**
- **Recommended fix:** Increase buffer resilience, monitor lost-event counters explicitly, and decide whether sustained loss should degrade status or disable the feature loudly.

### [MEDIUM] Highest-runlevel scheduled-task ACL hardening is not explicit and needs live validation

- **Location:** `src/autostart.rs:168-180`, `src/autostart.rs:246-260`
- **Attack surface:** Task Scheduler manipulation
- **Description:** The code registers a highest-runlevel interactive logon task but does not set an explicit task security descriptor or otherwise document hardened ACL expectations. Code review alone does not prove a direct exploit, but it leaves an important pre-release security question unresolved.
- **Exploitability:** Actual risk depends on how Task Scheduler materializes ACLs for this registration flow on supported Windows versions.
- **Risk for early-adopter release:** **fix or validate before release**
- **Recommended fix:** Live-test the registered task's ACLs and edit rights on representative Windows systems, document the resulting trust boundary, and set an explicit SDDL if defaults are broader than intended.

### [INFORMATIONAL] Existing safeguards reduce some obvious attack paths but do not remove the blockers above

- **Location:** `src/ipc.rs:420-434`, `src/ipc.rs:541`, `src/ipc.rs:575`, `src/ipc.rs:604-619`, `src/windows_util.rs:54-95`, `src/main.rs:173-179`, `src/main.rs:436-469`, `src/main.rs:588-593`, `src/main.rs:677-706`, `src/tray/mod.rs:498-507`
- **Attack surface:** General code review; named pipe security; privilege elevation boundary; tray/window message handling
- **Description:** The audit did find some meaningful positives: both named pipes set `PIPE_REJECT_REMOTE_CLIENTS`; malformed control-pipe JSON is rejected cleanly; elevation does not show a direct `ShellExecuteW` argument-injection issue; elevation and autostart use the executable parent as working directory rather than inheriting an uncontrolled current directory; and wake messages alone do not directly trigger tray or IPC actions.
- **Exploitability:** These controls reduce accidental exposure, but they do not close the local-attacker issues listed above.
- **Risk for early-adopter release:** **informational**
- **Recommended fix:** Preserve these protections while addressing the higher-severity local privilege and IPC weaknesses.

## Dependency review

Manual dependency review only: `cargo audit` was not available locally, so no RustSec scan was run.

- `chrono 0.4.44` — mainstream time/date crate; broadly used and actively maintained.
- `clap 4.6.0` — mainstream CLI parser; broadly used and actively maintained.
- `env_logger 0.11.9` — common logging bootstrap crate; low supply-chain concern by itself.
- `log 0.4.29` — de facto Rust logging facade; high trust and very common.
- `serde 1.0.228` — core serialization framework; mainstream and actively maintained.
- `serde_json 1.0.149` — mainstream JSON serializer/deserializer; broadly trusted.
- `tray-icon 0.21.3` — the main supply-chain concern here because it pulls broad cross-platform GUI baggage, including `muda`, GTK/appindicator, and `objc2` families in `Cargo.lock` (`Cargo.lock:625-660`, `Cargo.lock:1397-1415`).
- `windows 0.62.2` — official Microsoft bindings and high-trust, but it exposes a very large Win32/COM FFI surface that increases review complexity.
- `winres 0.1.12` — build-time only, but still part of the trusted build chain.

Additional notes:

- No git, path, or alternate-registry dependencies were visible; reviewed lockfile entries resolve from crates.io (`Cargo.lock:214-230`, `Cargo.lock:1397-1400`).
- No obvious hardcoded secrets, tokens, or credentials were found in the reviewed source tree.
- `SECURITY.md:14-23` still contains a placeholder private-report email, which is operationally weak even though it is not itself a code vulnerability.

## Conclusion

Wardoff in its current dev state should **not** be distributed yet, even to technical early adopters, because several issues are strong pre-release blockers in a local-attacker model: the writable-path highest-runlevel autostart task, the log-directory reparse-point issue, named-pipe squatting/hijacking, the squattable global mutex, and spoofable `WM_ENDSESSION`-driven teardown. The next tier of work should also resolve or explicitly document the trust boundary for same-user medium-integrity control of an elevated runtime, and live-validate scheduled-task ACL behavior on supported Windows versions. If those blockers are fixed and the remaining trust assumptions are documented clearly, Wardoff could become reasonable to ship as a source-first MVP to technically capable early adopters who understand that it is an elevated local system utility rather than a hardened multi-user service.
