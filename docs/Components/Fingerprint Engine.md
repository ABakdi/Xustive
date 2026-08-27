---
tags:
  - component
  - ingestion
  - collection
component-id: C26
binary: xustive-cli crawld (library only)
status: schema, catalogue and coherence checker built; impersonation not built
updated: 2026-08-27
---

# Fingerprint Engine

> **ID** C26 · **Module** `crates/xustive-ingest/src/fingerprint/` (`mod.rs` schema and
> `Catalogue`, `coherence.rs` checker) · **Catalogue** `data/fingerprints/*.toml` · **Upstream**
> [[Session Manager]], [[Web Fetcher]] · **Downstream** HTTP client, headless browser

## 0. What exists today (2026-08-27)

The **schema and the coherence checker**: `Profile` (parsed from TOML), `Catalogue::load_dir`,
`check(&Profile) -> Vec<Incoherence>`, version ageing (`is_retired`, `can_migrate_to`), and a
catalogue test that runs `check` over every shipped profile. Four profiles ship:
`chrome-131-win11-dz`, `chrome-131-android-dz`, `firefox-133-win11-dz`, `safari-18-ios-dz`, each
marked "generated as a coherent unit, verify against a real capture before trusting in
production". **Not built**: the impersonating HTTP client (§4.1 TLS and HTTP/2 layers — no
`rquest` or equivalent in the workspace), headless CDP patching (§4.6), the distribution file and
assignment (§4.4), echo-service self-validation (§4.7). The `Fingerprints` trait in §3 is the
intended shape; nothing takes a `client()` today.

## 1. Purpose

Make our requests indistinguishable from a real browser at every layer that platforms inspect.
Introduced by [[ADR-0009 - Direct Collection for Social Platforms]].

The critical insight: **detection is about coherence, not any single value.** A request advertising
Chrome 131 on Windows in its `User-Agent` while presenting a Rust `rustls` TLS handshake, Go-style
HTTP/2 settings, and alphabetically-sorted headers is trivially identifiable — not because any one
signal is wrong, but because no real browser produces that combination.

## 2. Responsibilities

**In scope**: coherent fingerprint profiles spanning TLS, HTTP/2, header order, Client Hints, and JS
surface; profile catalogue and versioning; profile assignment and pinning; headless browser patching;
self-validation against fingerprint echo services.

**Out of scope**: identity/account state (→ [[Session Manager]]); IP reputation (→ [[Proxy Manager]]);
request signing (→ [[Signature Service]]).

## 3. Interface

```rust
pub trait Fingerprints: Send + Sync {
    fn profile(&self, id: FingerprintId) -> &Profile;
    fn assign(&self, platform: Platform, geo: &str) -> FingerprintId;   // at identity creation only
    fn client(&self, id: FingerprintId) -> Result<HttpClient, FpError>; // pre-configured
    fn browser_patches(&self, id: FingerprintId) -> BrowserPatchScript; // CDP init script
}

pub struct Profile {
    pub id: FingerprintId,
    pub browser: BrowserKind,        // Chrome | Firefox | Safari
    pub version: String,             // "131.0.6778.86"
    pub platform_os: OsKind,         // Windows11 | MacOS | Android14 | iOS17
    pub tls: TlsProfile,             // cipher order, extensions, ALPN, GREASE, curves, sig algs
    pub http2: Http2Profile,         // SETTINGS order+values, WINDOW_UPDATE, priority, pseudo-header order
    pub headers: HeaderProfile,      // exact order and casing, Client Hints
    pub js_surface: JsSurface,       // navigator, screen, WebGL, canvas, fonts, timezone
}
```

`assign` is called **once**, at identity creation, and the result is pinned for life
([[Session Manager]] §4.2).

## 4. Internal Design

### 4.1 The four layers

| Layer | What is inspected | Our approach |
|:---|:---|:---|
| **TLS** | JA3 / JA4 hash: cipher suite order, extension order, supported groups, signature algorithms, ALPN, GREASE placement | `rquest` (a `reqwest` fork with browser-accurate ClientHello) — the default `reqwest`/`rustls` handshake is instantly identifiable |
| **HTTP/2** | SETTINGS frame field order and values, initial WINDOW_UPDATE, stream priority, pseudo-header order (`:method :authority :scheme :path` for Chrome vs Firefox ordering) | profile-driven, from the same library |
| **HTTP headers** | exact order, exact casing, presence and order of `sec-ch-ua*`, `sec-fetch-*`, `accept-language`, `accept-encoding` | ordered `HeaderMap`, never a sorted map |
| **JS surface** (headless only) | `navigator.webdriver`, plugins, languages, hardwareConcurrency, deviceMemory, screen metrics, WebGL vendor/renderer, canvas hash, AudioContext, font list, timezone, WebRTC local IPs | CDP `Page.addScriptToEvaluateOnNewDocument` patches applied before any page script runs |

