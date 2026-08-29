---
tags:
  - adr
  - crawling
  - community
  - security
status: accepted
date: 2026-08-29
---
# ADR-0033 - Volunteer Crawling, Verified Before It Is Believed

> Part of [[Decision Log]] · Milestone: [[Milestone 14 - One Server, Many Hands]] ·
> Components: [[Contribution Coordinator]], [[Community Node]], [[Crawler Orchestrator]]

## Context

Crawl throughput is bounded by politeness and by addresses, not by code
([[PROB-002 - Crawl and Index Throughput]]): one server crawls one host at a time and sleeps
between requests. Volunteers change the arithmetic — a hundred machines are a hundred hosts in
flight — and they change *whose* web gets crawled, because a volunteer in Béchar reaches the
sites a rented European server never hears about.

They also invite the oldest problem in volunteer computing: some of the returned work will be
wrong, and some of it will be wrong on purpose. [Mwmbl](https://blog.mwmbl.org/) runs a
volunteer crawler with a central index and moved from vetting each volunteer to letting anyone
generate a key — trust at the door does not scale. The volunteer-computing literature
(BOINC and the sabotage-tolerance work that preceded it) settled long ago on the alternative:
[redundancy, spot-checking and credibility](https://arxiv.org/pdf/1903.01699) — verify a sample
rather than everything, size the sample by the contributor's history, and never let a single
contributor's output stand alone where it matters.

A search index is a *particularly* attractive thing to poison: a page that ranks is a page that
is read. And our ranking now reads endorsement, authority, quality and spam scores — numbers a
malicious node would love to supply.

## Decision

1. **A node supplies evidence, never conclusions.** What it may send: the URL it was leased, the
   HTTP status, a digest of the response headers, the fetch time, the extracted text, the content
   hash, and the media it found. Every ranking-visible field — language, simhash, quality, spam,
   topics, wilaya, authority, endorsement — is **recomputed on the server** from that text. A
   node cannot score its own page.
2. **Work is leased, exclusively, per host.** The coordinator hands one host to one node at a
   time with the delay the frontier has already learned, and takes it back when the lease ends.
   Global politeness is a property of the coordinator, not of the volunteer's good behaviour.
3. **Everything lands in quarantine.** Admission to the index requires: structural checks
   (schema, the URL was leased, robots reproduced, hashes agree, caps respected), server-side
   recomputation, and the node being above a standing threshold.
4. **Verification is sampled, and the sample is earned.** A re-fetch of a random slice compares
   content and simhash; canary URLs whose current content the server already holds are mixed into
   leases; a slice of URLs is leased twice to independent nodes and disagreement escalates to a
   server fetch. The sampling rate falls as standing rises and snaps back to 100 % on any failure
   — credibility-based fault tolerance, not blanket replication, which is what makes this
   affordable.
5. **Blast radius is capped even for trusted nodes.** Per-node daily quotas, a ceiling on the
   share of any one host's pages one node may supply, and probation (quarantine-only, full
   sampling, small quota) for every newly enrolled node.
6. **Enrolment is cheap but not free.** An invite token from the operator or from a contributor
   in good standing, one Ed25519 keypair per install generated on the volunteer's machine,
   quotas per node and per /24.

## Consequences

- Crawl capacity becomes a function of how many people care, which is the point of the project.
- The server pays a re-fetch on a few percent of pages and a full enrichment run on all of them.
  Enrichment was always paid; the re-fetch is the new cost, and it is bounded by the sampling
  rate.
- A determined attacker with many machines can still get *some* text into the index for pages
  they were leased — bounded by quotas, host share and the fact that they cannot influence
  ranking directly. Detection lands on the console's quarantine review, and revocation is one
  key.
- Volunteers expose their IP to the sites they crawl. That must be said plainly in
  [[Running a Community Node]]; it is the one real cost to the volunteer.
- The frontier's per-host state becomes a shared, leased resource — a coordination point that did
  not exist when one process owned it.

## Alternatives considered

- **Vet every volunteer.** Mwmbl started there and left; it scales with the operator's time.
- **Replicate every URL to three nodes.** Triples the cost of the whole crawl to catch a rare
  event; the literature's answer to exactly this is spot-checking with credibility.
- **Accept everything and rely on ranking.** Spam scoring is a filter, not a defence: it does not
  notice a truthful-looking page whose text was quietly altered.
- **A browser extension instead of a binary** (Mwmbl's other path). Lower friction, far less
  throughput, no GPU story; worth revisiting as a second front door.

## Related

[[ADR-0034 - Volunteer GPUs Do Batch Work, Never a Reader's Query]] ·
[[Contribution Coordinator]] · [[Politeness and Robots]] · [[Deduplication Service]]
