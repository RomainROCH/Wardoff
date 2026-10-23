# Product vision: protect the work, explain the limits

Decision date: 2026-10-23. Planning baseline: `39aadd2`.
Status: proposed product direction requested by the maintainer; future features
below are not shipped behavior. [PLAN.md](../PLAN.md) owns scope and priority;
the [reference roadmap](planning/reference/index.md) contains the execution order.

## The ambition

Make Wardoff the Windows utility people trust when an interruption would waste
hours of work: a render, build, transfer, presentation or local computation.
Someone should be able to answer three questions immediately:

1. What work is being protected, and until when?
2. What has Wardoff actually requested or activated, and what can Windows override?
3. How do I stop this protection and return to normal behavior?

The aim is leadership on those workflows, demonstrated by public evidence.
Neither a larger feature count nor a claim to block every shutdown establishes
that leadership. The application must be useful to an ordinary person without
requiring them to understand Windows layers, IPC or Rust.

## Who to serve first

| Audience | Job to be done | Success |
| --- | --- | --- |
| Creators and technical desktop users | Leave a render, copy or calculation running | Protect it for a bounded duration, allow the display to sleep if desired, then release protection |
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

**Protection with ownership and an end.** A manual timer and a command can each
own a session. Their requests combine; release removes only the corresponding
owner. A visible global stop cancels everything without killing the work.
Bounded sessions make forgotten protection less likely.

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
on their reliable combination with shutdown-aware diagnostics and clear control.

## Choose focus over three tempting alternatives

| Direction | Benefit | Decision |
| --- | --- | --- |
| Add every timer, sensor and automation rule | Broad checklist coverage | Defer opaque CPU/network heuristics until concrete demand; they are already common and can guess task completion incorrectly |
| Chase stronger forced-shutdown interception | A striking marketing promise | Exclude IFEO, drivers and forced-shutdown guarantees from this roadmap; the recovery and security costs undermine trust |
| Make work protection understandable and verifiable | Solves a complete task with fewer surprises | Recommended direction: diagnostics, independent sessions, usable controls, command workflow, then measured distribution |

Respect explicit Sleep/Hibernate, lock-screen security, OS battery policy and
the user's decision to stop protection. Microsoft documents termination of
Power Requests on explicit sleep, and additional limits on Modern Standby on
battery. Wardoff cannot make those limits disappear through messaging or repeated
reacquisition. [Microsoft PowerSetRequest documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).

## What "a reference" will mean in practice

The following are proposed acceptance targets, not measured results:

- Five consenting volunteers, including at least two non-developers, try the same
  five tasks: start timed protection, keep the display optional, understand a
  limitation, stop one session, stop all. At least four complete each task
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

Start with `RM-reference-001`: establish one reviewed, tested integration head and
an honest capability/evidence baseline. Then build the read-only explanation in
`RM-reference-002`. No product implementation is activated by this vision alone.
