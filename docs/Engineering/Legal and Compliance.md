---
tags:
  - engineering
  - legal
type: reference
status: draft
updated: 2026-08-27
---

# Legal and Compliance

> [!warning] This is an engineering note, not legal advice
> It records the constraints the team believes apply, so that they shape the architecture. Items
> marked **⚖ VERIFY** should be confirmed with a qualified Algerian lawyer. Do not treat this note as
> clearance.

> [!note] Decision on record
> I have directed that **direct collection** be used for social platforms. I am covered legally, and
> I accept the contractual and legal risk. That decision is recorded in
> [[ADR-0009 - Direct Collection for Social Platforms]] and is not re-litigated here.
>
> This note is therefore no longer a *blocker* on the social connectors. It remains the register of
> obligations that are **independent of collection method** — chiefly personal-data duties and
> takedown handling, which the connectors implement regardless — plus the risk items an operator
> should be able to see in one place.

---

## 1. Why This Note Exists

A search engine that indexes public web pages sits on decades of established practice. One that
collects social media content operates under platform contracts and personal-data law. Those are
different regimes with different consequences, and the design has to reflect the distinction rather
than blur it.

I decided the **platform-contract** question
([[ADR-0009 - Direct Collection for Social Platforms]]) and accepted the risk. I cannot decide the
**personal-data** question the same way — it involves duties owed to the people appearing in the
content, not to the platforms. Those duties are the substance of this note, and the
architecture implements them regardless of collection method:

- takedown path removes content, comments, and vectors permanently, with a re-crawl blocklist
- upstream deletions propagate within 24 h
- no person-centric profiling, no author-history view, no face recognition
- EXIF/GPS stripped from all media
- no *identifiable* user data held at all ([[ADR-0008 - No Query Logging]], as amended by
  [[ADR-0018 - Anonymous Search History]]: normalised query terms may be kept as identifier-free
  counts, off by default — the public privacy page states this in four languages)

The open web is handled separately and conservatively throughout: robots-compliant, honestly
identified, halt-on-block ([[Politeness and Robots]] §4.0).

---

## 2. Four Distinct Legal Surfaces

| Surface | Governs | Primary risk |
|:---|:---|:---|
| **Open web crawling** | robots.txt norms, computer-access law, copyright | low, if we are polite and identified |
| **Platform terms of service** | Meta / TikTok Platform Terms — *contract*, not just etiquette | account/app termination, breach claims |
| **Personal data** | Algerian Law 18-07; GDPR if we touch EU residents | regulatory, and reputational |
| **Content liability** | defamation, copyright, illegal content in the index | takedown obligations |

Conflating these is the common mistake. `robots.txt` compliance says nothing about whether a
platform's terms permit automated collection, and neither says anything about data-protection duties.

---

## 3. Open Web Crawling

Our posture, implemented in [[Politeness and Robots]] and [[Web Fetcher]]:

| Commitment | Where enforced |
|:---|:---|
| Identify honestly (`XustiveBot/1.0` + a public `/bot` page) | [[Web Fetcher]] §4.3 |
| Obey `robots.txt`, `Crawl-delay`, meta-robots, `X-Robots-Tag` | [[Politeness and Robots]] §4 |
| Fail closed when `robots.txt` is unavailable | §4.1 of that note |
| One concurrent request per host, ≥ 1.5 s default delay | §4.2 |
| Back off on 429/503 instead of routing around | §4.3 |
| Never access content behind authentication or a paywall | [[System Architecture]] §8 |
| Honour opt-out requests within 72 h | [[Admin and Source Submission]] §4.3 |

**Copyright:** we index and display *title + short excerpt + link*, which is the standard search-engine
posture. We do not republish full articles, and full `body` text is stored for retrieval and ranking
but never served in full through the API — verified 2026-08-27: `displayedAttributes` in
`crates/xustive-search/src/settings.rs` lists `excerpt` and `body_len` but not `body`
([[Data Model]] §2). Remote thumbnails are proxied and signed rather than hot-linked
([[ADR-0021 - Proxied Thumbnails with Signed URLs]]); the entity panel carries each image's credit
and licence string inside the stored entity ([[Knowledge Store]]).

