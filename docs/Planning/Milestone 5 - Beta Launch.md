---
tags:
  - planning
  - milestone
milestone: 5
status: not-started
updated: 2026-08-06
---

# Milestone 5 - Beta Launch

> **Goal:** put Xustive in front of real Algerian users, with the legal, accessibility, and
> operational commitments actually met rather than intended.
> **Exit gate:** legal checklist cleared; WCAG 2.2 AA verified; takedown process staffed and
> rehearsed; public beta live with a working feedback loop.
> Parent: [[TODO]] · Previous: [[Milestone 4 - Quality and Operations]]

---

## Why This Milestone Exists

Launching is not deploying. The gap between "the system works" and "we can responsibly operate this
in public" is filled with obligations that have no code: someone must be answerable for takedowns,
the privacy policy must describe what the system actually does, and the Darija strings must have been
read by someone who speaks Darija.

This milestone is mostly that gap.

---

## M5-T01 — Legal checklist cleared

Every item from [[Legal and Compliance]] §8:

- [ ] M5-T01.1 Legal entity established and able to receive formal requests
- [ ] M5-T01.2 All **⚖ VERIFY** items resolved in writing by counsel
- [ ] M5-T01.3 ANPDP position clarified; notification/authorisation filed if required
- [ ] M5-T01.4 GDPR applicability determined; representative appointed if needed
- [ ] M5-T01.5 Privacy policy published — **and verified line-by-line against actual behaviour**
- [ ] M5-T01.6 Terms of use published
- [ ] M5-T01.7 Platform API approvals in place, or the corresponding connectors confirmed disabled
- [ ] M5-T01.8 Model and dependency licences audited for commercial use
- [ ] M5-T01.9 Data-processing record maintained
- [ ] M5-T01.10 Intermediary-liability position understood and documented

> M5-T01.5 is the one that catches people. A privacy policy claiming "we don't store searches" is only
> defensible if [[ADR-0008 - No Query Logging]] is still true of the code that shipped. Verify, don't
> assume.

## M5-T02 — Accessibility: full AA pass

- [ ] M5-T02.1 `axe-core` clean across every page × 4 languages × 2 themes
- [ ] M5-T02.2 **Manual screen-reader passes**: NVDA/Firefox, VoiceOver/iOS, TalkBack/Android
- [ ] M5-T02.3 Keyboard walk of every flow with a visible focus ring at each stop
- [ ] M5-T02.4 320 px @ 200 % zoom reflow; text-spacing overrides
- [ ] M5-T02.5 Contrast verification of every token pair in both themes
- [ ] M5-T02.6 `prefers-reduced-motion` audit
- [ ] M5-T02.7 No-JS flow verified
- [ ] M5-T02.8 Live-region behaviour verified — especially that the summary announces **once**
- [ ] M5-T02.9 Accessibility statement published, including known gaps
- [ ] M5-T02.10 Ideally: testing by an actual daily AT user ([[UI - Accessibility]] §10)

## M5-T03 — Public source submission

- [ ] M5-T03.1 `POST /sources` enabled with validation, `SafeUrl`, and rate limits
- [ ] M5-T03.2 Self-hosted proof-of-work or captcha — **no third-party script**
      ([[Security and Privacy]] P7)
- [ ] M5-T03.3 Sandboxed probe fetch building the review packet
- [ ] M5-T03.4 Moderation queue and reviewer workflow
- [ ] M5-T03.5 `auto_approve` locked false; default `trust_tier` C
- [ ] M5-T03.6 Contact-email retention capped at 30 days
- [ ] M5-T03.7 `/submit` page and UI flow
- [ ] M5-T03.8 SSRF suite run against the live submission endpoint
- [ ] M5-T03.9 Reviewer capacity planned; alert when the queue exceeds 500

## M5-T04 — Static pages

- [ ] M5-T04.1 `/about` — what Xustive is, what it indexes, what it does not
- [ ] M5-T04.2 `/privacy` — how the no-logging claim is **structurally** enforced, not just promised
- [ ] M5-T04.3 `/bot` — user-agent, crawl behaviour, contact, how to block or rate-limit us
- [ ] M5-T04.4 `/terms`
- [ ] M5-T04.5 A takedown/contact route that a non-technical person can find and use
- [ ] M5-T04.6 All pages in four languages, RTL-correct, and within the client budget

## M5-T05 — Takedown process

