---
tags:
  - component
  - ingestion
  - collection
component-id: C27
binary: xustive-crawler
status: specified
updated: 2026-08-06
---

# Signature Service

> **ID** C27 · **Binary** `xustive-crawler` · **Upstream** social connectors · **Downstream** none

## 1. Purpose

Produce the platform-specific request parameters that gate access to internal web APIs — TikTok's
`X-Bogus`/`X-Gnarly` and `msToken`, Facebook's `fb_dtsg`/`lsd`/`doc_id`, Instagram's `X-IG-App-ID`
and `X-IG-WWW-Claim`. Introduced by [[ADR-0009 - Direct Collection for Social Platforms]].

These values are computed by obfuscated JavaScript that platforms ship with their web app and
**rotate without notice**. This component isolates that volatility so a signer change breaks one
component, loudly, instead of every connector, silently.

## 2. Responsibilities

**In scope**: extracting signer routines from platform JS bundles; executing them in a sandboxed JS
runtime; token minting, caching, and TTL; parameter harvesting from page HTML; version tracking and
auto-refresh; failure detection.

**Out of scope**: identity and cookies (→ [[Session Manager]]); transport fingerprints
(→ [[Fingerprint Engine]]); knowing which endpoint needs which parameter (→ connectors).

## 3. Interface

```rust
#[async_trait]
pub trait Signer: Send + Sync {
    /// Compute all parameters an endpoint requires.
    async fn sign(&self, req: &SignRequest) -> Result<SignedParams, SignError>;
    /// Harvest per-session constants from a bootstrap page load.
    async fn bootstrap(&self, platform: Platform, lease: &SessionLease) -> Result<SessionConstants, SignError>;
    fn signer_version(&self, platform: Platform) -> SignerVersion;
}

pub struct SignRequest<'a> {
    pub platform: Platform,
    pub url: &'a str,
    pub body: Option<&'a [u8]>,
    pub user_agent: &'a str,        // must match [[Fingerprint Engine]] — signers hash the UA
    pub constants: &'a SessionConstants,
}
pub struct SignedParams { pub query: Vec<(String, String)>, pub headers: Vec<(String, String)> }
```

`user_agent` is required, not optional: several signers incorporate the UA into the computed value,
so a mismatch between the signed UA and the transmitted UA is itself a detection signal.

## 4. Internal Design

### 4.1 Two classes of parameter

| Class | Example | Source |
|:---|:---|:---|
| **Session constants** | `fb_dtsg`, `lsd`, `jazoest`, `X-IG-WWW-Claim`, `doc_id`/`query_hash` | harvested once per session from a bootstrap page load, then reused |
| **Per-request signatures** | `X-Bogus`, `_signature`, `msToken` | computed per request by platform JS |

Session constants are cheap and long-lived. Per-request signatures are the expensive, fragile part.

### 4.2 Signer extraction pipeline

```
1. fetch the platform's JS bundle (webmssdk.js / main.*.js) through a normal identity
2. locate the signer entry point by AST pattern, not by regex on minified text
3. extract the closure plus its dependencies
4. snapshot to data/signers/{platform}/{hash}.js, committed to git
5. record signer_version = content hash + observed_at
```

Extraction is **semi-automated**: the pipeline proposes a candidate and a human confirms the first
time a bundle changes shape. Fully automatic extraction from obfuscated code is brittle, and a wrong
signer produces valid-looking requests that are silently rejected.

### 4.3 Execution sandbox

Signers run in an embedded JS runtime — `deno_core` / `rusty_v8` — **not** a browser. A full browser
per signature would cost ~2 s; the isolate costs ~1 ms.