**⚖ VERIFY** — Algerian copyright treatment of search excerpts and thumbnails; whether any
press-publisher right applies.

---

## 4. Platform Terms of Service

This is the sharpest constraint and the one most often hand-waved.

| Platform | Compliant automated access | Practical requirement |
|:---|:---|:---|
| Facebook | Graph API with App Review; Pages need `Page Public Content Access`; **Groups require the group admin to install the app** | business verification + per-object authorisation |
| Instagram | Graph API for authorised Business/Creator accounts; Hashtag Search capped at **30 unique hashtags / 7 days / app** | account opt-in or curated hashtags |
| TikTok | Research API (approval, typically institutional) or Display API (user-authorised) | an approved application |

Meta's and TikTok's terms prohibit automated collection outside those paths. Consequences of breach
are contractual (app termination, account bans, legal action), and in some jurisdictions
unauthorised access claims have been argued on top.

### Our engineering stance (as decided)

Per [[ADR-0009 - Direct Collection for Social Platforms]]:

1. **Direct collection is a first-class path.** Platform APIs are used opportunistically where a
   source has authorised them, because they are cheaper and more stable — not out of obligation.
2. **The open web is treated entirely differently.** `robots.txt`, `Crawl-delay`, honest
   identification, and halt-on-block remain in force for all `web` sources, enforced by a separate
   crawl profile that cannot be overridden per request ([[Politeness and Robots]] §4.0).
3. **Detection is handled with a graded response ladder**, not a halt ([[Proxy Manager]] §4.6).
4. Any *further* change of stance — for example joining closed groups by default, or applying the
   platform profile to open-web sources — requires a new ADR, not a config change.

### Risk register (accepted, not resolved)

Recorded so an operator can see the exposure in one place:

| Risk | Realistic consequence | Mitigation in design |
|:---|:---|:---|
| Breach of platform terms | account/app termination; civil claim by the platform | identity pool is expendable and replaceable; no single point of failure ([[Session Manager]]) |
| Unauthorised-access claims (jurisdiction-dependent) | legal action | owner asserts coverage; scope limited to content visible to an ordinary logged-in user |
| Collected content includes personal data | Law 18-07 duties — **independent of collection method** | §5 below; no profiling, deletions honoured, takedowns |
| Copyright in collected posts | takedown claims | index title + excerpt + link only; full text never served ([[API Contract]] §2) |
| Platform blocks at scale | coverage loss, not legal risk | path ladders and graceful demotion in each connector |

**⚖ VERIFY** (still worth confirming, now for exposure sizing rather than go/no-go) — Algerian
treatment of automated access and of search excerpts; whether the platforms' terms have any practical
enforcement route in Algeria.

---

## 5. Personal Data — Algerian Law 18-07

Law 18-07 (2018) governs the protection of natural persons in the processing of personal data. As we
understand it, and **⚖ VERIFY every line**:

| Area | Our reading | Our implementation |
|:---|:---|:---|
| Scope | applies to processing personal data in Algeria | we process in Algeria, on Algerian data |
| Registration/authorisation | processing may require notification to or authorisation from the national authority (ANPDP) | **⚖ VERIFY** — likely required before beta |
| Lawful basis | consent, or another statutory basis | public-interest information access is our argument; **⚖ VERIFY** it holds for social content |
| Data minimisation | collect only what is needed | we index content, not profiles; no friend graphs, no author histories |
| Rights of the person | access, rectification, erasure | takedown path, 72 h target ([[Admin and Source Submission]] §4.3) |
| Sensitive data | heightened protection (political opinion, health, religion) | public posts inevitably contain it — we do not classify, target, or profile on it |
| Cross-border transfer | restricted | the backends and sidecars sit on an `internal` Docker network with no route out (`scripts/test-egress.sh`); **as built the API itself is a host process**, so the guarantee is by construction for the stores and by code for the API ([[Security and Privacy]] P2). Two opt-in features cross the border by design and are **off by default** — the external summariser and federated search ([[ADR-0017 - Query-Time Federation with External Metasearch]]); the privacy page discloses both |

