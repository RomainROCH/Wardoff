# Security audit

- Date: 2026-05-02
- Auditor: automated internal review
- Scope: full source review of src/
- Wardoff version: c419316cbb9f0c7f0449a7dcbbc3f7b67a64e03a

## Summary

Wardoff is materially safer than it was at the previous audit point, but it is still **not yet ready** for an elevated early-adopter release in a hostile local-attacker model. The original pre-release blockers around writable-path autostart, named-pipe pre-start squatting, spoofed `WM_ENDSESSION` teardown, and environment-variable task-principal spoofing are resolved in the current `dev` state, and the old global cross-session mutex problem is narrowed substantially by session scoping. The remaining release-significant concerns are the residual elevated logging file-targeting risk, the still-squattable same-session mutex, the status pipe's default DACL, and the control pipe's unbounded single-client request read path.

## Findings

### [INFORMATIONAL] Fixed: highest-runlevel autostart no longer accepts user-writable install paths

- **Location:** `src/autostart.rs:57-78`, `src/autostart.rs:403-523`
- **Attack surface:** Task Scheduler manipulation; privilege escalation risks
- **Description:** Wardoff now canonicalizes its executable path before autostart registration, requires elevation, and refuses to register the highest-runlevel task when the current interactive user's non-elevated token can modify the executable path or its parent directory.
- **Exploitability:** The original attack of enabling autostart from a user-writable location and then replacing the binary before next logon is addressed in the reviewed code.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **resolved**
- **Recommended fix:** Keep the current refusal behavior and regression coverage. The separate question of scheduled-task ACL materialization still needs live Windows validation.

### [INFORMATIONAL] Fixed: scheduled-task principal is no longer derived from spoofable environment variables

- **Location:** `src/autostart.rs:72`, `src/autostart.rs:568-679`
- **Attack surface:** Task Scheduler manipulation
- **Description:** Wardoff now resolves the scheduled-task principal from the process token and SID-to-account lookup path instead of trusting `USERNAME` and `USERDOMAIN`.
- **Exploitability:** The earlier principal-spoofing path through attacker-controlled environment variables is closed in the reviewed code.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **resolved**
- **Recommended fix:** Keep the token-based path and continue testing it with elevated autostart scenarios.

### [INFORMATIONAL] Fixed: named-pipe pre-start squatting and wrong-server startup are now blocked

- **Location:** `src/session_scope.rs:7-8`, `src/session_scope.rs:57-62`, `src/ipc.rs:120-160`, `src/ipc.rs:788-846`, `src/ipc.rs:895-897`
- **Attack surface:** Named pipe security
- **Description:** Wardoff now uses session-scoped pipe names, claims first pipe instances during startup, waits for both initial IPC servers to own their first instances before reporting success, and fails closed when that ownership cannot be established within the bounded startup retry window.
- **Exploitability:** The prior attack of pre-creating the fixed pipe names before startup to hijack or silently break the real IPC server is addressed for the original blocker shape.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **resolved**
- **Recommended fix:** Keep the fail-closed startup behavior and the current regression tests around startup-window exhaustion.

### [INFORMATIONAL] Fixed: spoofed `WM_ENDSESSION` no longer tears down protection outside a real shutdown

- **Location:** `src/blocker/shutdown.rs:196-245`
- **Attack surface:** Tray/window message handling
- **Description:** Layer 1 forced cleanup now runs only when both `wParam != 0` and `GetSystemMetrics(SM_SHUTTINGDOWN) != 0`, instead of trusting any successful `WM_ENDSESSION` delivery.
- **Exploitability:** The earlier same-integrity spoof path that could force cleanup outside a genuine system shutdown or sign-out is closed in the reviewed code.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **resolved**
- **Recommended fix:** Keep the current gating and continue validating it on target Windows versions during manual shutdown/sign-out testing.

### [INFORMATIONAL] Improved: the singleton is no longer global across all Windows sessions

