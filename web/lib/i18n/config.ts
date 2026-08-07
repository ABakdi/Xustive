/** The four interface languages. The first path segment is always one of these. */
export const LOCALES = ['ar', 'ary', 'fr', 'en'] as const
export type Locale = (typeof LOCALES)[number]

export const DEFAULT_LOCALE: Locale = 'ar'

/** Arabic and Darija are RTL. Set on <html> server-side, never detected client-side. */
export const RTL: readonly Locale[] = ['ar', 'ary']

export function dirOf(locale: Locale): 'rtl' | 'ltr' {
  return RTL.includes(locale) ? 'rtl' : 'ltr'
}

export function isLocale(value: string): value is Locale {
  return (LOCALES as readonly string[]).includes(value)
}

/**
 * Pick a locale from an Accept-Language header.
 *
 * Darija is matched before Arabic because `ary` starts with `ar` and a naive prefix match would
 * swallow it — someone who asked for Darija would silently get Arabic.
 */
export function negotiate(header: string | null): Locale {
  if (!header) return DEFAULT_LOCALE
  const tags = header
    .split(',')
    .map((part) => {
      const [tag, q] = part.trim().split(';q=')
      return { tag: (tag ?? '').toLowerCase(), q: q ? Number(q) : 1 }
    })
    .sort((a, b) => b.q - a.q)

  for (const { tag } of tags) {
    if (tag.startsWith('ary')) return 'ary'
    if (tag.startsWith('ar')) return 'ar'
    if (tag.startsWith('fr')) return 'fr'
    if (tag.startsWith('en')) return 'en'
  }
  return DEFAULT_LOCALE
}
