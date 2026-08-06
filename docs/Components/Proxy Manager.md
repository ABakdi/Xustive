---
tags:
  - component
  - platform
  - ingestion
  - collection
component-id: C21
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Proxy Manager

> **ID** C21 · **Binary** `xustive-crawler` · **Upstream** [[Web Fetcher]], [[Session Manager]], social connectors · **Downstream** external network

> Rebuilt under [[ADR-0009 - Direct Collection for Social Platforms]]. The previous
> `halt_and_flag`-only design is superseded; this note is the design in force.

## 1. Purpose

Manage outbound network identity: pools, health, geographic and ASN placement, session pinning, ban
attribution, and cost. Two distinct jobs sharing one component:

- **Open web** — rate distribution and reliability. Most traffic goes out `direct`.
- **Platforms** — IP reputation *is* an authentication factor. Datacentre addresses are classified
  instantly; residential and mobile addresses are the entry ticket.

## 2. Responsibilities

**In scope**: pool configuration and health; per-platform pool policy; sticky session pinning; ban
attribution (proxy vs host vs identity); circuit breakers; geo/ASN targeting; bandwidth accounting
and cost metrics; credential handling.

**Out of scope**: account state (→ [[Session Manager]]); TLS/browser fingerprints
(→ [[Fingerprint Engine]]); scheduling (→ [[Crawler Orchestrator]]); robots policy
(→ [[Politeness and Robots]]).

## 3. Interface

```rust
#[async_trait]
pub trait ProxyPool: Send + Sync {
    async fn acquire(&self, target: &Host, policy: PoolPolicy) -> Result<Lease, PoolError>;
    /// Pinned acquisition — an identity's proxy never varies. See [[Session Manager]] §4.2.
    async fn acquire_pinned(&self, proxy_id: &ProxyId) -> Result<Lease, PoolError>;
    fn report(&self, lease: &Lease, outcome: Outcome);   // ok | timeout | refused | blocked | challenged | banned
}

pub struct PoolPolicy { pub kind: PoolKind, pub geo: Option<CountryCode>, pub sticky: bool }
pub enum PoolKind { Direct, Datacenter, Residential, Mobile }
```

`report` is mandatory. A lease dropped without a report counts as a timeout — otherwise leaked leases
silently corrupt every health score in the pool.

## 4. Internal Design

### 4.1 Pools

| Pool | Cost model | Latency | Use |
|:---|:---|:---|:---|
| `direct` | free | best | **default** — all open-web `.dz` crawling |
| `datacenter` | flat / IP | good | high-volume open web where one IP hits per-host limits |
| `residential` | **per GB** | +150–400 ms | Instagram, Facebook |
| `mobile` | **per GB**, highest | +300–800 ms | Facebook where residential is insufficient; highest-trust IPs |

Policy is per source class, set in [[Data Sources Registry]]:

```toml
web         = { kind = "direct" }
tiktok_anon = { kind = "datacenter", geo = "DZ" }
instagram   = { kind = "residential", geo = "DZ", sticky = true }
facebook    = { kind = "residential", geo = "DZ", sticky = true }
```

**Most traffic must stay on `direct`.** Residential bandwidth is the largest variable cost in the
system, and a misconfigured source that routes open-web crawling through residential IPs can consume
a month's budget in a day. §8 makes cost a tracked metric for exactly this reason.

### 4.2 Geographic and ASN placement

Algerian content viewed from Algerian addresses is unremarkable; the same requests from a Frankfurt
datacentre are not. Targeting rules:

- Prefer `geo = DZ` for all platform collection.
- Spread across **≥ 4 distinct ASNs** (Algérie Télécom, Mobilis, Djezzy, Ooredoo ranges). A pool
  concentrated in one ASN and one /16 correlates trivially.
- Cap identities per /24 at `max_identities_per_subnet` (3). Fifty accounts behind one address is the
  clearest possible cluster signal.
- The proxy's geography must agree with the identity's `accept-language` and timezone
  ([[Fingerprint Engine]] §4.2) — an Algiers IP presenting `en-US` and `America/New_York` is
  incoherent.

### 4.3 Session pinning

For platform pools, `sticky = true` and the proxy is **pinned to the identity for its lifetime**
([[Session Manager]] §4.2). `acquire_pinned` is the only path platform connectors use.

