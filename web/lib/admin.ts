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

// --- weak coverage ---------------------------------------------------------------------------
export interface WeakCoverage {
  enabled: boolean
  k_anonymity: number
  terms: { term: string; count: number }[]
}
export const getWeakCoverage = (signal?: AbortSignal) =>
  getJSON<WeakCoverage>('/crawler/weak-coverage', signal)

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
}
export interface DocumentsPage {
  hits: DocHit[]
  estimated_total: number
  page: number
  per_page: number
}
export function getDocuments(
  params: { q?: string; host?: string; lang?: string; page?: number },
  signal?: AbortSignal,
) {
  const p = new URLSearchParams()
  if (params.q) p.set('q', params.q)
  if (params.host) p.set('host', params.host)
  if (params.lang) p.set('lang', params.lang)
  p.set('page', String(params.page ?? 1))
  return getJSON<DocumentsPage>(`/crawler/documents?${p}`, signal)
}

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
  postJSON<{ ok?: boolean; already_listed?: boolean; source_id?: string }>('/crawler/sources', {
    url,
    trust,
    category,
  })
export const removeSource = (url: string) => postJSON<{ ok?: boolean }>('/crawler/sources/remove', { url })

// --- live snapshot ---------------------------------------------------------------------------
export interface Snapshot {
  state: string
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

// --- force-crawl (enqueue a URL) -------------------------------------------------------------
/** Push a URL into the frontier now — as trusted as a seed. `front` jumps it to the head. */
export const enqueueUrl = (url: string, front: boolean) =>
  postJSON<{ url?: string; queued?: boolean; error?: { message?: string } }>('/crawler/enqueue', {
    url,
    front,
  })

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
  allowlist: string[]
  /** Live health probe of the gateway (on the core network — not internet egress). */
  reachable_from_api: boolean
  /** The gateway's circuit-breaker state: closed | open | half-open | none. */
  breaker: string
}
export const getIntegrations = (signal?: AbortSignal) =>
  getJSON<{ federation: FederationStatus }>('/integrations', signal)
/** Toggle one integration on or off at runtime. Only `federation` today. */
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
  top_queries?: { query: string; count: number; category: string }[]
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
  dead?: { url: string; attempts: number; reason: string; failed_at: number }[]
}
export const getQueue = (signal?: AbortSignal) => getJSON<QueueStatus>('/queue', signal)
export const replayDlq = () => postJSON<{ replayed?: number }>('/queue/replay', {})

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
