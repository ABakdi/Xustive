---
tags:
  - operations
  - community
status: specified
date: 2026-08-29
---
# Running a Community Node

> For volunteers. What the software does, what it costs you, and how to stop.
> Specification: [[Community Node]] · Decisions: [[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]],
> [[ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query]]

## Join

```bash
curl -fsSL https://get.xustive.dz | sh
xustive-node join --token YOUR-INVITE --crawl        # crawl the web for the index
xustive-node join --token YOUR-INVITE --gpu          # lend your GPU to batch work
xustive-node join --token YOUR-INVITE --crawl --gpu  # both
```

The installer downloads one signed binary, verifies it, and installs a **user** service — no
root. `xustive-node status` shows what your machine has done today.

## What it does

**Crawling.** The server leases you one website at a time and tells you how slowly to fetch it.
Your machine downloads those pages, extracts their text, and sends the text back with the
hashes that prove what it fetched. It does not touch anything else on your computer.

**GPU.** The server sends you pages and images it has already crawled, and a named model. Your
card computes embeddings, OCR text, descriptions, transcripts or summaries, and sends the numbers
back. **No search anyone types ever reaches your machine** — that is a rule of the system, not a
setting ([[ADR-0034]]).

## What it costs you

| Cost | Detail |
|:---|:---|
| **Your IP address is visible to the sites you crawl** | exactly as if you had visited them in a browser. This is the real cost; everything else is a resource limit you set. |
| Bandwidth | capped at 2 Mbit/s by default (`limits.bandwidth_kbps`) |
| CPU / GPU | lowest priority; one job at a time; a VRAM ceiling you choose |
| Disk | a bounded cache; models are the only large files |
| Battery / mobile data | it pauses on battery below 40 % and on metered connections |

## Stop, pause, leave

```bash
xustive-node pause      # stop taking work, keep the credential
xustive-node resume
xustive-node leave      # revoke the credential, stop the service, delete key, cache and models
```

Nothing you have already contributed is removed by leaving — those pages are in the index — but
your machine keeps nothing and receives nothing further.

## When something looks wrong

`xustive-node status` prints the last batches and, for anything rejected, the reason the server
gave. A rejection is usually a bug in a released version rather than a mistake on your side:
report it with the batch id from `status`. The log is plain text under the cache directory and
contains only URLs and timings.

## What the server does with your work

It checks it. Structure first, then it recomputes every ranking number itself, then it re-fetches
a sample of your pages to compare — often at first, rarely once your node has a history. Pages
that pass go into the index; pages that fail land in a review queue and your node's standing
falls. The full rules are in [[Contribution Coordinator]] §5 and the reasoning in
[[ADR-0033 - Volunteer Crawling, Verified Before It Is Believed]] — none of it is personal: the
system is built to be safe when it does not know who you are.

## Related

[[Community Node]] · [[Contribution Coordinator]] · [[Operating Xustive]]