### Design decisions that already reduce exposure

> These survive [[ADR-0009 - Direct Collection for Social Platforms]] unchanged. The collection
> method changed; the duties owed to the people *in* the data did not — and they are the obligations
> most likely to generate a real complaint from a real person.

- **No user data at all.** No accounts, no query logs, no cookies, no analytics
  ([[Security and Privacy]] §1). The most robust way to comply with data-protection law about users
  is to not hold data about users.
- **No person-centric views.** We index posts, and we deliberately do not offer "all posts by this
  author" — that would turn a search engine into a profiling tool.
- **No face recognition, ever** ([[Image Pipeline]] §10).
- **EXIF/GPS stripped** from user uploads and never read.
- **Upstream deletions honoured** — a post deleted on the platform is removed from our index within
  24 h ([[Social Connector - Facebook]] §4.4).

**⚖ VERIFY** — whether indexing public social posts requires a lawful basis beyond legitimate
interest under 18-07, and whether ANPDP notification is a prerequisite to launch.

### GDPR

If we serve EU-resident users or index EU-resident data subjects at scale, GDPR may apply
extraterritorially. The Algerian diaspora makes this plausible. **⚖ VERIFY** whether GDPR applies and,
if so, whether a representative in the EU is required.

---

## 6. Content Liability and Takedowns

| Obligation | Implementation |
|:---|:---|
| Accept takedown requests | public contact route + `POST /admin/takedown` — **as built (2026-08-27):** neither exists; removal is the operator CLI `xustive-cli takedown --domain <host> --yes`, which previews by default |
| Act within a reasonable time | 72 h target, SLA-alerted — no alert configured yet |
| Remove completely | documents + image vectors + raw bodies for the domain ([[Indexer Worker]] §4.5); comments have no producer yet |
| Prevent resurrection by re-crawl | intended: blocklist checked by [[Politeness and Robots]] before every fetch. **As built:** pair the takedown with `registry disable <source-id>`; the exclusion `Blocklist` type exists but is not persisted or wired into the crawler ([[Runbooks]]) |
| Keep a record | immutable audit log ([[Admin and Source Submission]] §4.4) |

Categories we expect: defamation claims, copyright complaints, personal-data erasure requests, and
illegal content. Each needs a documented handling path and a named responsible person before beta —
that is a [[Milestone 5 - Beta Launch]] deliverable, not an engineering task.

**⚖ VERIFY** — Algerian intermediary-liability rules for search engines; whether a notice-and-takedown
safe harbour exists and what it requires.

---

## 7. Other Obligations

| Item | Status |
|:---|:---|
| Company registration / legal entity | **⚖ VERIFY** — required before accepting submissions or handling takedowns |
| Hosting within Algeria | architectural commitment ([[Deployment Topology]]); **⚖ VERIFY** any licensing requirement for operating a public online service |
| Privacy policy + terms of use | a privacy page ships at `/{lang}/privacy` in ar/fr/en/ary (`web/app/[lang]/privacy`, strings in `web/lib/i18n/messages.ts`) and matches the built behaviour: what is kept as counts, what never is, the two opt-in external features. **Terms of use: not written** (2026-08-27) |
| Accessibility obligations | no known statutory requirement; we target WCAG 2.2 AA regardless ([[UI - Accessibility]]) |
| Open-source licence compliance | AGPL-3.0 (Grafana) is self-hosted only and not distributed; all other components are MIT/Apache/BSD. `cargo-deny` (`make audit`, CI job `dependency audit`) enforces the allowlist |
| Model licences | each model file's licence recorded in `models/LICENSES.md` (audited 2026-08-21) — see the finding below |
| Data licences | DB-IP City Lite (approximate location, [[ADR-0020 - Approximate Location from a Local Database]]) is **CC BY 4.0**: attribution is required and **is not yet rendered in the UI** (2026-08-27; `scripts/fetch-geoip.sh` says the weather card carries it — it does not). Wikidata/Wikipedia facts are CC0 / CC BY-SA and the entity panel keeps per-image credits |

