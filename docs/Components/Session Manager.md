---
tags:
  - component
  - ingestion
  - collection
component-id: C25
binary: xustive-cli crawld (library only)
status: decision logic built; login, warm-up and connectors not built
updated: 2026-08-27
---

# Session Manager

> **ID** C25 · **Module** `crates/xustive-ingest/src/session/` (`pool.rs`, `lifecycle.rs`,
> `budget.rs`, `budget_store.rs`, `detection.rs`, `crypto.rs`) · **Upstream** social connectors ·
> **Downstream** [[Proxy Manager]], [[Fingerprint Engine]]

## 0. What exists today (2026-08-27)

The same "build the engine, defer the fuel" split as the [[Proxy Manager]]. **Built and tested**:
`Identity` / `SessionPool::acquire(platform, capability)` returning a `SessionLease` that has *no
setter* for proxy or fingerprint (the pinning invariant, enforced by absence); `Lifecycle` tiers
and the doubling quarantine cooldown; `BudgetLimits` and `BudgetSpend` pacing with jitter and a
per-identity active window; the fail-closed Redis `BudgetStore`; `detection::classify` including
canary-based soft-ban detection; `CookieCrypto` (XChaCha20-Poly1305, zeroising). **Not built**:
account acquisition, real login flows, warm-up browsing, the canary fetch loop, identity
persistence in Redis (the pool is in-memory), metrics — and the social connectors that would call
any of it ([[Social Connector - Instagram]], [[Social Connector - Facebook]],
[[Social Connector - TikTok]]). `Platform` is `Instagram | Facebook | Tiktok`. The `#[async_trait]
SessionPool` in §3 is the intended shape; the built one is synchronous, and `report` is the
caller's `record_spend` plus `classify`.

## 1. Purpose

Own the **identities** used for direct collection: accounts, cookies, sessions, and the budgets that
keep them alive. Introduced by [[ADR-0009 - Direct Collection for Social Platforms]].

This is the component that determines whether collection works for months or for days. Every other
part of the scraping stack is replaceable; a burned account pool takes weeks to rebuild.

## 2. Responsibilities

**In scope**: account pool and lifecycle; login and session establishment; cookie/token persistence;
the identity pinning invariant; per-account request budgets; challenge and ban detection; quarantine
and recovery; warm-up scheduling; credential storage.

**Out of scope**: account *acquisition* (a procurement question, not an engineering one); proxy
health (→ [[Proxy Manager]]); fingerprint generation (→ [[Fingerprint Engine]]); request signing
(→ [[Signature Service]]); what to fetch (→ connectors).

## 3. Interface

```rust
#[async_trait]
pub trait SessionPool: Send + Sync {
    /// Lease an identity for one unit of work on a platform.
    async fn acquire(&self, platform: Platform, need: Capability) -> Result<SessionLease, PoolError>;
    /// Mandatory. Health scoring depends entirely on outcome feedback.
    async fn report(&self, lease: &SessionLease, outcome: SessionOutcome);
}

pub struct SessionLease {
    pub identity_id: IdentityId,
    pub cookies: CookieJar,
    pub headers: HeaderMap,          // from [[Fingerprint Engine]]
    pub proxy: ProxyUri,             // pinned, from [[Proxy Manager]]
    pub fingerprint: FingerprintId,  // pinned
    pub budget_remaining: u32,
}

pub enum Capability { Anonymous, LoggedIn, GroupMember(GroupId) }

pub enum SessionOutcome {
    Ok { bytes: u64, items: u32 },
    Empty,                    // 200 OK, zero items — suspected cloaking
    RateLimited,
    Challenge(ChallengeKind), // captcha | checkpoint | 2fa | suspicious_login
    Banned,
    NetworkError,
}
```

`Capability::Anonymous` is requested first wherever it works — an anonymous session risks nothing but
a proxy IP, while a logged-in session risks an asset that took weeks to warm.

## 4. Internal Design

### 4.1 Identity record

An **identity** is the unit of currency, not an account:

