# Reevaluation log

## Out-of-scope and unforeseen findings

### 2026-10-23 - A successful host build hid a portable-startup failure

- Status: loader correction verified; desktop-runtime gate open; linked ID:
  RM-reference-001.
- Impact: isolated smoke of the exact MSVC executable failed even on read-only
  CLI actions with `STATUS_DLL_NOT_FOUND`. Its PE imports include
  `VCRUNTIME140.dll`, which is an undeclared prerequisite for the portable
  contract. The development host's installed runtime can mask this failure.
- Decision: commit `5249c17` selects static CRT linkage. Full and explicit IPC
  checks pass; PE imports omit the external CRT and clean-Windows CLI startup
  works without a redistributable. The complete runtime smoke still fails in
  service session 0, and the separate desktop attempt could not select a safe
  existing session. Keep slice 001 active and diagnose the runtime in a proper
  isolated desktop environment. See the [baseline record](baseline-2026-10-23.md).

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

- Status: integrated locally; runtime acceptance in progress; linked ID:
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