- **Location:** `src/session_scope.rs:6`, `src/session_scope.rs:53-62`, `src/instance.rs:29-39`
- **Attack surface:** Singleton/mutex; named pipe security
- **Description:** Wardoff now scopes the mutex and both named pipes to the current Windows session, which removes the previous cross-session collision problem and makes session-local ownership consistent across singleton and IPC endpoints.
- **Exploitability:** The original cross-session squatting/DoS problem is narrowed substantially, but same-session squatting remains because the mutex still uses default security and no authenticated primary handshake.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **partially fixed**
- **Recommended fix:** See the still-open same-session mutex finding below.

### [HIGH] Residual elevated logging file-targeting risk remains after the reparse-point fix

- **Location:** `src/logger/mod.rs:395-463`
- **Attack surface:** Log file handling
- **Description:** The new logger hardening correctly rejects reparse points in the managed log tree and uses reparse-aware opens, but it still accepts any ordinary plain file at the managed log path. Because `%LOCALAPPDATA%\Wardoff\logs` remains user-writable, a same-user attacker can still plausibly pre-place a hardlinked or otherwise attacker-chosen plain file and let an elevated Wardoff runtime append to it.
- **Exploitability:** A same-user local attacker can still target elevated file writes if they can arrange a non-reparse plain-file target inside the managed log path before Wardoff opens it.
- **Risk for early-adopter release:** **blocker**
- **Fix status:** **partially fixed**
- **Recommended fix:** Reject unexpected link counts or otherwise verify file identity through opened handles, or move elevated logs to a location that is not user-writable.

### [MEDIUM] Same-session mutex squatting and denial of service still remain possible

- **Location:** `src/instance.rs:29-67`
- **Attack surface:** Singleton/mutex
- **Description:** The mutex name is now session-scoped, but Wardoff still creates it with default security and still decides primary-versus-secondary ownership without an authenticated primary-instance handshake. That means another process in the same session can still create or hold the mutex first and interfere with startup.
- **Exploitability:** A same-session local process can still mount startup denial-of-service or false-secondary behavior.
- **Risk for early-adopter release:** **acceptable only after the logging blocker is fixed**
- **Fix status:** **partially fixed**
- **Recommended fix:** Apply an explicit mutex security descriptor and/or pair the mutex with a stronger authenticated primary-instance check.

### [MEDIUM] Status pipe still inherits the process default DACL

- **Location:** `src/ipc.rs:841-846`
- **Attack surface:** Named pipe security
- **Description:** The control pipe uses explicit security attributes, but the status pipe still passes `None` and therefore relies on the creating token's default DACL.
- **Exploitability:** Exposure depends on runtime token defaults and host policy, so this is not as strong as the original blocker set, but it leaves the status endpoint less explicitly hardened than the control endpoint.
- **Risk for early-adopter release:** **fix before broader distribution**
- **Fix status:** **still open**
- **Recommended fix:** Apply an explicit reviewed security descriptor to the status pipe as well.

### [MEDIUM] Control pipe request handling still allows single-client read denial of service

- **Location:** `src/ipc.rs:430-537`, `src/ipc.rs:899-914`
- **Attack surface:** Named pipe security
- **Description:** The control server still accepts one client at a time and blocks on `BufRead::read_line` with no explicit size cap or read timeout. A client that connects and never finishes a line can still stall command processing.
- **Exploitability:** Any local client that can connect to the control pipe can deny service to other control requests for the duration of that stalled read.
- **Risk for early-adopter release:** **fix before broader distribution**
- **Fix status:** **still open**
- **Recommended fix:** Add request-size caps plus read timeouts or a more defensive multi-client/overlapped handling model.

### [MEDIUM] Scheduled-task ACL hardening is still not explicit

- **Location:** `src/autostart.rs:199-208`, `src/autostart.rs:256-267`
- **Attack surface:** Task Scheduler manipulation
- **Description:** The task-registration path still relies on default Task Scheduler ACL materialization and does not set an explicit task SDDL in code.
- **Exploitability:** Code review alone does not prove a direct exploit here, but the effective rights still need live Windows validation before Wardoff should be considered hardened.
- **Risk for early-adopter release:** **validation debt**
- **Fix status:** **still open**
- **Recommended fix:** Validate the resulting task ACLs on supported Windows versions and set explicit SDDL if the defaults are broader than intended.

