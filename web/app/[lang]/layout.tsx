import type { Metadata } from 'next'
import type { ReactNode } from 'react'
import { notFound } from 'next/navigation'

import '../fonts.css'
import '../globals.css'
import { dirOf, isLocale, LOCALES } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'
import { readDensity, readTheme, THEME_SCRIPT } from '@/lib/theme'

export function generateStaticParams() {
  return LOCALES.map((lang) => ({ lang }))
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ lang: string }>
}): Promise<Metadata> {
  const { lang } = await params
  if (!isLocale(lang)) return {}
  const t = messages(lang)
  return {
    title: { default: `Xustive — ${t.tagline}`, template: '%s — Xustive' },
    description: t.tagline,
    // Queries must not reach the sites a user clicks through to.
    referrer: 'no-referrer',
  }
}

export default async function LocaleLayout({
  children,
  params,
}: {
  children: ReactNode
  params: Promise<{ lang: string }>
}) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()

  // Both resolved server-side, so the first byte already carries the right theme and direction.
  const [theme, density] = await Promise.all([readTheme(), readDensity()])

  return (
    <html lang={lang} dir={dirOf(lang)} data-theme={theme} data-density={density}>
      <head>
        {/* Preloaded per direction, not both. A reader on an Arabic page never needs the Latin
            file and vice versa, and preloading a font that is not used costs the bandwidth twice
            over — once downloading it, once delaying the one that is.

            `unicode-range` already stops the browser fetching an unused subset, but only after it
            has parsed the stylesheet and laid out enough text to know which characters appear.
            The preload starts the one file that is certainly needed in the same round trip as the
            HTML. */}
        <link
          rel="preload"
          as="font"
          type="font/woff2"
          href={
            dirOf(lang) === 'rtl'
              ? '/fonts/ibm-plex-sans-arabic-400-arabic.woff2'
              : '/fonts/ibm-plex-sans-var-latin.woff2'
          }
          crossOrigin="anonymous"
        />
        {/* Before paint, not in an effect. An effect is what causes the flash. */}
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>{children}</body>
    </html>
  )
}
