/**
 * Relation queries (M8-T11): an entity plus a relationship — *cast of X*, *books by Y*, *films by
 * Z*. The answer is a list of entities, not one, so it gets a row of cards rather than a panel.
 *
 * Pure and cheap: this runs on the server for every search to decide whether to mount the list
 * component at all, so it is a handful of regular expressions over the raw query and nothing
 * else. Precision over recall, the same rule as the entity panel — a list of the wrong people is
 * worse than no list.
 */

export type Relation =
  | 'cast' // people in a film or series
  | 'books' // written works by an author
  | 'films' // films by a director, or films an actor appears in
  | 'albums' // albums by a performer

export interface RelationQuery {
  relation: Relation
  /** The entity the relation is about, as typed. */
  subject: string
}

/**
 * Patterns per relation, in all four scripts the audience types. The subject is the capture.
 * Order matters within a relation only for the capture; the first relation that matches wins.
 */
const PATTERNS: [Relation, RegExp][] = [
  // cast of X / X cast / actors in X / who plays in X
  ['cast', /^(?:the\s+)?cast\s+of\s+(.+)$/i],
  ['cast', /^(.+?)\s+cast$/i],
  ['cast', /^(?:actors?|stars?)\s+(?:in|of)\s+(.+)$/i],
  ['cast', /^(?:casting|acteurs|distribution)\s+(?:de|du|d['’]|des)\s*(.+)$/i],
  ['cast', /^(.+?)\s+(?:casting|acteurs|distribution)$/i],
  ['cast', /^(?:طاقم|أبطال|ممثلي|ممثلو|ممثلين)\s+(?:فيلم\s+|مسلسل\s+)?(.+)$/],
  ['cast', /^(.+?)\s+(?:الممثلين|الأبطال|طاقم التمثيل)$/],
  // books by X / X books / novels of X
  ['books', /^(?:books?|novels?|works?)\s+(?:by|of|from)\s+(.+)$/i],
  ['books', /^(.+?)\s+(?:books|novels|bibliography)$/i],
  ['books', /^(?:livres?|romans?|œuvres?|oeuvres?)\s+(?:de|du|d['’]|des)\s*(.+)$/i],
  ['books', /^(.+?)\s+(?:livres|romans)$/i],
  ['books', /^(?:كتب|روايات|مؤلفات)\s+(.+)$/],
  ['books', /^(.+?)\s+(?:كتب|روايات)$/],
  // films by X / X movies / filmography
  ['films', /^(?:films?|movies?)\s+(?:by|of|with|starring|directed by)\s+(.+)$/i],
  ['films', /^(.+?)\s+(?:films|movies|filmography)$/i],
  ['films', /^(?:films?|filmographie)\s+(?:de|du|d['’]|des|avec)\s*(.+)$/i],
  ['films', /^(.+?)\s+(?:films|filmographie)$/i],
  ['films', /^(?:أفلام|افلام)\s+(.+)$/],
  ['films', /^(.+?)\s+(?:أفلام|افلام)$/],
  // albums by X / X albums / discography
  ['albums', /^(?:albums?|discography|songs?)\s+(?:by|of)\s+(.+)$/i],
  ['albums', /^(.+?)\s+(?:albums|discography)$/i],
  ['albums', /^(?:albums?|discographie)\s+(?:de|du|d['’]|des)\s*(.+)$/i],
  ['albums', /^(?:ألبومات|البومات|أغاني|اغاني)\s+(.+)$/],
  ['albums', /^(.+?)\s+(?:ألبومات|البومات)$/],
]

/** The relation a query asks for, or `null` for an ordinary search. */
/**
 * What kind of thing each relation's subject must be. The hint that keeps "films by spielberg"
 * on the director and not the town, and lets the side panel ask for the film itself.
 */
export const SUBJECT_KINDS: Record<Relation, string[]> = {
  cast: ['film', 'series'],
  books: ['person'],
  films: ['person'],
  albums: ['person', 'music'],
}

export function detectRelation(raw: string): RelationQuery | null {
  const q = raw.trim().replace(/[?？؟]+$/, '').replace(/\s+/g, ' ')
  if (q.length < 4 || q.length > 80) return null
  for (const [relation, re] of PATTERNS) {
    const m = q.match(re)
    if (!m) continue
    // The subject is kept exactly as typed. A first version stripped a leading article, and
    // "cast of the matrix" became a search for "matrix" — the mathematics, not the film.
    const subject = (m[1] ?? '').trim()
    // A subject has to look like a name: at least two characters, at most eight words.
    if (subject.length < 2 || subject.split(' ').length > 8) continue
    return { relation, subject }
  }
  return null
}
