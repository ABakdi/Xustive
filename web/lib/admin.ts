/**
 * Typed client for the operator API (`/api/v1/admin/*`).
 *
 * The admin console is part of this Next.js app now, not a set of pages the Rust process renders.
 * It reaches the Rust API through the same `/api/v1/*` rewrite as the search UI, so every call here
 * is a same-origin relative fetch — which is also what keeps the browser on one origin and the CSP
 * simple. Auth is handled server-side: the rewrite proxies from the Next server (loopback to the
 * API), which the API admits without a key in a local deployment.
 */

const BASE = '/api/v1/admin'

export class AdminError extends Error {
  constructor(readonly code: string, message: string, readonly status: number) {
    super(message)
  }
}

async function getJSON<T>(path: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { accept: 'application/json' },
    cache: 'no-store',
    signal,
  })
  if (!res.ok) {
    const body = (await res.json().catch(() => null)) as { error?: { code: string; message: string } } | null
    throw new AdminError(body?.error?.code ?? 'unknown', body?.error?.message ?? `HTTP ${res.status}`, res.status)
  }
  return res.json() as Promise<T>
}

async function postJSON<T>(path: string, payload: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', accept: 'application/json' },
    body: JSON.stringify(payload),
  })
  const body = (await res.json().catch(() => null)) as T & { error?: { code: string; message: string } }
  if (!res.ok) {
    throw new AdminError(body?.error?.code ?? 'unknown', body?.error?.message ?? `HTTP ${res.status}`, res.status)
  }
  return body
}

// --- discovery yield -------------------------------------------------------------------------
export interface ChannelYield {
  channel: string
  discovered: number
  fetched: number
  indexed: number
  duplicate: number
  yield_rate: number | null
  unique_rate: number | null
}
export const getChannels = (signal?: AbortSignal) =>
  getJSON<{ channels: ChannelYield[] }>('/crawler/channels', signal).then((d) => d.channels)

// --- source health ---------------------------------------------------------------------------
export interface SourceHealthRow {
  id: string
  display_name: string | null
  lifecycle: string | null
  trust_tier: string | null
  crawlable: boolean | null
  policy: {
    enabled: boolean
    frequency: 'realtime' | 'hourly' | 'daily' | 'weekly'
    max_docs_per_run: number
    crawl_delay_ms: number
    depth_limit: number
  } | null
  counts: { fetched: number; failed: number; indexed: number; thin: number; duplicate: number }
  quality: {
    fetch_success_rate: number | null
    extraction_success_rate: number | null
    duplicate_ratio: number | null
    spam_mean: number | null
    date_unknown_ratio: number | null
  }
}
export const getSourceHealth = (signal?: AbortSignal) =>
  getJSON<{ sources: SourceHealthRow[] }>('/crawler/sources/health', signal).then((d) => d.sources)

/** Registry lifecycle + policy edit (PROB-003). Same transitions and guards as
 *  `xustive registry approve|activate|disable`; policy floors are enforced server-side. */
export interface RegistryEdit {
  id: string
  action?: 'approve' | 'activate' | 'disable'
  reason?: string
  policy?: {
    enabled?: boolean
    frequency?: 'realtime' | 'hourly' | 'daily' | 'weekly'
    max_docs_per_run?: number
    crawl_delay_ms?: number
    depth_limit?: number
  }
}
export const editRegistry = (edit: RegistryEdit) =>
  postJSON<{ ok: boolean; changed: string[]; lifecycle: string; crawlable: boolean; note?: string }>(
    '/crawler/registry',
    edit,
  )

// --- weak coverage ---------------------------------------------------------------------------
export interface WeakCoverage {
  enabled: boolean
  k_anonymity: number
  /** Whether anything can actually resolve weak terms to URLs (PROB-003). */
  resolution?: { serp_enabled: boolean; brave_usable: boolean }
  terms: { term: string; count: number }[]
  /** Panel-shaped queries that resolved to no entity (M8-T09). A different kind of gap: this one
   *  wants a harvest, not a crawl source. */
  entities?: { term: string; count: number }[]
}
export const getWeakCoverage = (signal?: AbortSignal) =>
  getJSON<WeakCoverage>('/crawler/weak-coverage', signal)
