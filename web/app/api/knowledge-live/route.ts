import { NextRequest, NextResponse } from 'next/server'

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
const TIMEOUT_MS = 6000
const CANDIDATES = 7

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

async function json(url: string, init?: RequestInit): Promise<unknown> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS)
  try {
    const res = await fetch(url, {
      ...init,
      headers: { 'User-Agent': UA, Accept: 'application/json', ...(init?.headers ?? {}) },
      signal: controller.signal,
    })
    if (!res.ok) return null
    return await res.json()
  } catch {
    return null
  } finally {
    clearTimeout(timer)
  }
}

type Doc = {
  id: string
  sitelinks?: Record<string, { title?: string }>
  claims?: Record<string, { mainsnak?: { datavalue?: { value?: { id?: string } } } }[]>
  descriptions?: Record<string, { value?: string }>
}

function instanceOf(doc: Doc): string[] {
  return (doc.claims?.P31 ?? [])
    .map((c) => c.mainsnak?.datavalue?.value?.id)
    .filter((x): x is string => typeof x === 'string')
}

/** The best entity for a query, or null. */
async function resolve(q: string, lang: string): Promise<Doc | null> {
  const wd = 'https://www.wikidata.org/w/api.php'
  // Search in the reader's language first, then English — most entities have an English label.
  const langs = lang === 'en' ? ['en'] : [lang === 'ary' ? 'ar' : lang, 'en']
  const ids: string[] = []
  for (const l of langs) {
    const r = (await json(
      `${wd}?action=wbsearchentities&search=${encodeURIComponent(q)}&language=${l}&uselang=${l}&type=item&limit=${CANDIDATES}&format=json`,
    )) as { search?: { id: string }[] } | null
    for (const s of r?.search ?? []) if (!ids.includes(s.id)) ids.push(s.id)
    if (ids.length >= CANDIDATES) break
  }
  if (ids.length === 0) return null

  const r = (await json(
    `${wd}?action=wbgetentities&ids=${ids.slice(0, CANDIDATES).join('|')}` +
      `&props=labels|descriptions|aliases|claims|sitelinks&languages=ar|ary|fr|en|mul&format=json`,
  )) as { entities?: Record<string, Doc> } | null
  const docs = Object.values(r?.entities ?? {}).filter((d) => d && d.id)

  const things = docs.filter((d) => !instanceOf(d).some((c) => NOT_A_THING.has(c)))
  if (things.length === 0) return null
  // Prominence: the number of language editions that bothered to write about it. Crude, honest,
  // and exactly what separates Cristiano Ronaldo from a list of people called Ronaldo.
  things.sort(
    (a, b) => Object.keys(b.sitelinks ?? {}).length - Object.keys(a.sitelinks ?? {}).length,
  )
  return things[0] ?? null
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
  if (q.length < 2 || q.length > 60) return new NextResponse(null, { status: 204 })

  const doc = await resolve(q, lang)
  if (!doc) return new NextResponse(null, { status: 204 })

  const extract = await extractFor(doc, lang)
  const first = await render({ doc, lang, extract })
  if (!first) return new NextResponse(null, { status: 204 })

  // Second round: the labels the templates will actually show.
  const unresolved = first.unresolved ?? []
  if (unresolved.length === 0) {
    return NextResponse.json(first, { headers: { 'Cache-Control': 'private, max-age=600' } })
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
  return NextResponse.json(second, { headers: { 'Cache-Control': 'private, max-age=600' } })
}