```jsonc
{
  "identity_id": "ig-014",
  "platform": "instagram",
  "tier": "mature",                  // fresh | warming | mature | quarantined | burned
  "credentials_ref": "secrets://ig/014",   // never inline
  "totp_secret_ref": "secrets://ig/014/totp",
  "cookie_jar": "enc:…",             // encrypted at rest
  "proxy_id": "res-dz-0912",         // PINNED
  "fingerprint_id": "chrome-131-win11-fr", // PINNED
  "device_profile": { "…": "…" },    // PINNED (mobile API paths)
  "created_at": 1748000000,
  "warmed_until": 1750592000,
  "health": 0.86,
  "budget": { "hourly": 60, "daily": 400, "used_hour": 12, "used_day": 210 },
  "last_challenge_at": null,
  "consecutive_empty": 0,
  "capabilities": ["LoggedIn", "GroupMember:1234567890"]
}
```

### 4.2 The pinning invariant ★

> **`account ↔ proxy ↔ fingerprint ↔ device` is stable for the life of the identity.**

An account that logs in from a Chrome-on-Windows fingerprint over an Algiers residential IP, and then
appears as Safari-on-iOS over a German datacentre IP, is flagged on the *first* request. Platforms
correlate these signals precisely because legitimate users do not change them.

Consequences enforced in code:
- `acquire` returns the identity's pinned proxy and fingerprint; callers **cannot** substitute them.
- A pinned proxy going permanently dead **quarantines the identity** rather than reassigning it.
  Reassignment is a deliberate, logged, rate-limited operation with a cool-down.
- Rotation happens *between* identities, never *within* one.

This is the rule most scrapers get wrong, and it is why rotation-heavy designs burn pools fast.

### 4.3 Lifecycle

```
acquired → fresh ──(warm-up, 7–14 days)──► warming ──► mature ──► (working)
                                                          │
                                   challenge/ban ─────────┼──► quarantined ──► recovered → warming
                                                          └──► burned (retired, never reused)
```

| Tier | Budget | Use |
|:---|:---|:---|
| `fresh` | 0 collection requests | warm-up traffic only |
| `warming` | 25 % of mature | low-value sources, tolerant of loss |
| `mature` | full | primary collection |
| `quarantined` | 0 | cooling down after a challenge |
| `burned` | 0 | retired permanently; credentials revoked |

### 4.4 Warm-up

New identities do **not** scrape. For `warmup_days` they perform human-shaped browsing on their
pinned proxy and fingerprint: view a feed, open a few profiles, idle, return at varying times of day,
with session lengths and gaps drawn from a distribution rather than a constant.

Warm-up is unglamorous and skipping it is the second-biggest cause of pool loss after violating §4.2.
An identity that starts issuing 400 profile requests an hour on day one looks exactly like what it is.

### 4.5 Budgets and pacing

Per-identity, not per-IP — platforms rate-limit the account far more tightly than the address.

| Setting | Instagram | Facebook | TikTok (anon) |
|:---|:---|:---|:---|
| `hourly_requests` | 60 | 45 | 300 |
| `daily_requests` | 400 | 300 | 2 500 |
| `min_gap_ms` | 2 500 | 3 000 | 800 |
| `jitter` | ±40 % | ±40 % | ±30 % |
| `session_length_min` | 8–25 | 8–25 | n/a |
| `daily_active_hours` | 6–14, offset per identity | same | n/a |

Starting values are deliberately conservative. They are tuned **downward on the first sign of
challenge pressure and upward only slowly**, because the feedback signal (a ban) arrives long after
the behaviour that caused it. Only the Instagram column is coded (`BudgetLimits::instagram()`,
also the default — the tightest platform is the safe default); the active window is 10 h long,
platform-wide, with the *start hour* per identity (`active_start_hour`) so the pool is not
uniformly busy. `scaled(ratio)` applies the tier's budget ratio.

Budget counters live in Redis (`BudgetStore`) as per-period keys — `{ns}:budget:{id}:h:{hour}` and
`…:d:{day}` with a TTL just past the period, so a new hour is a new key that starts at zero. A
durable sentinel `{ns}:budget:alive` is set at start-up; if it is ever missing the data was
flushed and **every spend is denied** until an operator re-initialises. Recovery is a decision,
not an accident (§7).

Requests are shaped, not just spaced: a diurnal curve per identity, aligned to `Africa/Algiers`, so
an identity is not uniformly active at 04:00.

### 4.6 Detection — including silent detection

Platforms increasingly *cloak* rather than error: HTTP 200, valid HTML, zero results. A connector
that trusts status codes reports success while collecting nothing.

