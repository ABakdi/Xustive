import { NextRequest, NextResponse } from 'next/server'

/**
 * Knowledge-panel source: Wikipedia.
 *
 * Runs on the Next server, never in the Rust API — the serving plane has no route to the internet
 * by design (ADR-0001), so an entity lookup that must reach out belongs here, in the web tier. The
 * browser talks only to this origin; this handler talks to Wikipedia. The image is proxied the same
 * way (see /api/wiki-image), so a reader's IP never reaches Wikimedia.
 *
 * It is deliberately conservative about *when* a panel appears: only short, entity-shaped queries,
 * and only when Wikipedia returns a real article ("standard" type) with an extract. A long or
 * question-shaped query returns 204 and the panel stays absent rather than guessing.
 */

const WIKI_BY_UI: Record<string, string> = { ar: 'ar', ary: 'ar', fr: 'fr', en: 'en' }
const UA = 'XustiveKnowledge/0.1 (+https://xustive.dz; contact via repository)'
const FETCH_TIMEOUT_MS = 6000

/** Words that mark a query as a how-to/question rather than an entity — no panel for these. */
const QUESTION_MARKERS = [
  'how ',
  'what ',
  'why ',
  'when ',
  'where ',
  'comment ',
  'pourquoi ',
  'كيف',
  'كيفاش',
  'لماذا',
  'علاش',
  'وين',
]

type WikiSummary = {
  type?: string
  title?: string
  description?: string
  extract?: string
  thumbnail?: { source?: string }
  content_urls?: { desktop?: { page?: string } }
  lang?: string
}

async function fetchJSON(url: string): Promise<unknown | null> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS)
  try {
    const res = await fetch(url, {
      headers: { 'User-Agent': UA, Accept: 'application/json' },
      signal: controller.signal,
      // Cache at the Next layer so a repeated entity does not hit Wikipedia every time.
      next: { revalidate: 3600 },
    })
    if (!res.ok) return null
    return await res.json()
  } catch {
    return null
  } finally {
    clearTimeout(timer)
  }
}

/** Top article title for a query on a given wiki, via the search API. */
async function topTitle(wiki: string, q: string): Promise<string | null> {
  const url =
    `https://${wiki}.wikipedia.org/w/api.php?action=query&format=json&list=search` +
    `&srsearch=${encodeURIComponent(q)}&srlimit=1&srnamespace=0&srqiprofile=classic`
  const data = (await fetchJSON(url)) as { query?: { search?: { title?: string }[] } } | null
  return data?.query?.search?.[0]?.title ?? null
}

async function summary(wiki: string, title: string): Promise<WikiSummary | null> {
  const url = `https://${wiki}.wikipedia.org/api/rest_v1/page/summary/${encodeURIComponent(title)}`
  return (await fetchJSON(url)) as WikiSummary | null
}

/** Do the query and the resolved title share a meaningful word? Guards against unrelated matches. */
function relevant(q: string, title: string): boolean {
  const norm = (s: string) =>
    s
      .toLowerCase()
      .replace(/[^\p{L}\p{N}\s]/gu, ' ')
      .split(/\s+/)
      .filter((w) => w.length > 2)
  const qWords = new Set(norm(q))
  if (qWords.size === 0) return true // single short token like a name
  return norm(title).some((w) => qWords.has(w))
}

export async function GET(req: NextRequest) {
  const q = (req.nextUrl.searchParams.get('q') ?? '').trim().replace(/^["']|["']$/g, '')
  const ui = req.nextUrl.searchParams.get('lang') ?? 'en'
  const wiki = WIKI_BY_UI[ui] ?? 'en'

  // Entity-shaped only: short, and not a how-to/question. Cheap gate before any network call.
  const words = q.split(/\s+/).filter(Boolean)
  const lower = ` ${q.toLowerCase()} `
  if (
    q.length < 2 ||
    q.length > 60 ||
    words.length > 8 ||
    q.includes('?') ||
    QUESTION_MARKERS.some((m) => lower.includes(m))
  ) {
    return new NextResponse(null, { status: 204 })
  }

  // Try the UI-language wiki, then English as a fallback for globally-known entities.
  const wikis = wiki === 'en' ? ['en'] : [wiki, 'en']
  for (const w of wikis) {
    const title = await topTitle(w, q)
    if (!title || !relevant(q, title)) continue
    const s = await summary(w, title)
    if (!s || s.type !== 'standard' || !s.extract) continue

    const thumb = s.thumbnail?.source
    return NextResponse.json(
      {
        title: s.title ?? title,
        description: s.description ?? null,
        extract: s.extract,
        // Proxied so the reader's browser never contacts Wikimedia directly.
        thumb: thumb ? `/api/wiki-image?u=${encodeURIComponent(thumb)}` : null,
        url: s.content_urls?.desktop?.page ?? `https://${w}.wikipedia.org/wiki/${encodeURIComponent(title)}`,
        lang: w,
      },
      { headers: { 'Cache-Control': 'private, max-age=600' } },
    )
  }

  return new NextResponse(null, { status: 204 })
}