When a pinned proxy dies permanently:
1. Quarantine the identity — do **not** silently reassign.
2. After `reassign_cooldown` (7 days), reassign to a proxy in the **same ASN and city** where
   possible, then return the identity to `warming`.
3. Record the reassignment. A rising `proxy_reassign_total` means the provider's pool is churning,
   which will show up as ban rate within a week or two.

### 4.4 Health scoring

EWMA per proxy:

```
health = 0.40·success_rate + 0.20·(1 − normalised_latency)
       + 0.30·(1 − block_rate) + 0.10·(1 − challenge_rate)
```

| State | Condition | Behaviour |
|:---|:---|:---|
| `healthy` | ≥ 0.7 | eligible |
| `degraded` | 0.4–0.7 | eligible at reduced weight; not for new pinning |
| `quarantined` | < 0.4 or 3 consecutive failures | excluded for `quarantine_s`, then probed |
| `dead` | 5 consecutive quarantines | removed; alert |

Active checks every 60 s against a neutral endpoint, plus an **egress-IP assertion**: the observed
public address must equal the expected proxy address. A proxy silently falling back to direct egress
would expose the crawler host and defeat the whole arrangement.

### 4.5 Failure attribution

A failure means the proxy is bad, the host is down, or the identity is flagged. Getting this wrong
produces the classic spiral where one dead host quarantines an entire pool.

| Pattern | Blame | Action |
|:---|:---|:---|
| ≥ 3 different proxies fail on one host within 60 s | **host** | open the host/platform breaker; leave proxies alone |
| One proxy fails across ≥ 3 different hosts | **proxy** | quarantine the proxy |
| One identity challenged while its proxy is healthy elsewhere | **identity** | quarantine the identity ([[Session Manager]] §4.7) |
| Challenge rate rising across many identities on one ASN | **ASN reputation** | drain that ASN, redistribute |
| Challenge rate rising platform-wide | **defence rollout** | halt the platform, page |

### 4.6 Response to blocking — the graded ladder

Replaces the old `halt_and_flag`. `on_blocked` is now per pool kind:

| Signal | Open web (`direct`) | Platform pools |
|:---|:---|:---|
| `robots.txt` disallow | **do not fetch** — unchanged ([[Politeness and Robots]]) | n/a |
| 429 / `Retry-After` | honour exactly, open breaker | honour, halve identity budget, back off ≥ 15 min |
| 403 / anti-bot challenge | halt, flag for review | quarantine identity, cool down, resume on a different identity |
| Captcha / checkpoint | halt, flag | quarantine identity → [[Session Manager]] recovery |
| Silent empty responses | n/a | canary comparison → soft-ban handling |
| Platform-wide challenge spike | n/a | **halt the platform**, page |

The open-web column is deliberately unchanged: [[ADR-0009 - Direct Collection for Social Platforms]]
altered the platform stance, not the commitment to well-behaved crawling of ordinary websites. A
small Algerian news site that asks us not to crawl a path is still obeyed.

### 4.7 Circuit breakers

Shared state in Redis (`breaker:{host}`, `breaker:platform:{p}`, `breaker:asn:{n}`) so all crawler
replicas agree. Cooldown 60 s doubling to a 30-minute ceiling; platform breakers start at 15 min
because platform-level blocks are slower to clear.

## 5. Configuration

| Key | Default |
|:---|:---|
| `default_pool` | `direct` |
| `pools.*` | §4.1 table |
| `health_check_interval_s` | 60 |
| `quarantine_s` | 300 |
| `sticky_ttl_s` | lifetime (pinned) |
| `reassign_cooldown_days` | 7 |
| `max_identities_per_subnet` | 3 |
| `min_distinct_asns` | 4 |
| `max_concurrent_per_proxy` | 4 |
| `attribution_window_s` | 60 |
| `bandwidth_budget_gb_month` | set per deployment — **alerts at 80 %** |
| `credentials_path` | secrets file |

## 6. Data

Redis: `proxy:{id}` (health, counters, ASN, geo), `breaker:*`, `pin:{identity}` → proxy,
`bandwidth:{pool}:{month}`. Credentials in the mounted secrets file, readable only by
`xustive-crawler`.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| All platform proxies unhealthy | pool gauge | **halt that platform** — never fall back to `direct` for platform collection; a datacentre IP after residential is an obvious tell |
| Credentials rejected (407) | status | quarantine, alert — usually expired billing |
| Proxy leaks real egress IP | egress assertion | quarantine immediately, alert |
| Bandwidth budget exhausted | counter | stop residential/mobile pools, continue `direct`, page |
| Provider pool churn | `proxy_reassign_total` rising | review provider; expect ban-rate follow-on |
| ASN reputation collapse | per-ASN challenge rate | drain the ASN |
| Redis unavailable | error | local health only; breakers become per-replica (documented degradation) |
| Host explicitly blocks us (open web) | 403 | halt and flag — unchanged |