/** Dismiss one weak term. If the gap is real it re-accumulates past the k-floor on its own. */
export const forgetWeakTerm = (term: string) =>
  postJSON<{ ok?: boolean }>('/crawler/weak-coverage/forget', { term })

// --- documents -------------------------------------------------------------------------------
export interface DocHit {
  id: string
  title?: string
  url: string
  domain?: string
  language?: string
  body_len?: number
  excerpt?: string
  published_at?: number
  /** Discovery channel that found this document — its provenance (M7). */
  discovery?: string
}
export interface DocumentsPage {
  hits: DocHit[]
  estimated_total: number
  page: number
  per_page: number
  /** Index composition by discovery channel within the current scope: `{ federation: 12, seed: 340 }`. */
  composition: Record<string, number>
}
export function getDocuments(
  params: { q?: string; host?: string; lang?: string; channel?: string; page?: number },
  signal?: AbortSignal,
) {
  const p = new URLSearchParams()
  if (params.q) p.set('q', params.q)
  if (params.host) p.set('host', params.host)
  if (params.lang) p.set('lang', params.lang)
  if (params.channel) p.set('channel', params.channel)
  p.set('page', String(params.page ?? 1))
  return getJSON<DocumentsPage>(`/crawler/documents?${p}`, signal)
}

/** Discovery channels grouped for the index-composition view. `federation` is what a user search
 *  pulled from SearXNG and we indexed; everything else the crawler found on its own (seeds, followed
 *  links, sitemaps, Common Crawl, and query-driven SERP/Brave discovery). */
export const SEARX_CHANNELS = ['federation'] as const
export const DISCOVERED_CHANNELS = ['seed', 'link', 'sitemap', 'cc', 'serp', 'brave', 'query'] as const

// --- sources (seed list) ---------------------------------------------------------------------
export interface Seed {
  source_id: string
  url: string
  trust: string
  category: string
  region: string
  note: string
}
/** The catalog categories, in display order. Keep in sync with CATEGORIES in admin_crawler.rs. */
export const CATEGORIES = [
  'news',
  'government',
  'education',
  'health',
  'science-tech',
  'sport',
  'culture',
  'business',
  'reference',
] as const
export const getSources = (signal?: AbortSignal) =>
  getJSON<{ seeds: Seed[] }>('/crawler/sources', signal).then((d) => d.seeds)
export const addSource = (url: string, trust: string, category?: string) =>
  postJSON<{ ok?: boolean; already_listed?: boolean; source_id?: string; queued?: boolean }>(
    '/crawler/sources',
    { url, trust, category },
  )
export const removeSource = (url: string) =>
  postJSON<{ ok?: boolean; removed?: number }>('/crawler/sources/remove', { url })

// --- live snapshot ---------------------------------------------------------------------------
export interface Snapshot {
  state: string
  /** Operator pause (PROB-003): held deliberately, distinct from idle or broken. */
  paused?: boolean
  fetched: number
  revisited: number
  parsed: number
  indexed: number
  discovered: number
  failed: number
  skipped: Record<string, number>
  recent: { url: string; host: string; outcome: string; at: number; words: number }[]
  hosts: Record<string, number>
  waiting: number
  inflight: number
  deferred: number
  unavailable: boolean
}
export const getStatus = (signal?: AbortSignal) => getJSON<Snapshot>('/crawler/status', signal)
/** Hold or release the crawl. Takes effect within seconds; in-flight fetches finish. */
export const setCrawlPaused = (paused: boolean) =>
  postJSON<{ ok?: boolean; paused?: boolean }>('/crawler/pause', { paused })

// --- force-crawl (enqueue a URL) -------------------------------------------------------------
/** Push a URL into the frontier now — as trusted as a seed. `front` jumps it to the head.
 *  The response distinguishes "queued fresh" (`added`) from "already known" (`already_known`) —
 *  the client type used to invent a `queued` field the API never sent (PROB-003). */