### [MEDIUM] Layer 2 ETW naming and buffering hardening remain unchanged

- **Location:** `src/blocker/local.rs:81-83`, `src/blocker/local.rs:553-559`, `src/blocker/local.rs:587-603`, `src/blocker/local.rs:674-679`
- **Attack surface:** ETW session
- **Description:** Layer 2 still uses a predictable ETW session name and still runs with relatively small buffering and limited lost-event resilience.
- **Exploitability:** This is still a robustness and defense-in-depth issue rather than the highest-priority release blocker, but it remains part of the current local hardening gap.
- **Risk for early-adopter release:** **acceptable only as later hardening debt**
- **Fix status:** **still open**
- **Recommended fix:** Randomize the ETW session name with retry-on-collision behavior and harden buffering plus lost-event handling.

### [INFORMATIONAL] Same-user medium-integrity control of an elevated runtime remains intentional and documented

- **Location:** `src/ipc.rs:1075-1092`, `docs/ARCHITECTURE.md:213-220`
- **Attack surface:** Named pipe security; privilege elevation boundary
- **Description:** Wardoff still intentionally allows the same interactive Windows user to send control commands from a non-elevated shell to an elevated runtime, but that trust boundary is now explicitly documented instead of being an undocumented surprise.
- **Exploitability:** This remains a product trust decision, not an accidental gap in the reviewed code.
- **Risk for early-adopter release:** **informational**
- **Fix status:** **resolved as documented behavior**
- **Recommended fix:** Keep this trust model explicit in user-facing and contributor-facing documentation so it is not mistaken for a hardened service boundary.

## Dependency review

Direct dependencies are unchanged from the previous audit, and the blocker fixes did not add new direct crates.

- `chrono 0.4.44` — mainstream time/date crate; broadly used and actively maintained.
- `clap 4.6.0` — mainstream CLI parser; broadly used and actively maintained.
- `env_logger 0.11.9` — common logging bootstrap crate; low supply-chain concern by itself.
- `log 0.4.29` — de facto Rust logging facade; high trust and very common.
- `serde 1.0.228` — core serialization framework; mainstream and actively maintained.
- `serde_json 1.0.149` — mainstream JSON serializer/deserializer; broadly trusted.
- `tray-icon 0.21.3` — still the main supply-chain concern because it pulls broad cross-platform GUI baggage, including `muda`, GTK/appindicator, and `objc2` families via `Cargo.lock`.
- `windows 0.62.2` — official Microsoft bindings and high-trust, but it exposes a large Win32/COM FFI surface that increases review complexity.
- `winres 0.1.12` — build-time only, but still part of the trusted build chain.

Additional notes:

- Manual review only: `cargo audit` was not available locally during the audit sessions, so no RustSec scan was run.
- No git, path, or alternate-registry dependencies were visible in the reviewed lockfile.
- No obvious hardcoded secrets, tokens, or credentials were found in the reviewed source tree.
- `SECURITY.md` still contains a placeholder private-report email, which is an operational weakness rather than a code vulnerability.

## Conclusion

Wardoff is **closer, but still not yet ready** for an elevated early-adopter release. The most important earlier blockers are now resolved or materially narrowed: autostart path trust is enforced, task-principal spoofing is closed, named-pipe startup squatting is blocked, and spoofed `WM_ENDSESSION` teardown is no longer accepted outside a real shutdown. However, the current `dev` branch still carries one release-significant local issue in the logging path and several remaining medium-severity hardening gaps: same-session mutex squatting, the status pipe's default DACL, the control pipe's unbounded `read_line` denial-of-service path, and unresolved scheduled-task ACL validation. If the logging issue is fixed next and the remaining medium-severity items are addressed or consciously accepted, Wardoff would become a much more credible source-first technical preview for advanced early adopters.
