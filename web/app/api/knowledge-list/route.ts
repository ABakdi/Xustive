import { NextRequest, NextResponse } from 'next/server'

import { viaUpstream } from '@/lib/upstream'

import { detectRelation, SUBJECT_KINDS, type Relation } from '@/lib/relations'
import { commonsThumbUrl } from '@/lib/commons'
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
 * No ratings. Goodreads has had no public API since 2020 and forbids scraping, so a Goodreads
 * rating cannot honestly be shown, and a card that shows one source's number next to another
 * source's link invites the wrong reading. A book is its cover, its year and its doors.
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
 * The medium cover Open Library holds for a work, or null. The work record names its covers by
 * id; `/b/olid/<work>` does not resolve them, so the record is read first. Three seconds, then
 * the card goes out without a picture rather than late.
 */
async function openLibraryCover(workId: string): Promise<string | null> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), 3000)
  try {
    const r = (await fetch(`https://openlibrary.org/works/${encodeURIComponent(workId)}.json`, {
      ...viaUpstream(),
      headers: { 'User-Agent': UA, Accept: 'application/json' },
      signal: controller.signal,
    } as RequestInit).then((x) => (x.ok ? x.json() : null))) as { covers?: number[] } | null
    const id = (r?.covers ?? []).find((c) => typeof c === 'number' && c > 0)
    return id ? `https://covers.openlibrary.org/b/id/${id}-M.jpg` : null
  } catch {
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


/** The subject's Wikidata id, from the store first and the live path second. */
async function subjectId(
  subject: string,
  relation: Relation,
  lang: string,
  origin: string,
): Promise<Subject | null> {
  const kinds = SUBJECT_KINDS[relation]
  const stored = (await json(
    `${API}/api/v1/knowledge?q=${encodeURIComponent(subject)}&lang=${encodeURIComponent(lang)}`,
  )) as { id?: string; title?: string; kind?: string } | null
  // The store answers without a hint; a stored entity of the wrong kind is passed over for the
  // live path, which can be told what to prefer.
  if (stored?.id && (!stored.kind || kinds.includes(stored.kind))) {
    return { id: stored.id, title: stored.title ?? subject, kind: stored.kind ?? null }
  }
  const live = (await json(
    `${origin}/api/knowledge-live?q=${encodeURIComponent(subject)}&lang=${encodeURIComponent(lang)}&kind=${kinds.join(',')}`,
  )) as { id?: string; title?: string; kind?: string } | null
  return live?.id ? { id: live.id, title: live.title ?? subject, kind: live.kind ?? null } : null
}

type Subject = { id: string; title: string; kind: string | null }

/** A subject named by id — the reader picked it from "see also" — with its label in `lang`. */
async function subjectById(id: string, lang: string): Promise<Subject | null> {
  const l = lang === 'ary' ? 'ar' : lang
  const r = (await json(
    `https://www.wikidata.org/w/api.php?action=wbgetentities&ids=${id}&props=labels|claims&languages=${l}|en|mul&format=json`,
  )) as { entities?: Record<string, { labels?: Record<string, { value?: string }>; claims?: Claims }> } | null
  const e = r?.entities?.[id]
  if (!e) return null
  const title = e.labels?.[l]?.value ?? e.labels?.mul?.value ?? e.labels?.en?.value ?? id
  const p31 = ids(e.claims, 'P31')
  const kind = p31.some((c) => SERIES_CLASSES.has(c)) ? 'series' : p31.some((c) => FILM_CLASSES.has(c)) ? 'film' : null
  return { id, title, kind }
}

type Claims = Record<string, { mainsnak?: { datavalue?: { value?: { id?: string; time?: string } } } }[]>

/** The item ids a property points at. */
function ids(claims: Claims | undefined, prop: string): string[] {
  return (claims?.[prop] ?? [])
    .map((c) => c.mainsnak?.datavalue?.value?.id)
    .filter((x): x is string => typeof x === 'string')
}

const FILM_CLASSES = new Set(['Q11424', 'Q24862', 'Q202866', 'Q506240', 'Q226730'])
const SERIES_CLASSES = new Set(['Q5398426', 'Q15416', 'Q581714', 'Q63952888'])

type Related = { group: 'series' | 'seasons'; items: { id: string; title: string; year: string | null }[] }

/**
 * The other parts of what the subject belongs to: the films of its series (`P179` → the series'
 * `P527`), or a series' seasons (its own `P527`). Ten at most, in the order Wikidata lists them,
 * which is release order for every series anyone has curated. Nothing when the subject stands
 * alone — a row of one chip would be noise.
 */
async function related(subject: Subject, lang: string): Promise<Related | null> {
  const wd = 'https://www.wikidata.org/w/api.php'
  const own = (await json(`${wd}?action=wbgetclaims&entity=${subject.id}&property=P179&format=json`)) as
    | { claims?: Claims }
    | null
  const series = ids(own?.claims, 'P179')[0]
  let group: Related['group']
  let members: string[]
  if (series) {
    const parts = (await json(`${wd}?action=wbgetclaims&entity=${series}&property=P527&format=json`)) as
      | { claims?: Claims }
      | null
    group = 'series'
    members = ids(parts?.claims, 'P527')
  } else if (subject.kind === 'series') {
    const parts = (await json(`${wd}?action=wbgetclaims&entity=${subject.id}&property=P527&format=json`)) as
      | { claims?: Claims }
      | null
    group = 'seasons'
    members = ids(parts?.claims, 'P527')
  } else {
    return null
  }
  members = members.slice(0, 10)
  if (members.length < 2) return null
  const l = lang === 'ary' ? 'ar' : lang
  const r = (await json(
    `${wd}?action=wbgetentities&ids=${members.join('|')}&props=labels|claims&languages=${l}|en|mul&format=json`,
  )) as { entities?: Record<string, { labels?: Record<string, { value?: string }>; claims?: Claims }> } | null
  if (!r?.entities) return null
  const items = members
    .map((id) => {
      const e = r.entities?.[id]
      if (!e) return null
      const title = e.labels?.[l]?.value ?? e.labels?.mul?.value ?? e.labels?.en?.value ?? id
      const time = (e.claims?.P577 ?? e.claims?.P580 ?? [])[0]?.mainsnak?.datavalue?.value?.time
      return { id, title, year: time ? time.slice(1, 5) : null }
    })
    .filter((x): x is Related['items'][number] => x !== null)
  return items.length ? { group, items } : null
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


export async function GET(req: NextRequest) {
  const q = (req.nextUrl.searchParams.get('q') ?? '').trim()
  const lang = req.nextUrl.searchParams.get('lang') ?? 'en'
  const rq = detectRelation(q)
  if (!rq) return new NextResponse(null, { status: 204 })

  const origin = req.nextUrl.origin
  // `subject=Q189600` — the reader picked a part or a season from "see also": that one, by id.
  const picked = (req.nextUrl.searchParams.get('subject') ?? '').trim()
  if (picked && !/^Q\d{1,12}$/.test(picked)) return new NextResponse(null, { status: 400 })
  const subject = picked ? await subjectById(picked, lang) : await subjectId(rq.subject, rq.relation, lang, origin)
  if (!subject) {
    console.warn(`[knowledge-list] no subject for relation=${rq.relation}`)
    return new NextResponse(null, { status: 204 })
  }

  const [rows, siblings] = await Promise.all([
    members(subject.id, rq.relation, lang),
    rq.relation === 'cast' ? related(subject, lang).catch(() => null) : Promise.resolve(null),
  ])
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
      const image = row.image?.value
      // Wikidata seldom carries a book's cover; Open Library nearly always does, by its own
      // cover id, one small JSON away. Only for books, and only when Wikidata had nothing.
      const cover = !image && rq.relation === 'books' && ol ? await openLibraryCover(ol) : null
      return {
        id,
        title: row.itemLabel?.value ?? id,
        description: row.itemDescription?.value ?? null,
        year: row.date?.value ? row.date.value.slice(0, 4) : null,
        thumb: image
          ? signThumb(commonsThumbUrl(decodeURIComponent(image.split('/').pop() ?? '')))
          : cover
            ? signThumb(cover)
            : null,
        links,
      }
  })

  return NextResponse.json(
    { relation: rq.relation, subject: { id: subject.id, title: subject.title }, related: siblings, wiki, cards },
    { headers: { 'Cache-Control': 'private, max-age=300' } },
  )
}
