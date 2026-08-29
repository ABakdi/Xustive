---
tags:
  - component
  - community
  - crawling
  - ml
status: specified
date: 2026-08-29
milestone: 14
---
# Community Node

> Component C26 · Milestone: [[Milestone 14 - One Server, Many Hands]] · Decisions:
> [[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]],
> [[ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query]] · Peer:
> [[Contribution Coordinator]]

## 0. What it is

`xustive-node` — one binary, installed by one command, that lends a volunteer's machine to the
engine. It is the same crawl pipeline the server runs (`xustive-ingest`), the same media and
model clients, linked into a small agent that leases work and returns evidence. Nothing about it
is a plugin system: the volunteer installs a signed binary knowingly, and the server never sends
it code ([[ADR-0034]] §5).

```bash
curl -fsSL https://get.xustive.dz | sh          # verified download, user-level service
xustive-node join --token INVITE --crawl        # crawl for the engine
xustive-node join --token INVITE --gpu          # lend the GPU to batch work
xustive-node join --token INVITE --crawl --gpu  # both
xustive-node status | pause | resume | leave
```

## 1. The two capabilities

| Mode | What the machine does | What leaves the machine |
|:---|:---|:---|
| `--crawl` | leases a host, fetches its pages at the leased delay, extracts and enriches locally | the evidence of §3 — text, hashes, status, links, media URLs |
| `--gpu` | leases a batch job, runs a digest-pinned model over content the crawler already fetched | vectors, labels, OCR text, transcripts, summaries |

Either alone is useful. A laptop on a domestic line is a good crawler and a poor GPU; a desktop
with a card and a data cap is the reverse.

## 2. Crawl loop

```
lease(host, urls, delay, robots, deadline)
  └─ for url in urls:
       respect delay and robots (the lease's copy, and the site's own if it changed)
       fetch with the shared client (IPv4, timeouts, size caps, redirect rules)
       parse → extract → enrich locally (language, text, links, media)
       accumulate evidence; renew the lease while work remains
  └─ submit(batch, signed)   → accepted / quarantined / rejected
```

The pipeline is the server's, so a page crawled by a volunteer and a page crawled by the operator
differ in exactly one thing: who fetched it. Enrichment runs locally because it is the expensive
half and because the server recomputes what it needs anyway — running it here saves the server
CPU without the server having to trust the result.

**Politeness is not the node's decision.** The delay comes with the lease; the node may go slower
(it does, on battery) but never faster, and it holds exactly one host at a time.

## 3. What it sends, and what it keeps

Sent: the fields listed in [[Contribution Coordinator]] §3 — URL, status, timing, header digest,
content hash, extracted text, title, links, media URLs, and the robots decision it applied.

Never sent: anything about the volunteer. No path names, no machine name, no browsing history, no
local files, no environment. The node reads nothing on the machine except its own config
directory. It is a crawler with a job queue, not an agent that looks around.

Kept locally: the config, the Ed25519 private key (generated on the machine, never transmitted),
a small resumable work file, and a rotating log the volunteer can read.

## 4. GPU loop

Capability probe at start — VRAM, CUDA or CPU, which models are present — and again after any
failure. A job names a model and its SHA-256; the node downloads it once into its cache, verifies
the digest, and refuses the job on a mismatch. One job at a time by default, a VRAM ceiling the
volunteer sets, and a hard timeout per item so a wedged model costs one job rather than the
evening.

Where the model has a CPU path (CLIP, faster-whisper small, the ONNX reranker), a machine without
a card can still take those jobs at its own pace. That is the same choice the operator's own
sidecars make ([[Image Pipeline]], [[Speech to Text]]).

## 5. Manners on someone else's computer

| Rule | Default |
|:---|:---|
| Bandwidth ceiling | 2 Mbit/s, configurable; measured, not assumed |
| Concurrency | one host, one GPU job |
| Battery | pause below 40 % on battery, resume on mains |
| Metered connection | pause (Windows/macOS report it; Linux via NetworkManager) |
| Process priority | `nice`/`ionice` at the bottom |
| Disk | a bounded cache; models are the only large files, and `leave` removes them |
| Visibility | `xustive-node status` shows exactly what it did today, and the log is plain text |
| Leaving | `xustive-node leave` revokes the credential, stops the service, deletes key, cache and models |

The one honest cost, stated at install time and in [[Running a Community Node]]: **the sites the
node crawls see the volunteer's IP address**, exactly as if they had visited. Nothing else about
them is exposed, and no reader's query ever reaches their machine.

## 6. Failure and offline behaviour

A lost connection is normal, not exceptional: work in progress is written to the resume file,
submission is retried with backoff, and a lease that expires is simply lost — the server hands
the host to someone else. The node never queues unbounded work: if it cannot submit for an hour,
it stops fetching rather than filling a disk.

A rejected batch is reported in `status` with the verifier's reason, because the most likely
cause of a rejection is a bug in a released node, and the volunteer is the one who can see it
first.

## 7. Distribution

One static binary per platform (Linux x86-64/aarch64, macOS arm64, Windows x86-64), signed and
published with its SHA-256; the installer verifies the signature before it runs anything. The
service is user-level (systemd `--user`, launchd, or a Windows service) so joining never needs
root. Auto-update is opt-in and always verifies the same signature; a node too old for the
protocol is told to update and stops rather than sending malformed work.

## 8. Configuration

```toml
# ~/.config/xustive/node.toml
node_id      = "…"            # assigned at enrolment
server       = "https://xustive.dz"
capabilities = ["crawl", "gpu"]
[limits]
bandwidth_kbps = 2000
gpu_vram_mb    = 4096
pause_on_battery = true
pause_on_metered = true
[cache]
dir = "~/.cache/xustive"      # models and resume state
max_gb = 20
```

## 9. Test plan

A node harness that runs against a coordinator in a test container: joins, leases, crawls a local
fixture site, submits, and is admitted. Adversarial variants live on the server side
([[Contribution Coordinator]] §11) but are driven from here: a node that alters text, one that
ignores the delay (the server sees the timing evidence), one that submits outside its lease, one
that reports a GPU it does not have. Platform smoke tests for the installer on each target.

## 10. Open questions

- A browser extension as a second, lower-trust front door — far lower friction, far less
  throughput.
- Should a volunteer be able to say "crawl my wilaya", and what stops that from being a way to
  steer the corpus?
- Model cache sharing between volunteers (peer-to-peer) versus every node downloading weights
  from the operator's bandwidth.

## Related

[[Contribution Coordinator]] · [[Running a Community Node]] · [[Web Fetcher]] ·
[[Enrichment Pipeline]] · [[Politeness and Robots]] · [[Vector Index]]
