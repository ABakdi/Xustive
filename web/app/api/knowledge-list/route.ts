import { NextRequest, NextResponse } from 'next/server'

import { viaUpstream } from '@/lib/upstream'

import { detectRelation, type Relation } from '@/lib/relations'
import { signThumb } from '@/lib/thumb'

/**
 * List answers (M8-T11): the cast of a film, the books of an author, the films of a director.
 *
 * Three steps, each already sanctioned. The **subject** is resolved by the store, or live through
 * the same path the entity panel uses — never re-implemented here. The **members** come from one
 * Wikidata SPARQL query, from the web tier, which is the one place with egress that ADR-0014
 * allows. And every card **links to authorities by identifier** — Wikipedia, IMDb, Goodreads,
 * Open Library, Google Books — none of which is fetched or scraped (ADR-0019).
 *
 * Ratings: Goodreads has had no public API since 2020 and forbids scraping, so a Goodreads rating
 * cannot honestly be shown. Open Library publishes ratings openly, and those are shown with the
 * source named. The Goodreads link is still there for the reader to click.
 */

const UA = 'XustiveKnowledge/0.1 (+https://xustive.dz; contact via repository)'
const API = process.env.XUSTIVE_API_URL ?? 'http://127.0.0.1:8080'
const TIMEOUT_MS = 12000
const MAX_MEMBERS = 16

const WIKI_OF: Record<string, string> = { ar: 'ar', ary: 'ar', fr: 'fr', en: 'en' }