export const enqueueUrl = (url: string, front: boolean) =>
  postJSON<{ ok?: boolean; added?: boolean; already_known?: boolean; url?: string; error?: { message?: string } }>(
    '/crawler/enqueue',
    { url, front },
  )

// --- configuration (read-only, secrets redacted) ---------------------------------------------
export const getConfig = (signal?: AbortSignal) =>
  getJSON<{ config: Record<string, unknown> }>('/config', signal)

// --- compute ---------------------------------------------------------------------------------
export const getCompute = (signal?: AbortSignal) => getJSON<Record<string, unknown>>('/status', signal)
export const setDevice = (preference: string, gpuLayers: number | null) =>
  postJSON<Record<string, unknown>>('/device', { preference, gpu_layers: gpuLayers })
export const setPoliteness = (ignore: boolean) =>
  postJSON<Record<string, unknown>>('/politeness', { ignore_politeness: ignore })
/** Set a temporary tracing filter (auto-reverts). `null` reverts to the baseline now. */
export const setLogLevel = (filter: string | null) =>
  postJSON<{ filter?: string; baseline?: string; expires_in?: number | null }>('/log-level', {
    filter,
  })

// --- integrations (external tools) -----------------------------------------------------------
export interface FederationStatus {
  enabled: boolean
  configured: boolean
  searxng_url: string
  federator_url: string
  budget_ms: number
  max_hits: number
  /** When true, federated results are indexed immediately as thin docs (title+snippet), then the
   *  full crawl overwrites them. When false, only the crawl-feed runs (slower to appear). */
  eager_index: boolean
  /** Live health probe of the gateway (on the core network — not internet egress). */
  reachable_from_api: boolean
  /** The gateway's circuit-breaker state: closed | open | half-open | none. */
  breaker: string
}
/** Semantic (dense) text search status (M7-T02). */
export interface SemanticStatus {
  enabled: boolean
  configured: boolean
  embedder_endpoint?: string
  collection?: string
  dim?: number
  reachable?: boolean
  breaker?: string
  /** Documents embedded into the text vector collection, or null if Qdrant is unreachable. */
  documents_embedded?: number | null
}
/** Image-similarity (CLIP) vector status (M3). */
export interface ImageVectorStatus {
  enabled: boolean
  configured: boolean
  embedder_endpoint?: string
  collection?: string
  reachable?: boolean
  images_embedded?: number | null
}
/** Effectiveness counters, read from the metrics registry (M7). */
export interface IntegrationEffectiveness {
  federation_searches_hits: number
  federation_searches_empty: number
  federation_urls_fed: number
  /** Cards served on federation-armed first pages, by source. The web share falling over time is
   *  the convergence measure — the index catching up with what people search for (M7-T09.2). */
  blend_cards_web: number
  blend_cards_local: number
  semantic_fused_recall: number
  semantic_fused_reinforce: number
}
/** The external AI summariser (M7-T08): third-party SaaS behind the Federation Gateway. */
export interface ExternalSummariserStatus {
  enabled: boolean
  /** A gateway client exists; whether the gateway holds an LLM endpoint is its own deployment env. */
  configured: boolean
  /** Always true — the flag the console uses to show the "sends data off-box" warning. */
  third_party: boolean
  attempts_ok: number
  attempts_failed: number
}
export const getIntegrations = (signal?: AbortSignal) =>
  getJSON<{
    federation: FederationStatus
    semantic: SemanticStatus
    image: ImageVectorStatus
    external_summariser: ExternalSummariserStatus
    effectiveness: IntegrationEffectiveness
  }>('/integrations', signal)
/** Toggle one integration on or off at runtime: `federation` or `external_summariser`. */
export const setIntegration = (integration: string, enabled: boolean) =>
  postJSON<{ ok: boolean; integration: string; enabled: boolean }>('/integrations', {
    integration,
    enabled,
  })