## 8. Performance and Cost

| Metric | Budget |
|:---|:---|
| `acquire` / `acquire_pinned` | ≤ 1 ms p95 |
| Added latency, residential | ≤ 400 ms p95 |
| Added latency, mobile | ≤ 800 ms p95 |
| Health check overhead | ≤ 1 % of requests |
| **Cost per 1 000 documents** | tracked per source — the number that decides whether a source is worth collecting |
| Bandwidth per document | ≤ 400 KB average (drives residential spend directly) |

Bandwidth per document is the lever: fetching full pages with images through residential proxies is
what makes a source expensive. Prefer JSON endpoints and embedded-JSON paths
([[Signature Service]] §4.6), and fetch media through `direct` or `datacenter` where the CDN permits.

## 9. Observability

`xustive_proxy_healthy{pool}`, `xustive_proxy_health_score`, `xustive_proxy_outcome_total{outcome}`,
`xustive_proxy_quarantined_total`, `xustive_proxy_banned_total{platform}`,
`xustive_proxy_reassign_total`, `xustive_proxy_egress_mismatch_total` (must be **0**),
`xustive_proxy_bandwidth_bytes{pool}`, `xustive_proxy_cost_per_1k_docs{source}`,
`xustive_asn_challenge_rate{asn}`, `xustive_breaker_open{scope}`.

Dashboard **Collection Health** (shared with [[Session Manager]]). Alerts: `ProxyPoolDegraded`,
`BandwidthBudget80` , `EgressMismatch` (page), `ASNReputationDrop`.

## 10. Security

- Credentials are file-mounted secrets, never in images, env dumps, or logs.
- **Egress segmentation is unchanged and remains the load-bearing privacy control**: only
  `xustive-crawler` has an outbound route. `xustive-api` and `xustive-ml` have none, so no proxy
  misconfiguration can send a user query anywhere ([[Deployment Topology]] §3,
  [[Security and Privacy]] P2).
- Proxy providers see destination hosts but only ever crawl traffic — guaranteed structurally by the
  line above, not by policy.
- Provider selection has its own ethical dimension: residential pools are sometimes assembled from
  users who did not meaningfully consent to their bandwidth being resold. Prefer providers with
  auditable, explicit consent for their exit nodes; record the choice in [[Decision Log]].

## 11. Testing

- Unit: health EWMA transitions, selection weighting, attribution rules
  (3-proxies-1-host vs 1-proxy-3-hosts vs identity-specific).
- **Pinning test**: `acquire_pinned` always returns the same proxy; no path reassigns without the
  cooldown.
- Egress assertion: a proxy that falls back to direct egress is quarantined.
- Subnet/ASN caps: assert no more than `max_identities_per_subnet` share a /24 and that pinning
  respects `min_distinct_asns`.
- Open-web posture: a fixture host with `robots.txt` disallow, and one returning 403 — assert we halt
  and flag in both cases and do **not** retry through another pool.
- Platform fallback guard: with all residential proxies down, assert platform collection **halts**
  rather than switching to `direct`.
- Cost accounting: bandwidth counters match transferred bytes within 5 %.

## 12. Open Questions

- [ ] Provider selection: which residential/mobile providers offer genuine DZ coverage across
      multiple ASNs, and can they evidence consent for their exit nodes?
- [ ] Is mobile needed at all, or is residential sufficient for Facebook? Mobile is materially more
      expensive; decide with measured challenge rates in [[Milestone 3 - Ingestion at Scale]].
- [ ] Bandwidth budget per month — needs a real number before residential pools are enabled.
- [ ] Should media fetching route through `direct` by default to keep residential spend down, and
      does that create a correlatable split (page from residential, images from datacentre)?

## Related

[[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] · [[Fingerprint Engine]] ·
[[Web Fetcher]] · [[Politeness and Robots]] · [[Error Handling and Resilience]] ·
[[Data Sources Registry]] · [[Security and Privacy]]
