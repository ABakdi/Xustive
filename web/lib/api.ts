/**
 * Typed client for the Rust API.
 *
 * Hand-written for now; M1B-T01.3 replaces it with types generated from an OpenAPI description,
 * so a contract change becomes a compile error. Until then these interfaces are the contract and
 * must be kept honest — the whole reason for the rewrite was two renderers drifting apart.
 */

/** Server-side requests go direct; browser requests go through the rewrite. */
const BASE = typeof window === 'undefined'
  ? (process.env.XUSTIVE_API_URL ?? 'http://127.0.0.1:8080')
  : ''

export interface Sentiment {
  label: 'positive' | 'neutral' | 'negative'
  score: number
  confidence: number
}

export interface ResultCard {
  id: string
  title: string
  url: string
  display_url: string
  excerpt: string
  source_type: string
  source_name: string
  published_at: number
  published_at_precision: string
  sentiment: Sentiment | null
  language: string
  score: number
  similar_count?: number
}

export interface InstantAnswer {
  tool: string
  confidence: number
  /** How the query was read, so a misreading is visible rather than silent. */
  interpretation: string
  value: string
  detail?: unknown
  /** When the underlying data was measured. Absent means timeless — arithmetic has no age. */
  as_of?: number
}

export interface SearchResponse {
  query: {
    raw: string
    normalized: string
    language: string
    language_confidence: number
    expanded_terms: string[]
  }
  summary_token: string | null
  instant?: InstantAnswer
  pagination: {
    page: number
    hits_per_page: number
    total_hits: number
    total_pages: number
    estimated: boolean
  }
  took_ms: number
  results: ResultCard[]
  facets: Record<string, Record<string, number>>
}

export interface Suggestion {
  text: string
  source: string
}

export interface ApiError {
  error: { code: string; message: string }
}

export class SearchFailed extends Error {
  constructor(readonly code: string, message: string, readonly status: number) {
    super(message)
  }
}

export async function search(params: URLSearchParams): Promise<SearchResponse> {
  const res = await fetch(`${BASE}/api/v1/search?${params}`, {
    headers: { Accept: 'application/json' },
    // Results are per-query and change as the index does. Caching them would serve a stale
    // corpus and, worse, could serve one user's query response to another.
    cache: 'no-store',
  })

  if (!res.ok) {
    const body = (await res.json().catch(() => null)) as ApiError | null
    throw new SearchFailed(
      body?.error?.code ?? 'unknown',
      body?.error?.message ?? 'Search failed',
      res.status,
    )
  }
  return res.json() as Promise<SearchResponse>
}

export async function suggest(q: string, signal?: AbortSignal): Promise<Suggestion[]> {
  const res = await fetch(`${BASE}/api/v1/suggest?q=${encodeURIComponent(q)}&limit=8`, {
    headers: { Accept: 'application/json' },
    signal,
    cache: 'no-store',
  })
  if (!res.ok) return []
  const data = (await res.json()) as { suggestions?: Suggestion[] }
  return data.suggestions ?? []
}

export interface SummaryResponse {
  summary: string | null
  citations?: { n: number; result_id: string }[]
  reason?: string
  took_ms: number
}

export async function summarise(token: string, signal?: AbortSignal): Promise<SummaryResponse> {
  const res = await fetch(`${BASE}/api/v1/summary`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token }),
    signal,
    cache: 'no-store',
  })
  if (!res.ok) return { summary: null, took_ms: 0 }
  return res.json() as Promise<SummaryResponse>
}

/**
 * The tool inventory.
 *
 * Fetched rather than hardcoded, so the settings page cannot drift from what the engine actually
 * runs — a tool missing from a hardcoded list looks exactly like a tool that is switched off.
 *
 * Cached: the list is static content that depends on nothing about the request, so re-fetching it
 * per page view would be pure waste.
 */
export async function tools(): Promise<{ id: string; keyword: string }[]> {
  const res = await fetch(`${BASE}/api/v1/tools`, {
    headers: { Accept: 'application/json' },
    next: { revalidate: 3600 },
  })
  if (!res.ok) return []
  const body = (await res.json()) as { tools?: { id: string; keyword: string }[] }
  return body.tools ?? []
}
