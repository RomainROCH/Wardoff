# Business model

> [!IMPORTANT]
> `docs/BUSINESS_MODEL.md` is the canonical source of truth for Wardoff's monetization and commercialization strategy.
> Repository messaging about sponsorship, signed binaries, paid support or consulting, and other commercialization decisions should follow this document rather than ad hoc guesses.
> Do **not** rewrite this file just to mirror incidental README, roadmap, or implementation drift; update it only to record explicit business decisions and their rationale.

It is a living document. It records what was decided, why, and what comes next. Changes to this document require an explicit decision — do not update it to chase code or market drift without discussion.

## Core principles

These principles are non-negotiable. Every monetization decision must satisfy all four.

1. **The core protection stays free and open.** Wardoff's blocking layers, tray, CLI, logging, and autostart are MIT-licensed and will remain so. No paywall on protective behavior, ever.
2. **Honesty is part of the product value.** Wardoff does not overclaim, does not hide limits, and does not mislead users about what it can or cannot do. The monetization model must preserve that trust.
3. **No dark patterns.** No ads, no telemetry resale, no forced upsells, no artificial crippling of the free version. Users who build from source get the full product.
4. **Revenue comes from convenience, expertise, and trust — not from restricting access to the code.**

## Market reality

The Windows shutdown-blocker category is dominated by freeware (Don't Sleep, ShutdownBlocker, PowerToys Awake, PreventTurnOff). Users in this space expect free, lightweight, low-friction tools. A hard consumer paywall on the core function would be a positioning mistake.

Wardoff's differentiation is not features — it is transparency, honesty about limits, structured logging, CLI scriptability, and conservative multi-layer architecture. That trust positioning is itself part of the monetizable value.

## The model — three layers

### Layer 1 — Donations and sponsorship

**Status:** to activate immediately.

**What:** GitHub Sponsors profile + Buy Me a Coffee link in README.

**Why it fits:** lowest friction, zero cost to set up, compatible with MIT, does not conflict with any other layer. Normalized path for OSS funding.

**Expected revenue:** modest and unpredictable. This is a baseline, not a primary income source.

**Setup checklist:**

- [ ] Create GitHub Sponsors profile
- [ ] Create Buy Me a Coffee page
- [ ] Add sponsor button to repo (`.github/FUNDING.yml`)
- [ ] Add "Support the project" section in README with both links
- [ ] Keep messaging simple: "If Wardoff saves you time or headaches, consider supporting the project."

**Rules:**

- No tier promises that consume maintenance time (no "sponsor gets priority support" — that belongs in Layer 3)
- Sponsor recognition is fine (list in README or Discussions) but not mandatory
- Keep the funding page honest and short

### Layer 2 — Signed binary sales

**Status:** to activate once the Authenticode certificate is acquired.

**What:** sell the official signed release binary on Gumroad or Lemonsqueezy. Price: 5–10 €. The source code stays MIT on GitHub — anyone can build from source for free.

**Why it fits:** this model is proven and ethical. Projects like Ardour, Radium, and Fritzing sell official binaries while keeping the source fully open. It works because the value is not the code — it is the convenience and trust of a signed, verified binary.

**Why it matters for Wardoff specifically:**

- Wardoff runs as an elevated Windows process that hooks into shutdown, Task Scheduler, and power management. An unsigned binary triggers SmartScreen warnings and may be blocked by enterprise IT policies.
- Sysadmins and power users — Wardoff's core audience — understand the value of Authenticode signatures.
- Signing eliminates false-positive antivirus flags, which are common for unsigned Rust binaries that call low-level Windows APIs.

**Investment required:**

- Certum Open Source Developer certificate: ~28 € / year
- Smart card (one-time, if needed): ~80 €
- Shipping: ~30 €
- Total first year: ~140 €. Subsequent years: ~28 € / year.

**Setup checklist:**

- [ ] Purchase Certum Open Source Developer Authenticode certificate
- [ ] Set up signing workflow (signtool.exe with SHA-256 + timestamp)
- [ ] Create Gumroad or Lemonsqueezy product page
- [ ] Build release binary, sign it, upload
- [ ] Add "Download signed binary" link in README alongside "Build from source" instructions
- [ ] Add a short explanation: "The source is free and open. The signed binary is a convenience purchase that supports the project and eliminates SmartScreen warnings."

**Rules:**

- The signed binary and the source build must be functionally identical. No feature gating.
- Price stays accessible (5–10 €). This is a convenience/trust purchase, not a premium tier.
- Every release that ships a signed binary must also ship the corresponding source tag on GitHub.
- If someone cannot afford it, they build from source. No shame, no nag screens.

**Pricing evolution:**

- Start at 5 € to minimize friction during early adoption.
- Reassess at 100 sales. If demand is healthy, 10 € is reasonable.
- Never go above 15 € for a single binary — this is a utility, not a platform.

### Layer 3 — Paid consulting and validation

**Status:** to activate once real users exist and start requesting help.

**What:** paid sessions for environment review, deployment guidance, log interpretation, and compatibility validation. Fixed-fee or hourly.

**Why it fits:** Wardoff's likely early users are technical and may need help validating behavior in their specific Windows environment (edition, policies, admin context, VM setup). This monetizes expertise, not code access.

**Scope of services:**

- Environment review: does Wardoff behave as expected in your specific setup?
- Deployment guidance: autostart, Task Scheduler, elevation strategy for your org
- Log interpretation: reading `--status` and JSONL logs to diagnose issues
- Compatibility report: written notes on what works, what does not, and what is environment-dependent

**Setup checklist (when ready):**

- [ ] Create a simple booking/contact form (Cal.com, Calendly, or plain email)
- [ ] Define rates (suggested starting point: 50 €/hour or 150 € fixed session)
- [ ] Add a "Professional support" section in README or a dedicated SUPPORT.md
- [ ] Keep scope honest: this is expert help, not an SLA or a support contract

**Rules:**

- Never promise things Wardoff cannot deliver. If a customer's environment makes a layer unreliable, say so.
- Consulting insights that reveal real bugs become free fixes for everyone.
- Do not let consulting eat all maintenance time. Cap at a sustainable number of hours per month.

## Models explicitly rejected

These were evaluated and rejected. Revisit only if the project fundamentally changes in scale or audience.

| Model | Why rejected |
| --- | --- |
| Hard paywall on core | Kills trust, contradicts market norms, alienates the audience |
| Enterprise sales motion | No packaging, no SLA, no support process — premature |
| Feature gating (free vs. pro) | The core protective behavior is the product. Gating it makes Wardoff less honest |
| Ads or telemetry resale | Directly at odds with transparency and privacy values |
| Dual-licensing / relicensing | Project is MIT-positioned. Tightening the license creates trust questions |
| Venture-style growth | Wardoff is a narrow utility, not a blitz-scale startup |
| Open Collective (now) | Admin overhead with no revenue to manage. Revisit if sponsorship grows |

## Evolution roadmap

This section tracks when to reassess or add new monetization elements.

| Trigger | Action |
| --- | --- |
| GitHub Sponsors + BMAC set up | Layer 1 is live. Monitor monthly. |
| Authenticode certificate acquired | Layer 2 is live. Ship first signed binary with next release. |
| First 10 signed binary sales | Reassess pricing. Consider adding a "pay what you want" option. |
| First 3 consulting requests | Layer 3 is live. Define rates and booking flow. |
| First 50 signed binary sales | Consider Winget/Scoop/Chocolatey packaging (free, unsigned) alongside signed direct download. |
| Consistent monthly revenue > 200 € | Consider Open Collective for transparent finances. |
| First enterprise inquiry | Evaluate whether a lightweight support package makes sense. Do not over-engineer. |
| v1.0 ships (IFEO, Event Log, profiles) | Reassess signed binary pricing. Consider a "v1 upgrade" for existing buyers or keep it flat. |

## What this document does NOT cover

- Technical architecture (see `ARCHITECTURE.md`)
- Product roadmap and feature scope (see `../PLAN.md`)
- Community outreach strategy (see `EARLY_ADOPTER_OUTREACH_PLAN.md`)
- Manual validation (see `MVP_VALIDATION_MATRIX.md` and `MANUAL_VALIDATION_PLAN.md`)