The runtime provides a minimal shim: `window`, `navigator` (UA, platform, languages from the
identity's [[Fingerprint Engine]] profile), `document` stubs, `screen`, `Date`, `crypto.getRandomValues`.
Signers probe these, so the shim values must match the profile exactly — another coherence surface.

Sandbox constraints: no network, no filesystem, no timers beyond `Date`, 50 ms execution cap, 32 MB
heap cap, isolate recycled every `isolate_reuse_limit` (500) calls to bound leaked state. A panic or
timeout inside the isolate is caught at the task boundary and never reaches the worker.

### 4.4 Token caching

| Token | TTL | Scope |
|:---|:---|:---|
| `msToken` | ~30 min | per identity |
| `fb_dtsg` / `lsd` | session | per identity |
| `X-IG-WWW-Claim` | rolling, updated from response headers | per identity |
| `doc_id` / `query_hash` | until bundle change | global |
| `X-Bogus` | none — per request | — |

Cached in Redis under `signer:{platform}:{identity}:{token}` with the TTL. `X-IG-WWW-Claim` is
special: the platform returns an updated value in response headers and expects it echoed on the next
request. Failing to propagate it is a slow, silent path to being flagged.

### 4.5 Version tracking and auto-refresh

The dominant failure mode is a platform rotating its signer. Detection and response:

```
signature failure rate for a platform > 30 % over 5 min
   → fetch the current JS bundle, compare hash to the pinned signer
   → changed?  →  page + open a re-extraction ticket + halt that platform's collection
   → unchanged? → not a signer problem; investigate identity or fingerprint layers
```

Halting rather than retrying matters: hammering an endpoint with invalid signatures is a strong
detection signal and burns identities for no gain.

A daily job fetches each platform's bundle and diffs the hash, so drift is usually noticed before it
breaks anything.

### 4.6 Fallback ladder

| Tier | Method | Cost | Use |
|:---|:---|:---|:---|
| 1 | Embedded isolate (default) | ~1 ms | everything |
| 2 | Persistent headless browser page evaluating the real bundle | ~200 ms | when extraction is stale but the page still works |
| 3 | Embedded-JSON path (`__UNIVERSAL_DATA_FOR_REHYDRATION__`, `_sharedData`) | page fetch | no signature needed; less data, still useful |
| 4 | Halt | — | signer broken and no alternative |

Tier 3 is worth designing for deliberately: several platforms still embed a hydration blob in public
profile HTML, which needs no signing at all and survives signer rotations entirely. Where it carries
enough content, prefer it — it is the most stable path available.

## 5. Configuration

| Key | Default |
|:---|:---|
| `signer_dir` | `data/signers/` |
| `isolate_timeout_ms` | 50 |
| `isolate_heap_mb` | 32 |
| `isolate_reuse_limit` | 500 |
| `isolate_pool_size` | 4 |
| `failure_rate_halt_threshold` | 0.30 |
| `failure_window_s` | 300 |
| `bundle_check_cron` | `0 4 * * *` |
| `prefer_embedded_json` | `true` |
| `mstoken_ttl_s` | 1500 |

## 6. Data

Reads signer snapshots from `data/signers/` (git-versioned). Writes token caches to Redis with TTLs.
Holds no durable state of its own — a full Redis loss costs a round of token re-minting.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| Signer rotated by platform | failure rate + bundle hash diff | **halt the platform**, page, re-extract |
| Signature computes but is rejected | 4xx with valid-looking params | check UA coherence first, then re-extract |
| Isolate timeout | 50 ms cap | one retry on a fresh isolate, then tier-2 fallback |
| Isolate OOM / panic | task boundary | recycle isolate; never crashes the worker |
| Shim value missing (signer probes an unstubbed property) | JS exception | extend the shim; fixture-test it |
| `X-IG-WWW-Claim` not propagated | rising challenge rate | header-propagation test |
| Token cache lost | cache miss | re-mint; degraded throughput only |
| Bundle fetch blocked | fetch failure | use the pinned snapshot; alert if > 7 days stale |

## 8. Performance

| Metric | Budget |
|:---|:---|
| Signature (tier 1) | ≤ 3 ms p95 |
| Bootstrap / session constants | ≤ 2 s, once per session |
| Tier-2 fallback | ≤ 400 ms |
| Throughput | ≥ 2 000 signatures/s/worker across the isolate pool |
| Memory | ≤ 200 MB (4 isolates) |

## 9. Observability

`xustive_signature_total{platform,tier,outcome}`, `xustive_signature_duration_seconds{platform}`,
`xustive_signer_version_age_days{platform}`, `xustive_signer_failure_rate{platform}`,
`xustive_signer_rotation_detected_total`, `xustive_isolate_recycled_total`,
`xustive_token_cache_hit_ratio`, `xustive_embedded_json_used_total`.

`xustive_signer_failure_rate` is a **paging** alert. It is the earliest and clearest signal that a
platform changed something, and every hour spent failing signatures is an hour of identity damage.

## 10. Security

- Signer snapshots are **third-party obfuscated JavaScript**. They are executed in a locked-down
  isolate with no network, no filesystem, a hard time cap, and a heap cap — treated as hostile code,
  because that is what it is.
- Snapshots are committed to git so what we execute is reviewable and diffable across versions. An
  unreviewed silent auto-update of executable third-party code would be a supply-chain hole.
- The isolate has no access to credentials, cookies, or the secrets file; it receives only the URL,
  body, and UA it needs.
- No user data is ever involved — this runs entirely on the collection path.

## 11. Testing

- Fixture signatures: recorded `(input, expected_output)` pairs per platform and signer version;
  assert byte-exact reproduction.
- Shim completeness: signer executes without throwing across all catalogue fingerprint profiles.
- UA coherence: the signed UA equals the transmitted UA — a mismatch test that would otherwise only
  surface as a slow ban-rate rise.
- Sandbox: a signer attempting `fetch`, file access, or an infinite loop is contained and times out.
- Rotation drill: swap in a deliberately wrong signer; assert the halt fires at the threshold and
  the platform stops rather than retrying.
- Isolate hygiene: 10 000 sequential signatures show no memory growth after recycling.
- Embedded-JSON path tested independently, so tier 3 still works when tiers 1–2 are broken.

## 12. Open Questions

- [ ] `deno_core` vs raw `rusty_v8` vs `boa` — Boa is pure Rust but may not run heavily obfuscated
      bundles; V8-based options are heavier but far more likely to just work. Prototype before
      committing.
- [ ] How much of extraction can be safely automated? Current stance: propose automatically, confirm
      manually the first time a bundle's shape changes.
- [ ] Do mobile-app API paths (different signing schemes entirely) belong here or in a sibling
      component? Leaning: here, as a separate signer family per platform.
- [ ] Should tier 3 (embedded JSON) be the *default* wherever it carries enough content, given it is
      the most stable path? Probably yes for profile-level collection.

## Related

[[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] · [[Fingerprint Engine]] ·
[[Social Connector - TikTok]] · [[Social Connector - Instagram]] · [[Social Connector - Facebook]] ·
[[Observability]]
