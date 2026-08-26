import { NextRequest, NextResponse } from 'next/server'

import { viaUpstream } from '@/lib/upstream'

/**
 * The live entity fallback (M8-T03.6 done properly).
 *
 * When the knowledge store does not hold an entity, this looks it up on Wikidata from the web
 * tier — the one place with egress that ADR-0014 already sanctioned — and hands the raw document
 * to the API's `/knowledge/render`, so the panel is built by the same parser and templates a
 * harvested entity gets. No second, weaker parser in TypeScript.
 *
 * The part that fixes "ronaldo → a name list": candidates are ranked by **prominence** (sitelink
 * count) after **disambiguation, given-name and family-name pages are removed**. Wikipedia's first
 * search hit for a bare surname is the name article; the thing people mean is the entity with the
 * most language editions that is not one.
 */

const UA = 'XustiveKnowledge/0.1 (+https://xustive.dz; contact via repository)'
const API = process.env.XUSTIVE_API_URL ?? 'http://127.0.0.1:8080'
const TIMEOUT_MS = 12000
/** Names Wikidata's search may return per language. Zinedine Zidane is eighth for "zidane"; a
 *  shortlist of seven never saw him and the resolver picked an Algerian namesake. */
const CANDIDATES = 12

/** `P31` classes that are about a *name*, not a thing. A panel on one is the "name list" failure. */
const NOT_A_THING = new Set([
  'Q4167410', // Wikimedia disambiguation page
  'Q101352', // family name
  'Q202444', // given name
  'Q12308941', // male given name
  'Q11879590', // female given name
  'Q4167836', // Wikimedia category
  'Q13406463', // Wikimedia list article
  'Q11266439', // Wikimedia template
])

const WIKI_OF: Record<string, string> = { ar: 'ar', ary: 'ar', fr: 'fr', en: 'en' }

async function json(url: string, init?: RequestInit, attempt = 0): Promise<unknown> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS)
  try {
    const res = await fetch(url, {
      ...init,
      ...viaUpstream(),
      headers: { 'User-Agent': UA, Accept: 'application/json', ...(init?.headers ?? {}) },
      signal: controller.signal,
    } as RequestInit)
    if (res.status === 204) return null
    if (!res.ok) {
      // Loud in the server log, silent to the reader: an upstream refusing or throttling us is
      // the operator's problem to see, and a 204 on the page hides it completely. The host is
      // logged; the query is not.
      console.warn(`[knowledge] ${new URL(url).host} answered ${res.status}`)
      return null
    }
    return await res.json()
  } catch (e) {
    const cause = (e as { cause?: { code?: string; message?: string } }).cause
    console.warn(
      `[knowledge] ${new URL(url).host} failed: ${(e as Error).name}${cause ? ` (${cause.code ?? cause.message})` : ''}`,
    )
    // A refused or reset connection is retried once: Wikimedia's edge drops a surplus
    // connection rather than queueing it, and the second attempt reuses the pool's live one.
    if (attempt === 0 && cause?.code && /CONNECT|RESET|SOCKET/.test(cause.code)) {
      clearTimeout(timer)
      return json(url, init, 1)
    }
    return null
  } finally {
    clearTimeout(timer)
  }
}

/**
 * Run `fn` over `items` with at most `limit` in flight.
 *
 * Wikimedia's API etiquette asks for a handful of concurrent connections per client at most, and
 * it enforces it: a dozen parallel requests came back as connection resets ("fetch failed"), not
 * as slow answers. Three at a time is polite and, for a dozen tiny calls, still under a second.
 */
async function mapLimit<T, R>(items: T[], limit: number, fn: (item: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(items.length)
  let next = 0
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const i = next++
      out[i] = await fn(items[i]!)
    }
  })
  await Promise.all(workers)
  return out
}

type Doc = {
  id: string
  sitelinks?: Record<string, { title?: string }>
  claims?: Record<string, { mainsnak?: { datavalue?: { value?: { id?: string } } } }[]>
  descriptions?: Record<string, { value?: string }>
}

/** A page about a *name* rather than a thing, from its description — the phase-one documents carry
 *  no claims. "family name", "given name", "disambiguation page" are what Wikidata writes there. */
function isNamePage(doc: Doc): boolean {
  const descs = Object.values(doc.descriptions ?? {}).map((d) => (d.value ?? '').toLowerCase())
  if (descs.some((d) => /disambiguation|given name|family name|surname|name list|wikimedia/.test(d)))
    return true
  return instanceOf(doc).some((c) => NOT_A_THING.has(c))
}