| Signal | Detection | Response |
|:---|:---|:---|
| Explicit rate limit (429, `/challenge/`, checkpoint URL) | status + URL match | quarantine identity, cool down |
| Captcha / checkpoint interstitial | body fingerprint | quarantine, flag for recovery |
| Login wall on previously-anonymous content | body fingerprint | downgrade capability, retry with `LoggedIn` |
| **Empty result on a source known to have content** | `consecutive_empty ≥ 3` **and** canary disagrees | treat as **soft ban**, quarantine |
| Truncated result sets (10 items where 50 expected) | expected-count model per source | degrade identity health, slow down |
| Response shape drift (fields missing) | schema check | alert — likely a platform change, not a ban |
| Latency spike + identical payloads | heuristic | suspected shadow-throttle |

**Canaries** are the ground truth: a small set of known-stable public objects with known content,
fetched every 15 minutes by a dedicated low-value identity per platform. When canaries return content
and production identities return empty, the identities are being cloaked. When canaries *also* return
empty, the platform changed and it is a code problem.

### 4.7 Quarantine and recovery

```
challenge → quarantine (cooldown 6h → 24h → 72h, doubling)
         → recovery attempt (manual or semi-automated):
             solve checkpoint · confirm via TOTP · re-verify email/phone
         → success → warming (25 % budget, 7 days)
         → 3rd quarantine → burned
```

2FA/TOTP is generated from the stored secret. Anything requiring SMS, ID upload, or a human decision
is surfaced to an operator queue — it is not automated.

### 4.8 Storage and crypto

Cookies and credentials are encrypted at rest with a 32-byte key from the secrets file
(XChaCha20-Poly1305, a fresh random 24-byte nonce per seal, prepended so a blob is self-contained
`nonce ‖ ct`; wrong key, corruption and tampering are deliberately one indistinguishable error).
`CookieCrypto::open` returns a `Zeroizing` buffer. Redis would hold only ciphertext; no identity
is persisted yet. Plaintext exists in process memory for the duration
of a lease and is zeroised on drop. Credentials never appear in logs, metrics, traces, or DLQ
payloads — the telemetry lint covers `credentials`, `cookie`, `password`, and `totp` in addition to
query fields ([[Observability]] §1).

## 5. Configuration

Nothing in `config/*.toml`; the built types take these as constructor arguments
(`SessionPool::new(limits, warming_ratio)`, `Lifecycle::fresh(burn_after)`,
`quarantine_cooldown(initial, max, n)`, `classify(.., empty_threshold)`). The table is the
design target.

| Key | Default |
|:---|:---|
| `warmup_days` | 10 |
| `warming_budget_ratio` | 0.25 |
| `quarantine_initial_h` | 6 |
| `quarantine_max_h` | 72 |
| `burn_after_quarantines` | 3 |
| `consecutive_empty_threshold` | 3 |
| `canary_interval_s` | 900 |
| `min_pool_size` | 5 per platform |
| `reassign_proxy_cooldown_h` | 168 |
| `budgets.*` | §4.5 table |
| `anonymous_first` | `true` |

## 6. Data

Built: `{ns}:budget:{id}:h:{hour}`, `…:d:{day}` and the `{ns}:budget:alive` sentinel (§4.5).
Designed, not built (2026-08-27): `identity:{id}` (encrypted), `identity:pool:{platform}` (sorted
set by health), `canary:{platform}:{object}` (last-known-good); the secrets file with credentials
and TOTP seeds. Nothing identity-related touches the index.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Pool exhausted (all quarantined) | pool size gauge | **stop collection for that platform**, page — do not fall back to unpinned identities |
| Pinned proxy dead | [[Proxy Manager]] | quarantine identity; reassign only after cool-down |
| Login flow changed | login failure rate spike | alert; likely a platform change, needs code |
| Mass simultaneous challenge | challenge rate > 30 % in 15 min | halt the platform entirely, page — usually means a defence rolled out |
| Cloaked responses | canary disagreement | quarantine affected identities |
| Credential decryption failure | crypto error | **Fatal** — refuse to start rather than run unauthenticated |
| Clock skew breaking TOTP | validation failure | NTP check; alert |
| Budget counter lost (Redis flush) | counter absent | fail **closed** — assume budget spent for the period |

