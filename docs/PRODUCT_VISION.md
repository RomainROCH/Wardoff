# Product vision: user control over shutdown, restart and idle power

Decision date: 2026-10-23. Planning baseline: `39aadd2`.
Status: direction approved with the maintainer's clarification that Wardoff's
philosophy stays central; future features below are not shipped behavior.
[PLAN.md](../PLAN.md) owns scope and priority;
the [reference roadmap](planning/reference/index.md) contains the execution order.

## The ambition

Wardoff gives people control over supported shutdown, restart and idle-power
paths, through understandable protection they can leave active until they choose
Allow, or optionally bound to a duration or task. Make it the Windows utility
people trust to keep that choice visible and under their control.
Someone should be able to answer three questions immediately:

1. Which protections are active, until I release them or until what chosen end?
2. What has Wardoff actually requested or activated, and what can Windows override?
3. How do I stop this protection and return to normal behavior?

The aim is leadership in that control, demonstrated by public evidence.
Neither a larger feature count nor a claim to block every shutdown establishes
that leadership. The application must be useful to an ordinary person without
requiring them to understand Windows layers, IPC or Rust.

## Philosophy that future features must preserve

- Simple Block/Allow remains a first-class use, including indefinite manual
  protection with no task, account, timer or profile required.
- Shutdown and restart control remain central alongside idle-power protection.
  Success is not measured only by keeping a computer awake.
- Timers, work sessions and automation are optional extensions. They cannot
  silently expire or cancel an existing permanent manual Block choice.
- The user can always inspect and explicitly release protection. Automation
  cannot silently reactivate it after a global Allow.
- Keep the core free, local, open and honest about Windows limits. Stronger
  marketing claims do not justify invasive interception or hidden side effects.

## Who to serve first

| Audience | Job to be done | Success |
| --- | --- | --- |
| People on self-administered Windows desktops | Choose Block or Allow for supported shutdown/restart and idle-power paths | A clear, persistent choice until they explicitly change it, with actual limits visible |
| Creators and technical desktop users | Leave a render, copy or calculation running | Optionally bound protection to a duration, allow the display to sleep if desired, then release that request |
| Developers and self-administering operators | Protect a command or two overlapping jobs | Preserve command output and exit status; finishing one job does not unprotect the other |
| Presenters and occasional users | Keep the system and display available temporarily | Start and stop a clearly labeled session from the tray without a terminal |

Managed fleets, unattended servers and adversarial multi-user deployments are
later audiences. Today's security and physical validation do not justify those
deployment promises. This audience order is a strategic hypothesis, to be tested
with volunteers rather than paid acquisition.

## What could make Wardoff better

**Explainable protection.** Show the work, remaining duration, requested effects,
activation errors and applicable Windows limits. Separate configured intent,
successful API activation and behavior verified on a particular machine. A
successful Power Request is not proof that every policy will honor it.

**Optional ownership and duration.** A timer and a command can each own an
additional session. Their requests combine with indefinite manual Block; release
removes only the corresponding owner. A visible global Allow cancels everything
without killing the work. Bounded sessions reduce forgotten protection when
chosen; they do not replace permanent manual protection.

**One experience for people and automation.** Tray controls, a small accessible
panel and a stable machine-readable interface expose the same state. Advanced
users can protect a command; everyone can inspect and stop it. Preserve the
existing CLI meanings while adding the new workflow.

**Trust supported by evidence.** Keep the native Windows implementation, local
operation and inspectable source. Publish exact tested builds, cleanup results,
compatibility gaps and reproducible measurements. Native Rust alone does not
prove lower CPU, memory or energy use.

These are differentiation hypotheses, not claims of uniqueness. The
[competitive research](COMPARISON.md) shows that timers, process attachment,
activity rules and AI integrations already exist elsewhere. Wardoff should win
on their reliable combination with shutdown/restart control, diagnostics and a
simple manual choice that remains useful without any automation.

## Choose focus over three tempting alternatives

| Direction | Benefit | Decision |
| --- | --- | --- |
| Add every timer, sensor and automation rule | Broad checklist coverage | Defer opaque CPU/network heuristics until concrete demand; they are already common and can guess task completion incorrectly |
| Chase stronger forced-shutdown interception | A striking marketing promise | Exclude IFEO, drivers and forced-shutdown guarantees from this roadmap; the recovery and security costs undermine trust |
| Make user-controlled protection understandable and verifiable | Keeps the core choice simple while supporting optional work-bound use | Recommended direction: diagnostics, independent sessions, usable controls, command workflow, then measured distribution |

Respect explicit Sleep/Hibernate, lock-screen security, OS battery policy and
the user's decision to stop protection. Microsoft documents termination of
Power Requests on explicit sleep, and additional limits on Modern Standby on
battery. Wardoff cannot make those limits disappear through messaging or repeated
reacquisition. [Microsoft PowerSetRequest documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).

## What "a reference" will mean in practice

The following are proposed acceptance targets, not measured results:

- Five consenting volunteers, including at least two non-developers, try the same
  five tasks: use indefinite Block/Allow, add timed protection with optional
  display intent, understand a shutdown/power limitation, stop one optional
  session without cancelling manual Block, stop all. At least four complete each task
  without coaching; typical completion is under two minutes per task.
- Automated lifecycle tests cover overlapping owners, expiration, invalid input,
  disconnect, command failure and global cancellation without leaked requests.
- Every advertised Windows/power scenario has a result tied to a build and
  environment. Missing physical hardware remains untested.
- Matched comparisons publish raw resource and workflow results. Claims such as
  "lighter" or "easier" are used only for the tested versions and scenarios.
- A person can obtain the free build, understand unsigned-download limitations,
  remove it cleanly, and provide a redacted local diagnostic without an account.

Small volunteer samples guide decisions; they do not establish universal
superiority. Downloads, stars and synthetic test counts are supporting signals,
not substitutes for those outcomes.

## Zero-budget operating boundary

No certificate purchase, paid runner, cloud backend, paid API, advertising,
hardware purchase, subscription or trial requiring payment. Use existing licensed
Windows environments, existing hardware and local fixtures. Missing resources
reduce the tested scope rather than create a spending requirement.

The core remains free and MIT-licensed, without telemetry, accounts or a feature
paywall. Distribution starts with the existing repository infrastructure; package
manager submission follows validation. Existing commercial ideas remain separate
and cannot block free product adoption. The
[business model](BUSINESS_MODEL.md#zero-budget-product-work--2026-10-23) records
that execution constraint.

## Next action

The maintainer authorized starting the roadmap with its philosophy preserved.
Work on `RM-reference-001`: establish one reviewed, tested integration head and
an honest capability/evidence baseline. The next slice is the read-only
explanation in `RM-reference-002`; this is not authorization for broad parallel
implementation of the remaining roadmap.