### 4.2 Coherence rules

A profile is generated as a **unit**; individual fields are never mixed. Enforced invariants:

- `User-Agent` version == `sec-ch-ua` version == TLS profile version == JS `navigator.userAgent`.
- OS in the UA matches `sec-ch-ua-platform`, the font list, `navigator.platform`, and screen metrics.
- Safari profiles never emit `sec-ch-ua` (Chromium-only headers).
- `accept-language` matches the JS `navigator.languages` array **and** the proxy's geography — an
  Algiers residential IP presenting `en-US` only is a mismatch; `fr-FR,fr,ar,en` is plausible here.
- Timezone from the JS surface matches the proxy geo (`Africa/Algiers`, UTC+1).
- WebGL vendor/renderer pair must be one that actually ships together (e.g. `Google Inc. (NVIDIA)` /
  `ANGLE (NVIDIA GeForce RTX 3060 …)`), drawn from a real-hardware table.
- WebRTC is disabled or forced through the proxy — a leaked local IP defeats everything above it.

`tests/fingerprint_catalogue.rs` asserts every profile in the catalogue satisfies all of these.
Incoherence is the bug class that matters here, so it is checked mechanically rather than by
review. The checker's verdicts are `Incoherence::{Version, Os, ClientHints, Language, Timezone,
Webgl}`; `WebRtc` has no `Direct` variant at all, so a leaking profile cannot be expressed.

### 4.3 Catalogue

`data/fingerprints/*.toml`, one file per profile, versioned in git. The built schema is **flat**
— the TLS and HTTP/2 sections of the earlier draft wait on the client library that would consume
them:

```toml
id = "chrome-131-win11-dz"
browser = "Chrome"            # Chrome | Firefox | Safari
version = "131.0.6778.86"
os = "Windows11"              # Windows11 | MacOS | Android | IOS
geo = "DZ"                    # language and timezone must agree with it
user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) … Chrome/131.0.0.0 Safari/537.36"
sec_ch_ua = "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\""
sec_ch_ua_platform = "Windows"
header_order = ["sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform", …, "accept-language"]
accept_language = "fr-FR,fr;q=0.9,ar;q=0.8,en-US;q=0.7,en;q=0.6"
navigator_platform = "Win32"
navigator_languages = ["fr-FR", "fr", "ar", "en-US", "en"]
timezone = "Africa/Algiers"
webgl_vendor = "Google Inc. (NVIDIA)"
webgl_renderer = "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 … D3D11)"
webrtc = "disabled"           # disabled | through_proxy — there is no direct
introduced_at = 0             # unix seconds; 0 = unset
retire_after = 0
```

A file that fails to parse is a hard error at load: a malformed fingerprint is one that would be
assigned and then flagged, worse than absent.

Target catalogue: **12–20 profiles** — Chrome/Firefox on Windows and macOS, Chrome on Android,
Safari on iOS. Four exist. Enough diversity that identities are not clones; few enough that each
is maintained and validated.

### 4.4 Distribution realism — not built (2026-08-27)

Profiles should be assigned to match the real Algerian browser mix, not uniformly. A pool where
20 % of identities run Firefox-on-macOS does not resemble the local population. Weights would live
in `data/fingerprints/distribution.toml`, reviewed against public market-share data; no such file
exists and nothing assigns profiles yet.

### 4.5 Version ageing

Real browsers auto-update; a fleet pinned to Chrome 131 for a year becomes anomalous. Profiles carry
`introduced_at` and `retire_after`. When a profile retires, its identities are migrated to the
successor version of the **same browser and OS** — this is the one sanctioned exception to §4.2
pinning, because it mirrors what a real browser does. Bumping Chrome 131→133 on a Windows identity is
normal; switching that identity to Safari is not. Built: `Profile::is_retired(now)` and
`can_migrate_to(successor)` (same browser and OS, newer version); nothing runs the migration yet.

### 4.6 Headless patching — not built (2026-08-27)

For the [[Web Fetcher]] headless path, itself not built: real Chrome (not
Chromium-headless-shell), `--headless=new`,
persistent profile directory per identity, and a CDP init script that patches the `js_surface` values
before page scripts run. `navigator.webdriver` removal, plugin/mimeType stubs, permissions query
shim, and canvas/AudioContext noise seeded **deterministically per identity** — a canvas hash that
changes every request is itself a signal.

### 4.7 Self-validation — not built (2026-08-27)

`make fp-verify` would drive each profile through public fingerprint echo endpoints in staging
and assert
the observed JA3/JA4, HTTP/2 fingerprint, and header order match the profile's declared values. Run
nightly and on every catalogue change. This catches library upgrades silently altering our
handshake — which happens, and is otherwise invisible until ban rates climb.

## 5. Configuration

Nothing in `config/*.toml`; `Catalogue::load_dir` takes the directory as an argument and the
catalogue test points it at `data/fingerprints/`. The rest of the table is the design target.

| Key | Default |
|:---|:---|
| `catalogue_dir` | `data/fingerprints/` |
| `distribution_path` | `data/fingerprints/distribution.toml` |
| `profile_retire_days` | 120 |
| `verify_interval` | nightly |
| `headless_real_chrome` | `true` |
| `webrtc_policy` | `disable_non_proxied_udp` |
| `canvas_noise_seed` | per identity, deterministic |

## 6. Data

Reads the profile catalogue and distribution weights. Holds no state — profile *assignment* is stored
on the identity by [[Session Manager]]. Pure function of `(catalogue, identity)`.

## 7. Failure Modes

| Failure | Detection | Response |
|:---|:---|:---|
| HTTP library upgrade changes our TLS fingerprint | nightly `fp-verify` | **block the release**; pin the dependency |
| Profile incoherence introduced by an edit | CI coherence test | build failure |
| Profile version aged out | `retire_after` | migrate to successor version, log |
| Echo service unavailable | verify job | warn, retry; do not block collection |
| Headless patch broken by a Chrome update | verify job + ban-rate rise | pin the Chrome version, fix patches |
| WebRTC leak | verify job checks observed IP == proxy IP | **quarantine the profile immediately** — a leak exposes the real host |
| Catalogue file malformed | startup | **Fatal** |

## 8. Performance

| Metric | Budget |
|:---|:---|
| `profile()` lookup | ≤ 10 µs (in-memory) |
| Client construction | ≤ 5 ms, pooled and reused per identity |
| Headless patch injection | ≤ 20 ms per page |
| Memory | ≤ 50 MB catalogue |

Clients are built once per identity and reused. Constructing a TLS client per request would both cost
latency and produce a session-resumption pattern no browser exhibits.

## 9. Observability

`xustive_fp_profile_assigned_total{profile}`, `xustive_fp_verify_status{profile,layer}`,
`xustive_fp_drift_total{layer}` (observed ≠ declared — the leading indicator of a silent library
change), `xustive_fp_retired_total`, `xustive_fp_webrtc_leak_total` (must be **0**).

## 10. Security

The catalogue is git-reviewed data describing our own client behaviour; it contains no secrets. The
genuine security concern is the **WebRTC leak path**: a leak reveals the crawler host's real address,
which both defeats collection and exposes infrastructure. It is forced off and verified, and a single
leak quarantines the profile.

The headless browser remains the highest-risk surface in the system — it executes untrusted JS. It
keeps its sandbox: own container, no access to the `core` network, read-only filesystem, dropped
capabilities, seccomp ([[Web Fetcher]] §10). Fingerprint patching does not relax any of that.

## 11. Testing

Built: the coherence suite (`tests/fingerprint_catalogue.rs` plus unit tests in `coherence.rs`
for each invariant, e.g. a Chromium profile missing `sec-ch-ua` is incoherent) and the ageing
rules. Everything from echo verification down needs the client library or a browser.

- **Coherence suite**: every profile checked against all §4.2 invariants.
- **Echo verification**: JA3/JA4, HTTP/2 fingerprint, header order match declarations (staging).
- Header order test: assert the wire order equals the profile order — a `HashMap` anywhere in the
  path silently sorts headers and breaks this.
- WebRTC: assert the observed public IP equals the proxy IP.
- Determinism: the same identity yields the same canvas/audio hash across runs; different identities
  differ.
- Regression: pin the HTTP client version; a dependency bump that changes the fingerprint fails CI.
- Distribution: sample 1 000 assignments, assert they match the configured weights within tolerance.

## 12. Open Questions

- [ ] Which impersonation library — `rquest`, `curl-impersonate` via FFI, or a hand-rolled `rustls`
      ClientHello? `rquest` is the current pick for being pure Rust and actively tracking browser
      versions; validate its JA4 accuracy before committing.
- [ ] Is 12–20 profiles the right catalogue size? Too few looks like a fleet; too many is unmaintained.
- [ ] Do mobile-app API paths need full device emulation (device id, install id, app version signing)
      as a separate profile family? Likely yes for Instagram and TikTok mobile endpoints.
- [ ] How do we source realistic WebGL vendor/renderer pairs without collecting them from real users?

## Related

[[ADR-0013 - Direct SERP Collection for Discovery]] ·
[[ADR-0009 - Direct Collection for Social Platforms]] · [[Session Manager]] · [[Proxy Manager]] ·
[[Signature Service]] · [[Web Fetcher]] · [[Security and Privacy]]