The budget fail-closed rule matters: after a Redis restart, assuming zero usage would let every
identity burn its full daily allowance at once.

## 8. Performance

| Metric | Budget |
|:---|:---|
| `acquire` | ≤ 3 ms p95 |
| Login (cold) | ≤ 15 s |
| Session reuse rate | ≥ 95 % of requests use an existing cookie jar |
| Pool health | ≥ 70 % of identities `mature` in steady state |
| Identity lifespan | **≥ 90 days median** ← the metric that actually matters |

## 9. Observability

`xustive_identity_pool_size{platform,tier}`, `xustive_identity_health`,
`xustive_identity_lifespan_days` (histogram, on burn), `xustive_challenge_total{platform,kind}`,
`xustive_ban_total{platform}`, `xustive_empty_response_total{platform,source}`,
`xustive_canary_status{platform}`, `xustive_budget_exhausted_total`,
`xustive_login_duration_seconds`, `xustive_proxy_reassign_total`.

Dashboard **Collection Health**: pool composition by tier, challenge rate, ban rate, canary status,
median identity lifespan, cost per 1 000 documents.

Alerts: `PoolExhausted` (page), `ChallengeSpike` (page), `CanaryDown` (page),
`IdentityLifespanDrop` (ticket — the leading indicator that pacing is too aggressive).

## 10. Security

- Credentials and cookies encrypted at rest; keys in the secrets file, mounted only to
  `xustive-crawler` ([[Security and Privacy]] §7).
- No collection credential grants access to anything in the serving plane; compromise of the crawler
  cannot reach user traffic, which has no egress path anyway
  ([[Deployment Topology]] §3).
- Identity data is operational, not user data. It is the one place we hold account secrets, so it is
  the highest-value target in the system and is treated accordingly.
- Content collected through a logged-in identity is subject to exactly the same downstream duties as
  anything else: deletion propagation, takedowns, no profiling
  ([[ADR-0009 - Direct Collection for Social Platforms]] §"What Does Not Change").

## 11. Testing

Built: unit tests in each `session/*.rs` (tier transitions, budget scaling and jitter, the
doubling cooldown, burn after the third quarantine, `classify` at and below the empty threshold
with the canary both ways, the pinning invariant, crypto round-trip and tamper rejection) and
`tests/budget_store_redis.rs` (fail-closed on a wiped sentinel). The cloaking fixture server,
log-scan and pool-exhaustion items need connectors that do not exist yet.

- Unit: tier transitions, budget accounting with jitter, quarantine backoff, burn rules.
- **Pinning invariant test**: assert no code path can return a lease whose proxy or fingerprint
  differs from the identity's pinned values.
- Fail-closed test: wipe budget counters, assert the identity is treated as spent.
- Cloaking simulation: fixture server returning 200-with-zero-items; assert soft-ban detection fires
  at the threshold and not before.
- Canary logic: canary-ok + production-empty → quarantine; canary-empty + production-empty → alert as
  a platform change, **not** a ban.
- Crypto: cookie jar round-trip; assert plaintext never reaches a log sink (log-scan test).
- Pool exhaustion: quarantine every identity, assert collection stops rather than degrading.

## 12. Open Questions

- [ ] Pool sizing: how many identities per platform for the target crawl rate? Derive from
      `daily_requests` per identity against the document target, with 40 % headroom for quarantine.
- [ ] Account acquisition and replacement — sourcing, cost, and who owns it operationally.
- [ ] Do we join closed groups with our identities? Per-source decision, currently default no
      ([[ADR-0009 - Direct Collection for Social Platforms]] §Decision.6).
- [ ] Semi-automated checkpoint recovery, or always route to a human queue?
- [ ] Should identity lifespan feed back into pacing automatically (a control loop), or stay a
      human-tuned parameter? A control loop that reacts to bans is reacting to week-old information.

## Related

[[ADR-0013 - Direct SERP Collection for Discovery]] ·
[[ADR-0009 - Direct Collection for Social Platforms]] · [[Proxy Manager]] · [[Fingerprint Engine]] ·
[[Signature Service]] · [[Social Connector - Facebook]] · [[Social Connector - Instagram]] ·
[[Social Connector - TikTok]] · [[Security and Privacy]] · [[Observability]]