// --- image AI (OCR + vector) -----------------------------------------------------------------
export interface MediaStatus {
  ocr: { backend: string; healthy: boolean; sidecar_endpoint: string }
  vector:
    | { enabled: false }
    | {
        enabled: true
        embedder_healthy: boolean
        qdrant_reachable: boolean
        image_vectors: number | null
        embedder_endpoint: string
        collection: string
      }
  stt:
    | { enabled: false }
    | { enabled: true; healthy: boolean; breaker: string; endpoint: string }
}
export const getMedia = (signal?: AbortSignal) => getJSON<MediaStatus>('/media', signal)

// --- interaction analytics (M6) --------------------------------------------------------------
export interface InteractionStatus {
  enabled: boolean
  k_anonymity?: number
  window_days?: number
  /** Clicks before a doc becomes a re-crawl candidate — used server-side, now visible (PROB-003). */
  hot_floor?: number
  top_queries?: {
    query: string
    count: number
    category: string
    result_count?: number
    clicks?: number
  }[]
  categories?: Record<string, number>
  ctr_leaders?: {
    doc: string
    impressions: number
    clicks: number
    ctr: number
    title: string
    url: string
  }[]
  hot_docs?: { doc: string; title: string; url: string }[]
}
export const getInteraction = (signal?: AbortSignal) =>
  getJSON<InteractionStatus>('/interaction', signal)

// --- index queue & dead letters --------------------------------------------------------------
export interface QueueStatus {
  unavailable: boolean
  backlog?: number
  dead_count?: number
  dead?: { entry_id?: string; url: string; attempts: number; reason: string; failed_at: number }[]
  /** The capacity alarm (PROB-001): Redis memory vs its ceiling + frontier size. Null when the
   *  frontier store is unreachable; pct null when Redis runs uncapped. */
  capacity?: {
    redis_used_bytes: number | null
    redis_max_bytes: number | null
    redis_pct: number | null
    frontier_waiting: number
    frontier_deferred: number
  } | null
}
export const getQueue = (signal?: AbortSignal) => getJSON<QueueStatus>('/queue', signal)

// --- evaluation (PROB-003) -------------------------------------------------------------------
/** One report file under eval/reports/, summarised. Absent scores mean the file does not carry
 *  them (an ab-/serp-/calibration- report), never zero. */
export interface EvalReport {
  file: string
  kind: 'eval' | 'baseline' | 'ab' | 'serp' | 'calibration'
  generated_at?: string | null
  queries?: number | null
  ndcg_at_10?: number
  mrr_at_10?: number
  recall_at_50?: number
  zero_result_rate?: number
  by_language?: Record<string, number>
  variants?: { name: string; why?: string; ndcg_at_10?: number; mrr_at_10?: number }[]
  ranked?: unknown[]
  engine?: string
}
export interface EvalStatus {
  reports: EvalReport[]
  unreadable: string[]
  /** Latest dated eval vs baseline.json, same relative tolerance as `xustive eval --baseline`. */
  gate: {
    baseline_ndcg: number
    latest_ndcg: number
    latest_file: string
    delta: number
    tolerance_pct: number
    pass: boolean
  } | null
  candidates: { file: string; rows: number }[]
}
export const getEval = (signal?: AbortSignal) => getJSON<EvalStatus>('/eval', signal)
export const replayDlq = () => postJSON<{ replayed?: number }>('/queue/replay', {})
/** Put one dead letter back on the queue. found:false means it was already gone. */
export const replayDeadOne = (entry_id: string) =>
  postJSON<{ ok: boolean; found: boolean }>('/queue/dead/replay', { entry_id })
/** Discard one dead letter for good — the only deliberate discard in the queue. */
export const dropDeadOne = (entry_id: string) =>
  postJSON<{ ok: boolean; found: boolean }>('/queue/dead/drop', { entry_id })

// --- maintenance (takedown) ------------------------------------------------------------------
export interface TakedownResult {
  domain: string
  matched?: number
  executed: boolean
  documents_removed?: number
  vector_groups_removed?: number
  raw_bodies_removed?: number
  note?: string
}
/** Preview (execute:false) or run a domain takedown. `confirm` must equal `domain` to execute. */
export const takedown = (domain: string, execute: boolean, confirm: string) =>
  postJSON<TakedownResult>('/takedown', { domain, execute, confirm })
