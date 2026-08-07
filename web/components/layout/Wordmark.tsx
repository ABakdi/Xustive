import Link from 'next/link'

/**
 * The wordmark is always Latin and always LTR, in every locale.
 *
 * A brand name is not translated and not mirrored — it is the one string on the page that must
 * look identical to an Arabic and a French reader.
 */
export function Wordmark({ lang, size = 'lg' }: { lang: string; size?: 'lg' | 'sm' }) {
  return (
    <Link
      href={`/${lang}`}
      dir="ltr"
      className={
        size === 'lg'
          ? 'text-[2rem] font-semibold tracking-[0.22em] no-underline'
          : 'text-lg font-semibold tracking-[0.18em] no-underline'
      }
      style={{ color: 'var(--ink)' }}
    >
      XUSTIVE
    </Link>
  )
}