- [ ] M5-T05.1 Named responsible person and a documented deputy
- [ ] M5-T05.2 Intake, triage, and decision procedure per request category
- [ ] M5-T05.3 72 h SLA with monitoring and an alert on breach
- [ ] M5-T05.4 **End-to-end rehearsal** on a real indexed document, including a re-crawl attempt
- [ ] M5-T05.5 Audit-log review procedure
- [ ] M5-T05.6 Decide whether to publish a transparency report

## M5-T06 — Native-speaker string review

- [ ] M5-T06.1 Full review of `ar` strings
- [ ] M5-T06.2 Full review of `ary` (Darija) strings — **written, not machine-translated**
- [ ] M5-T06.3 Full review of `fr` strings
- [ ] M5-T06.4 Verify Algerian month names in display **and** parsing
- [ ] M5-T06.5 Tone check: is the Darija chrome warm and clear, or does it read as a novelty?
- [ ] M5-T06.6 Verify the plural forms actually render correctly for Arabic's six categories

## M5-T07 — Beta programme

- [ ] M5-T07.1 Recruit 50–100 beta users across languages, devices, and regions
- [ ] M5-T07.2 Feedback channel that works **without query logging** — a "report this result" button
      that submits only what the user chooses to send
- [ ] M5-T07.3 Structured intake: every quality complaint becomes a golden-set row
      ([[Testing Strategy]] §12)
- [ ] M5-T07.4 Weekly triage of feedback into the backlog
- [ ] M5-T07.5 Measure what we ethically can: zero-result rate by language, latency, error rate
- [ ] M5-T07.6 Real-device testing on low-end Android over 3G

> M5-T07.2 is the design problem of this milestone. We deliberately have no query logs
> ([[ADR-0008 - No Query Logging]]), so the *user* must be the one who volunteers a failing query.
> Making that one tap, with a clear statement of exactly what gets sent, is what determines whether
> the feedback loop exists at all.

## M5-T08 — Launch operations

- [ ] M5-T08.1 Launch runbook: sequence, checks, owners, go/no-go criteria
- [ ] M5-T08.2 Rollback plan for every component, rehearsed
- [ ] M5-T08.3 On-call rota and escalation path for the launch window
- [ ] M5-T08.4 Capacity headroom for a traffic spike; load-shedding thresholds confirmed
- [ ] M5-T08.5 Status page or equivalent
- [ ] M5-T08.6 Communications plan, including what we say about coverage limitations
- [ ] M5-T08.7 Post-launch review scheduled at +1 week and +1 month

---

## Exit Gate

| Check | Threshold |
|:---|:---|
| Legal | every checklist item cleared; counsel sign-off on record |
| Accessibility | WCAG 2.2 AA verified by automated **and** manual passes |
| Takedown | rehearsed end to end; named owner; SLA monitored |
| Localisation | all four languages reviewed by native speakers |
| Submissions | live, rate-limited, human-moderated, SSRF-tested |
| Feedback | a working loop that respects the no-logging commitment |
| Operations | runbooks, rota, rollback plan, status page in place |
| Honesty | public materials accurately describe coverage and limitations |

## Risks

| Risk | Mitigation |
|:---|:---|
| Legal items slip and block launch indefinitely | started in M3; tracked as blockers B1/B2 in [[TODO]] §5 |
| No feedback loop without query logs | M5-T07.2 designs it deliberately rather than hoping |
| Coverage disappoints users expecting "all of Algerian Facebook" | be explicit about what is indexed; this is a communications decision made in advance |
| Accessibility treated as a checkbox | manual AT passes are an exit gate, not a nice-to-have |
| Takedowns arrive faster than one person can handle | capacity planned; alert on SLA breach |
| Launch traffic exceeds capacity | shedding thresholds confirmed in M4 load tests |

## After Beta

Candidates for the next cycle, deliberately **not** in scope now:

- Semantic/dense retrieval fused with lexical ([[Ranking and Relevance]] §8)
- Cross-language duplicate clustering ([[Deduplication Service]] §12)
- Text→image CLIP search ([[Image Pipeline]] §12)
- Whisper fine-tuning for Darija ([[Speech to Text]] §12)
- Tamazight UI and content support ([[UI - RTL and Localization]] §11)
- Vertical search (jobs, classifieds) from structured extraction

## Related

[[TODO]] · [[Legal and Compliance]] · [[UI - Accessibility]] · [[Admin and Source Submission]] ·
[[ADR-0008 - No Query Logging]] · [[Testing Strategy]] · [[Milestone 4 - Quality and Operations]]