async function json(url: string, init?: RequestInit, attempt = 0): Promise<unknown> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS)
  try {
    // The pool with the four-connection cap is for Wikimedia and Open Library. The self-call to
    // the live route must not queue behind them: it was seen timing out against its own host.
    const pooled = /(^|\.)(wikidata|wikipedia|openlibrary)\.org$/.test(new URL(url).hostname)
    const res = await fetch(url, {
      ...init,
      ...(pooled ? viaUpstream() : {}),
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

/** What kind of thing each relation's subject must be. The hint that keeps "films by
 *  spielberg" from resolving to the Austrian town whose name is an exact match. */
const SUBJECT_KINDS: Record<Relation, string[]> = {
  cast: ['film', 'series'],
  books: ['person'],
  films: ['person'],
  albums: ['person', 'music'],
}

/** The subject's Wikidata id, from the store first and the live path second. */
async function subjectId(
  subject: string,
  relation: Relation,
  lang: string,
  origin: string,
): Promise<{ id: string; title: string } | null> {
  const kinds = SUBJECT_KINDS[relation]
  const stored = (await json(
    `${API}/api/v1/knowledge?q=${encodeURIComponent(subject)}&lang=${encodeURIComponent(lang)}`,
  )) as { id?: string; title?: string; kind?: string } | null
  // The store answers without a hint; a stored entity of the wrong kind is passed over for the
  // live path, which can be told what to prefer.
  if (stored?.id && (!stored.kind || kinds.includes(stored.kind))) {
    return { id: stored.id, title: stored.title ?? subject }
  }
  const live = (await json(
    `${origin}/api/knowledge-live?q=${encodeURIComponent(subject)}&lang=${encodeURIComponent(lang)}&kind=${kinds.join(',')}`,
  )) as { id?: string; title?: string } | null
  return live?.id ? { id: live.id, title: live.title ?? subject } : null
}

/** The SPARQL pattern that lists the members of a relation for subject `S`. */
function membersPattern(relation: Relation): string {
  switch (relation) {
    case 'cast':
      return 'wd:S wdt:P161 ?item .'
    case 'books':
      // Works whose author is S, restricted to written-work classes so a lecture or a letter does
      // not appear as a book.
      return `?item wdt:P50 wd:S .
              ?item wdt:P31 ?cls .
              VALUES ?cls { wd:Q571 wd:Q7725634 wd:Q8261 wd:Q47461344 wd:Q49084 wd:Q1279564 wd:Q35760 }`
    case 'films':
      // Directed by, or starring — a director's filmography and an actor's are the same question.
      // A VALUES list of film classes rather than `wdt:P31/wdt:P279*`: the subclass path walks
      // the whole class tree per candidate and timed out on SPARQL for every director tried.
      return `{ ?item wdt:P57 wd:S } UNION { ?item wdt:P161 wd:S }
              ?item wdt:P31 ?cls .
              VALUES ?cls { wd:Q11424 wd:Q24869 wd:Q506240 wd:Q93204 wd:Q202866 wd:Q5398426 }`
    case 'albums':
      return '?item wdt:P175 wd:S . ?item wdt:P31 wd:Q482994 .'
  }
}

type Row = Record<string, { value: string } | undefined>

/** SPARQL's leash. The endpoint queues under load — 6.8 s for a trivial lookup was observed —
 *  and past this the claims path below answers instead. */
const SPARQL_TIMEOUT_MS = 6000

async function members(subject: string, relation: Relation, lang: string): Promise<Row[]> {
  const viaSparql = await membersSparql(subject, relation, lang)
  if (viaSparql.length > 0) return viaSparql
  return membersFromClaims(subject, relation, lang)
}

/**
 * The fallback when SPARQL stalls: the subject's own claims. Forward relations are exact — the
 * cast is `P161` on the film. Reverse ones are approximated by `P800` (notable work) on the
 * person, which is a shorter list than the truth, and honest about it: it is what the entity
 * itself says its notable works are.
 */
async function membersFromClaims(subject: string, relation: Relation, lang: string): Promise<Row[]> {
  const wd = 'https://www.wikidata.org/w/api.php'
  const prop = relation === 'cast' ? 'P161' : 'P800'
  const own = (await json(`${wd}?action=wbgetclaims&entity=${subject}&property=${prop}&format=json`)) as {
    claims?: Record<string, { mainsnak?: { datavalue?: { value?: { id?: string } } } }[]>
  } | null
  const ids = (own?.claims?.[prop] ?? [])
    .map((c) => c.mainsnak?.datavalue?.value?.id)
    .filter((x): x is string => typeof x === 'string')
    .slice(0, 12)
  if (ids.length === 0) return []
  const wiki = WIKI_OF[lang] ?? 'en'
  type Ent = {
    id: string
    labels?: Record<string, { value: string }>
    descriptions?: Record<string, { value: string }>
    sitelinks?: Record<string, { title?: string }>
    claims?: Record<string, { mainsnak?: { datavalue?: { value?: unknown } } }[]>
  }
  const r = (await json(
    `${wd}?action=wbgetentities&ids=${ids.join('|')}&props=labels|descriptions|claims|sitelinks&languages=${wiki}|mul|en|fr|ar&format=json`,
  )) as { entities?: Record<string, Ent> } | null
  const first = (e: Ent, p: string) => {
    const v = e.claims?.[p]?.[0]?.mainsnak?.datavalue?.value
    if (typeof v === 'string') return v
    if (v && typeof v === 'object' && 'time' in v) return String((v as { time: string }).time).replace(/^\+/, '')
    return undefined
  }
  const rows: Row[] = []
  for (const id of ids) {
    const e = r?.entities?.[id]
    if (!e) continue
    const label = e.labels?.[wiki]?.value ?? e.labels?.mul?.value ?? e.labels?.en?.value ?? id
    const desc = e.descriptions?.[wiki]?.value ?? e.descriptions?.en?.value
    const title = e.sitelinks?.[`${wiki}wiki`]?.title
    const img = first(e, 'P18')
    const row: Row = {
      item: { value: `http://www.wikidata.org/entity/${id}` },
      itemLabel: { value: label },
      itemDescription: desc ? { value: desc } : undefined,
      image: img ? { value: `http://commons.wikimedia.org/wiki/Special:FilePath/${encodeURIComponent(img)}` } : undefined,
      imdb: first(e, 'P345') ? { value: first(e, 'P345')! } : undefined,
      goodreads: first(e, 'P2969') ? { value: first(e, 'P2969')! } : undefined,
      ol: first(e, 'P648') ? { value: first(e, 'P648')! } : undefined,
      isbn: first(e, 'P212') ? { value: first(e, 'P212')! } : undefined,
      date: first(e, 'P577') ?? first(e, 'P569') ? { value: (first(e, 'P577') ?? first(e, 'P569'))! } : undefined,
      article: title ? { value: `https://${wiki}.wikipedia.org/wiki/${encodeURIComponent(title.replace(/ /g, '_'))}` } : undefined,
    }
    rows.push(row)
  }
  return rows
}

async function membersSparql(subject: string, relation: Relation, lang: string): Promise<Row[]> {
  const wiki = WIKI_OF[lang] ?? 'en'
  const query = `
    SELECT ?item ?itemLabel ?itemDescription ?image ?imdb ?goodreads ?ol ?isbn ?date ?article WHERE {
      ${membersPattern(relation).replace(/wd:S\b/g, `wd:${subject}`)}
      OPTIONAL { ?item wdt:P18 ?image }
      OPTIONAL { ?item wdt:P345 ?imdb }
      OPTIONAL { ?item wdt:P2969 ?goodreads }
      OPTIONAL { ?item wdt:P648 ?ol }
      OPTIONAL { ?item wdt:P212 ?isbn }
      OPTIONAL { ?item wdt:P577 ?date }
      OPTIONAL { ?item wdt:P569 ?date }
      OPTIONAL { ?article schema:about ?item ; schema:isPartOf <https://${wiki}.wikipedia.org/> }
      SERVICE wikibase:label { bd:serviceParam wikibase:language "${wiki},mul,en,fr,ar". }
    } LIMIT ${MAX_MEMBERS * 3}`
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), SPARQL_TIMEOUT_MS)
  const r = (await fetch(`https://query.wikidata.org/sparql?format=json&query=${encodeURIComponent(query)}`, {
    ...viaUpstream(),
    headers: { 'User-Agent': UA, Accept: 'application/json' },
    signal: controller.signal,
  } as RequestInit)
    .then((res) => (res.ok ? res.json() : (console.warn(`[knowledge] sparql answered ${res.status}`), null)))
    .catch((e) => (console.warn(`[knowledge] sparql failed: ${(e as Error).name}`), null))
    .finally(() => clearTimeout(timer))) as { results?: { bindings?: Row[] } } | null
  const rows = r?.results?.bindings ?? []
  // One row per item (OPTIONALs multiply rows); first wins, which keeps SPARQL's order.
  const seen = new Set<string>()
  const out: Row[] = []
  for (const row of rows) {
    const id = row.item?.value.split('/').pop() ?? ''
    if (!id || seen.has(id)) continue
    seen.add(id)
    out.push(row)
    if (out.length >= MAX_MEMBERS) break
  }
  return out
}

/** Open Library's average rating for a work, when it has one. Open data, named as the source. */
async function openLibraryRating(olId: string): Promise<{ average: number; count: number } | null> {
  // A short leash of its own: Open Library can take ten seconds to answer, and a rating is an
  // extra on a card that is already complete without it — it must never hold the row.
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), 3000)
  const r = (await fetch(`https://openlibrary.org/works/${encodeURIComponent(olId)}/ratings.json`, {
    ...viaUpstream(),
    headers: { 'User-Agent': UA, Accept: 'application/json' },
    signal: controller.signal,
  } as RequestInit)
    .then((res) => (res.ok ? res.json() : null))
    .catch(() => null)
    .finally(() => clearTimeout(timer))) as {
    summary?: { average?: number; count?: number }
  } | null
  const s = r?.summary
  if (!s || !s.average || !s.count) return null
  return { average: Math.round(s.average * 10) / 10, count: s.count }
}