The model-licence item is easy to miss and expensive to discover late: a summarisation model with a
non-commercial licence would invalidate [[Summarizer]]'s design choice
([[ADR-0005 - Local Quantised LLM for Summaries]]).

> [!warning] Finding (2026-08-21, still open 2026-08-27): the default summariser is not commercially licensed
> The model file present by default is **Qwen2.5-3B-Instruct** (GGUF), released under the
> **`qwen-research`** licence — research / non-commercial. The 3B size is the exception in its
> family: Qwen2.5 **1.5B** and **7B** (and 0.5B/14B/32B) are **Apache-2.0**. Local evaluation under
> the research licence is fine. For any commercial launch, pin an Apache-2.0 size via
> `[ml] summariser_model` — 1.5B is already provisioned and is the fast option, 7B is the quality
> option on a GPU — and remove the 3B file from `models/`. This keeps the "Chinese open models
> first" choice intact; it changes a size, not a family. Tracked in `models/LICENSES.md` and the
> checklist below.

---

## 8. Pre-Launch Legal Checklist

Tracked as tasks in [[Milestone 5 - Beta Launch]]:

- [ ] Legal entity established
- [ ] Counsel engaged; **⚖ VERIFY** items resolved in writing (for exposure sizing)
- [ ] ANPDP position clarified; notification/authorisation filed if required
- [ ] Privacy policy and terms published, matching actual system behaviour — privacy page ✅
      (2026-08-27), terms ❌
- [ ] Takedown process documented, staffed, and tested end-to-end — the command exists
      (`xustive-cli takedown --domain … --yes`, [[Runbooks]]); the contact route, the single-URL
      form and the persisted blocklist do not (2026-08-27)
- [ ] Model and dependency licences audited for commercial use — audit ✅ (`models/LICENSES.md`,
      2026-08-21); **the 3B finding above is unresolved**; CLIP weights' licence field and the
      tessdata / whisper conversions still carry ⚠️ "confirm at provisioning"
- [ ] DB-IP CC BY 4.0 attribution rendered wherever the location-derived card appears
- [ ] `/bot` page published with contact details and opt-out instructions — **covers open-web
      crawling**, which is the traffic site owners can see and identify
- [ ] Data-processing record maintained
- [ ] My acceptance of collection risk recorded and dated
      ([[ADR-0009 - Direct Collection for Social Platforms]])

---

## 9. Open Questions

- [ ] Does "self-hosted in Algeria" create any obligation to respond to state data requests? Our
      architecture means there is no user data to hand over — but the index itself exists, and the
      position should be stated publicly and deliberately.
- [ ] Do we publish a transparency report (takedown counts, request categories)?
- [ ] What do public materials say about how content is collected? The privacy policy must be
      accurate about *user* data (which we hold none of). Describing collection methods is a separate
      choice — but nothing published may be actively misleading, since that creates its own exposure
      on top of the accepted one.
- [ ] Residential proxy sourcing: providers vary in whether their exit nodes gave meaningful consent.
      This is both an ethical question and a due-diligence one ([[Proxy Manager]] §10).

## Related

[[Security and Privacy]] · [[Data Sources Registry]] · [[Politeness and Robots]] ·
[[Social Connector - Facebook]] · [[Social Connector - Instagram]] · [[Social Connector - TikTok]] ·
[[Admin and Source Submission]] · [[Decision Log]] · [[Milestone 5 - Beta Launch]] ·
[[ADR-0018 - Anonymous Search History]] · [[Knowledge Store]]
