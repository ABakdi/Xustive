import type { Locale } from './config'

/**
 * Locale-correct formatting.
 *
 * Three things that a naive implementation gets wrong in ways a native reader notices immediately.
 */

/**
 * The numeral system, chosen explicitly rather than taken from the locale default.
 *
 * `Intl` defaults Arabic to Eastern Arabic digits (٤٥). Algerian print, signage and official
 * documents overwhelmingly use Western digits, so the locale default would be wrong here — and
 * wrong in a way that reads as the site being built for a different country.
 *
 * Still an open question pending a native-speaker read; keeping it in one place is what makes
 * changing the answer a one-line edit rather than an audit.
 */
const NUMBERING = 'latn'

export function formatNumber(locale: Locale, value: number): string {
  return new Intl.NumberFormat(intlLocale(locale), {
    numberingSystem: NUMBERING,
  }).format(value)
}

export function formatDate(locale: Locale, unixSeconds: number): string {
  return new Intl.DateTimeFormat(intlLocale(locale), {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    numberingSystem: NUMBERING,
  }).format(new Date(unixSeconds * 1000))
}

/**
 * Pluralise a count.
 *
 * `Intl.PluralRules`, not `n === 1 ? singular : plural`. **Arabic has six plural categories** —
 * zero, one, two, few, many, other — and the ternary is wrong for four of them. To a native
 * reader that is not a rounding error; it reads as the interface being written by someone who
 * does not speak the language.
 *
 * A missing category falls back to `other`, which is the category every locale defines.
 */
export function plural(
  locale: Locale,
  count: number,
  forms: Partial<Record<Intl.LDMLPluralRule, string>>,
): string {
  const rule = new Intl.PluralRules(intlLocale(locale)).select(count)
  return forms[rule] ?? forms.other ?? ''
}

/**
 * Darija has no CLDR data, so formatting borrows Arabic.
 *
 * Passing `ary` to `Intl` yields the root locale and loses Arabic month names entirely — the
 * failure is silent and looks like a formatting bug rather than a missing locale.
 */
function intlLocale(locale: Locale): string {
  return locale === 'ary' ? 'ar-DZ' : locale === 'ar' ? 'ar-DZ' : locale
}