function instanceOf(doc: Doc): string[] {
  return (doc.claims?.P31 ?? [])
    .map((c) => c.mainsnak?.datavalue?.value?.id)
    .filter((x): x is string => typeof x === 'string')
}

/**
 * Candidate documents for a query — **names only**, no claims.
 *
 * Two phases, because a full Wikidata document is megabytes and a dozen of them timed out. The
 * resolver only needs what a name lookup needs — labels, aliases, descriptions, and sitelinks for
 * prominence — so that is all this fetches. The claims are fetched once, for the winner.
 */
async function candidates(q: string, lang: string): Promise<Doc[]> {
  const wd = 'https://www.wikidata.org/w/api.php'
  const langs = lang === 'en' ? ['en'] : [lang === 'ary' ? 'ar' : lang, 'en']
  const ids: string[] = []
  for (const l of langs) {
    const r = (await json(
      `${wd}?action=wbsearchentities&search=${encodeURIComponent(q)}&language=${l}&uselang=${l}&type=item&limit=${CANDIDATES}&format=json`,
    )) as { search?: { id: string }[] } | null
    for (const s of r?.search ?? []) if (!ids.includes(s.id)) ids.push(s.id)
  }
  if (ids.length === 0) return []
  const r = (await json(
    `${wd}?action=wbgetentities&ids=${ids.slice(0, CANDIDATES * 2).join('|')}` +
      `&props=labels|descriptions|aliases|sitelinks&languages=ar|ary|fr|en|mul&format=json`,
  )) as { entities?: Record<string, Doc> } | null
  const docs = Object.values(r?.entities ?? {}).filter(
    // Name pages — disambiguation, given names, family names — are never the thing meant. Without
    // claims in this phase the check is on the description Wikidata writes for them.
    (d) => d && d.id && !isNamePage(d),
  )
  // The candidates' `instance of`, in one small SPARQL call, so the resolver can tell a town
  // from a director in phase one without fetching a dozen multi-megabyte documents. Synthesised
  // into the document shape the parser reads, so nothing downstream knows the difference.
  const kinds = await instanceOfMany(docs.map((d) => d.id))
  // One failed lookup is a refusal for the whole shortlist. With the director's kind unknown and
  // the painter's known, "films by spielberg" resolved to Johannes Spilberg — a correct choice
  // among the candidates that could be typed, and the wrong answer. Partial knowledge of the
  // field is not knowledge of the winner.
  if (!kinds) {
    console.warn('[knowledge] kind lookup incomplete; declining rather than choosing among the known')
    return []
  }
  for (const d of docs) {
    const p31 = kinds.get(d.id)
    if (p31?.length) {
      d.claims = { P31: p31.map((id) => ({ mainsnak: { datavalue: { value: { id } } } })) }
    }
  }
  return docs.filter((d) => !instanceOf(d).some((c) => NOT_A_THING.has(c)))
}

/**
 * `instance of` for many ids at once: `Map<id, [class ids]>`.
 *
 * One small `wbgetclaims` call per id, in parallel — not SPARQL. The SPARQL endpoint took 6.8 s
 * for a six-id VALUES lookup on the day this was written, which is queueing rather than work,
 * and a resolver that waits on a queue is a panel that never arrives. The claims API answers
 * in tens of milliseconds and carries only the one property asked for.
 */
async function instanceOfMany(ids: string[]): Promise<Map<string, string[]> | null> {
  const out = new Map<string, string[]>()
  let failed = false
  const results = await mapLimit(ids, 3, async (id) => {
      const r = (await json(
        `https://www.wikidata.org/w/api.php?action=wbgetclaims&entity=${id}&property=P31&format=json`,
      )) as { claims?: { P31?: { mainsnak?: { datavalue?: { value?: { id?: string } } } }[] } } | null
      // An entity with no `instance of` is a fact; a lookup that never answered is not.
      if (r === null) failed = true
      const cls = (r?.claims?.P31 ?? [])
        .map((c) => c.mainsnak?.datavalue?.value?.id)
        .filter((x): x is string => typeof x === 'string')
      return [id, cls] as const
  })
  if (failed) return null
  for (const [id, cls] of results) if (cls.length) out.set(id, cls)
  return out
}