export async function GET(req: NextRequest) {
  const q = (req.nextUrl.searchParams.get('q') ?? '').trim()
  const lang = req.nextUrl.searchParams.get('lang') ?? 'en'
  const rq = detectRelation(q)
  if (!rq) return new NextResponse(null, { status: 204 })

  const origin = req.nextUrl.origin
  const subject = await subjectId(rq.subject, rq.relation, lang, origin)
  if (!subject) {
    console.warn(`[knowledge-list] no subject for relation=${rq.relation}`)
    return new NextResponse(null, { status: 204 })
  }

  const rows = await members(subject.id, rq.relation, lang)
  console.warn(`[knowledge-list] relation=${rq.relation} subject=${subject.id} members=${rows.length}`)
  if (rows.length === 0) return new NextResponse(null, { status: 204 })

  const wiki = WIKI_OF[lang] ?? 'en'
  const cards = await mapLimit(rows, 3, async (row) => {
      const id = row.item?.value.split('/').pop() ?? ''
      const ol = row.ol?.value
      const isbn = row.isbn?.value
      const links: { key: string; url: string }[] = []
      if (row.article?.value) links.push({ key: 'wikipedia', url: row.article.value })
      else links.push({ key: 'wikidata', url: `https://www.wikidata.org/wiki/${id}` })
      if (row.imdb?.value) {
        const imdb = row.imdb.value
        links.push({
          key: 'imdb',
          url: imdb.startsWith('nm')
            ? `https://www.imdb.com/name/${imdb}/`
            : `https://www.imdb.com/title/${imdb}/`,
        })
      }
      if (row.goodreads?.value)
        links.push({ key: 'goodreads', url: `https://www.goodreads.com/book/show/${row.goodreads.value}` })
      if (ol) links.push({ key: 'openlibrary', url: `https://openlibrary.org/works/${ol}` })
      if (isbn)
        links.push({ key: 'googlebooks', url: `https://books.google.com/books?vid=ISBN${isbn.replace(/-/g, '')}` })
      const rating = rq.relation === 'books' && ol ? await openLibraryRating(ol) : null
      const image = row.image?.value
      return {
        id,
        title: row.itemLabel?.value ?? id,
        description: row.itemDescription?.value ?? null,
        year: row.date?.value ? row.date.value.slice(0, 4) : null,
        thumb: image
          ? signThumb(
              `https://commons.wikimedia.org/wiki/Special:FilePath/${encodeURIComponent(
                decodeURIComponent(image.split('/').pop() ?? ''),
              )}?width=240`,
            )
          : null,
        links,
        rating: rating ? { ...rating, source: 'Open Library' } : null,
      }
  })

  return NextResponse.json(
    { relation: rq.relation, subject: { id: subject.id, title: subject.title }, wiki, cards },
    { headers: { 'Cache-Control': 'private, max-age=300' } },
  )
}
