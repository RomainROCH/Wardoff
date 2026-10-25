# Reevaluation log

## Out-of-scope and unforeseen findings

### 2026-10-25 - Owned sessions preserve the manual choice

- Status: accepted and backed up; linked IDs: RM-reference-004, RM-reference-005 and
  RM-reference-007.
- Evidence: [session acceptance](sessions-plan.md#acceptance-2026-10-25) binds
  216 local tests, standard/admin ownership and regression checks, actual tray,
  Explorer and Quit, and two passive real-S3 expiry cycles to source `85125f2`.
  The expired temporary full-pair contributor did not replace or cancel manual
  system-only Block at wake. Allow invalidated old messages without killing work.
- Decision: keep the selected order. After verifying the phase backup, deliver
  the native accessible controls in 005 using the accepted ledger; keep simple
  indefinite Block/Allow central. Preserve hardware gaps for 007/009 without
  buying hardware, cloud capacity or signing.
- Limits: cold fixture startup, long paths and observation windows produced
  retained failed/inconclusive attempts. Use explicit readiness and source-bound
  raw receipts; never count a late observation as proof of expiry during sleep.
  No broad Windows latency or protection guarantee follows from these VM cases.

### 2026-10-24 - M1 acceptance preserves the manual contract

- Status: accepted locally; linked IDs: RM-reference-001, RM-reference-002,
  RM-reference-003, RM-reference-004 and RM-reference-007.
- Evidence: [power-intent acceptance](power-intent-plan.md#acceptance-2026-10-24)
  binds Full checks and desktop/real-S3/tray observations to one source/artifact.
  Doctor and explicit system-only Block add useful visibility and choice while
  default indefinite Block retains its system/display pair.
- Decision: keep the selected sequence. After the verified phase backup, design
  owned sessions in 004 around independent ownership and expiration; none may
  cancel or expire manual Block. Keep real-device and Modern Standby gaps for
  007/009, using existing environments and no purchase.
- Limits: a first uninstrumented Doctor failure remains retained and unlocalized;
  subsequent instrumented checks passed after lab recovery. Native startup races
  and input reliability also required fixture corrections. Reuse ownership checks
  and explicit readiness before interaction, and distinguish application budgets
  from synchronous Windows latency. No universal protection or performance
  leadership claim follows from M1.

### 2026-10-23 - Desktop validation resolves the startup failure

- Status: baseline accepted locally; linked IDs: RM-reference-001,
  RM-reference-002.
- Impact: stderr identified `PowerRequestDisplayRequired` rejected with
  `ERROR_NOT_SUPPORTED` in service session 0. It still failed after a desktop
  was opened; the same executable passed in desktop session 1 with both full
  and limited UAC tokens. This is evidence for this VM's execution contexts,
  not a universal Windows service restriction.
- Decision: run desktop acceptance in the actual signed-in desktop and retain
  startup stderr/JSONL. Preserve transactional Block failure and its rollback.
  Accept slice 001 from the [baseline evidence](baseline-2026-10-23.md), keeping
  physical-power and disruptive manual gaps explicit. No power-plan or lock-policy
  change, product fallback, paid resource or publication was used.

### 2026-10-23 - A writable smoke fixture hid an autostart test defect

- Status: corrected and verified; linked ID: RM-reference-001.
- Impact: smoke can pass its autostart case by safely refusing a writable
  executable location. A separate protected install created the task, exposing
  a false ownership failure: its canonical `\\?\` paths were compared literally
  with ordinary paths.
- Decision: commit `d04df86` corrects only the test comparison, preserving exact
  task XML and extended-path component semantics. RED/GREEN fixtures, developer
  checks and protected-fixture desktop smoke pass. Structured events confirm
  actual task creation and removal; a separate tray run confirms task launch,
  Block/Allow controls, autostart-off, Explorer recovery and clean Quit.

### 2026-10-23 - A successful host build hid a portable-startup failure

- Status: loader and desktop-runtime follow-up verified; linked ID:
  RM-reference-001.
- Impact: isolated smoke of the exact MSVC executable failed even on read-only
  CLI actions with `STATUS_DLL_NOT_FOUND`. Its PE imports include
  `VCRUNTIME140.dll`, which is an undeclared prerequisite for the portable
  contract. The development host's installed runtime can mask this failure.
- Decision: commit `5249c17` selects static CRT linkage. Full and explicit IPC
  checks pass; PE imports omit the external CRT and clean-Windows CLI startup
  works without a redistributable. The initial service-session smoke failed and
  a separate desktop attempt could not select a safe session. The later signed-in
  desktop follow-up above resolves that gate. See the
  [baseline record](baseline-2026-10-23.md).

### 2026-10-23 - Preserve Wardoff's philosophy while starting the roadmap

- Status: resolved direction; linked IDs: RM-reference-001, RM-reference-002,
  RM-reference-004, RM-reference-005, RM-reference-007.
- Impact: the maintainer approved proceeding only while keeping Wardoff's
  original philosophy. A task-first framing could reduce it to a keep-awake tool.
- Decision: user control over supported shutdown/restart/idle-power paths stays
  central. Indefinite manual Block/Allow needs no task or timer. Expiration and
  battery rules for optional automatic sessions cannot cancel manual Block.
  Start slice 001; do not broaden implementation to all future slices at once.

### 2026-10-23 - Remote main already integrates earlier work

- Status: integrated and accepted locally; linked ID:
  RM-reference-001.
- Impact: fresh fetch found main `8c45977` containing `39aadd2` and `6039955`,
  plus subsequent IPC/status, smoke, release and test improvements. Old `dev`
  and the still-open IPC draft no longer describe the actual integration state.
- Decision: consolidate current main with the local roadmap, preserve other
  worktrees and validate this combination instead of reimplementing existing fixes.

### 2026-10-23 - Basic automation is already competitive parity

- Status: open; linked IDs: RM-reference-004, RM-reference-006, RM-reference-009,
  RM-reference-010.
- Impact: PowerToys documents process attachment and timers; Don't Sleep documents
  activity rules; Insomnia documents AI/app-aware activation. Merely adding those
  features cannot establish a unique advantage.
- Decision: prioritize explanation, owned overlapping sessions and workflow
  verification. Reassess after comparative/usability evidence; do not claim
  exclusive capability. Sources: [comparison](../../COMPARISON.md).

### 2026-10-23 - Physical validation is still incomplete

- Status: open; linked IDs: RM-reference-001, RM-reference-007, RM-reference-009.
- Impact: historical VM results do not establish locked-laptop, Hibernate or
  Modern Standby behavior on real hardware.
- Decision: use existing environments, record missing rows and limit claims.
  No hardware/cloud purchase is a dependency.

### 2026-10-23 - Commercial milestones are not product prerequisites

- Status: open; linked ID: RM-reference-008.
- Impact: the previous commercial evolution table linked packaging to signed
  sales and named IFEO in a future 1.0 scenario.
- Decision: the maintainer requested a zero-budget roadmap. Free distribution
  is independent of those sales, and IFEO is not a 1.0 gate. The
  [business owner](../../BUSINESS_MODEL.md#zero-budget-product-work--2026-10-23)
  records this execution decision; paid offers are not implemented here.

### 2026-10-23 - An inaccessible primary competitor source is not evidence of abandonment

- Status: open; linked ID: RM-reference-009.
- Impact: the ShutdownBlocker publisher page could not be fetched in this review.
  Its current support and capabilities cannot be established from old mirrors.
- Decision: retain it as unverified context; retry its primary source before any
  live comparison. No separate executable action item is created.