/** The full document for one entity. */
async function fullDocument(id: string): Promise<Doc | null> {
  const r = (await json(
    `https://www.wikidata.org/w/api.php?action=wbgetentities&ids=${id}` +
      `&props=labels|descriptions|aliases|claims|sitelinks&languages=ar|ary|fr|en|mul&format=json`,
  )) as { entities?: Record<string, Doc> } | null
  return r?.entities?.[id] ?? null
}

/**
 * Which candidate the query means — decided by the API, not here.
 *
 * The first version ranked by sitelink count in this file and resolved "messi" on the French page
 * to Jesus Christ (the French search matched *Messie*). The store's own resolver — exact name
 * first, corpus agreement, a precision floor — already knows better, so the candidates go to it
 * and it names the winner or declines.
 */
async function resolve(q: string, lang: string, preferKinds: string[] = []): Promise<Doc | null> {
  const docs = await candidates(q, lang)
  if (docs.length === 0) return null
  const res = await fetch(`${API}/api/v1/knowledge/resolve-live`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query: q, docs, prefer_kinds: preferKinds }),
    cache: 'no-store',
  })
  if (res.status === 204 || !res.ok) return null
  const { id } = (await res.json()) as { id?: string }
  if (!id) return null
  return (await fullDocument(id)) ?? docs.find((d) => d.id === id) ?? null
}

/** The reader's-language Wikipedia extract for a document, when it has an article. */
async function extractFor(doc: Doc, lang: string) {
  const wiki = WIKI_OF[lang] ?? 'en'
  for (const w of wiki === 'en' ? ['en'] : [wiki, 'en']) {
    const title = doc.sitelinks?.[`${w}wiki`]?.title
    if (!title) continue
    const s = (await json(
      `https://${w}.wikipedia.org/api/rest_v1/page/summary/${encodeURIComponent(title.replace(/ /g, '_'))}`,
    )) as { type?: string; extract?: string; content_urls?: { desktop?: { page?: string } } } | null
    if (s?.type === 'standard' && s.extract) {
      return { lang: w, text: s.extract, url: s.content_urls?.desktop?.page ?? '' }
    }
  }
  return null
}

async function render(body: Record<string, unknown>) {
  const res = await fetch(`${API}/api/v1/knowledge/render`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    cache: 'no-store',
  })
  if (res.status === 204 || !res.ok) return null
  return (await res.json()) as Record<string, unknown> & { unresolved?: string[] }
}

export async function GET(req: NextRequest) {
  const q = (req.nextUrl.searchParams.get('q') ?? '').trim()
  const lang = req.nextUrl.searchParams.get('lang') ?? 'en'
  // `kind=person,film` — what the caller knows the subject must be (the list route passes the
  // kinds its relation implies). A lift in the resolver, never a filter.
  const preferKinds = (req.nextUrl.searchParams.get('kind') ?? '')
    .split(',')
    .map((k) => k.trim())
    .filter(Boolean)
  if (q.length < 2 || q.length > 60) return new NextResponse(null, { status: 204 })

  const doc = await resolve(q, lang, preferKinds)
  if (!doc) return new NextResponse(null, { status: 204 })

  const extract = await extractFor(doc, lang)
  const first = await render({ doc, lang, extract })
  if (!first) return new NextResponse(null, { status: 204 })

  // Second round: the labels the templates will actually show.
  const unresolved = first.unresolved ?? []
  if (unresolved.length === 0) {
    return NextResponse.json(first, { headers: { 'Cache-Control': 'private, max-age=300' } })
  }
  const r = (await json(
    `https://www.wikidata.org/w/api.php?action=wbgetentities&ids=${unresolved.slice(0, 50).join('|')}` +
      `&props=labels&languages=${lang === 'ary' ? 'ar' : lang}|en|mul&format=json`,
  )) as { entities?: Record<string, { labels?: Record<string, { value?: string }> }> } | null
  const labels: [string, string][] = []
  for (const [id, e] of Object.entries(r?.entities ?? {})) {
    const l = e.labels?.[lang === 'ary' ? 'ar' : lang]?.value ?? e.labels?.mul?.value ?? e.labels?.en?.value
    if (l) labels.push([id, l])
  }
  const second = (await render({ doc, lang, extract, labels })) ?? first
  return NextResponse.json(second, { headers: { 'Cache-Control': 'private, max-age=300' } })
}
