import Link from 'next/link'
import { notFound } from 'next/navigation'

import { LangSwitcher } from '@/components/layout/LangSwitcher'
import { ThemeToggle } from '@/components/layout/ThemeToggle'
import { Wordmark } from '@/components/layout/Wordmark'
import { ReverseImage } from '@/components/search/ReverseImage'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'
import { readTheme } from '@/lib/theme'

/**
 * Reverse image search — the page ([[Milestone 10 - Reverse Image Search]] T04).
 *
 * A server shell around one client island: the header, the title and the privacy line are
 * static HTML; the drop zone, the query image and the grids are the island. There is no URL for
 * a search — the picture is a POST body — except the `?u=&s=` form, which names a picture
 * already on the Images tab by its signed thumbnail URL and searches for it without an upload.
 */
export default async function ReverseImagePage({
  params,
  searchParams,
}: {
  params: Promise<{ lang: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t = messages(lang)
  const tt = t as unknown as Record<string, string>
  const theme = await readTheme()
  const sp = await searchParams
  const u = typeof sp.u === 'string' ? sp.u : ''
  const s = typeof sp.s === 'string' ? sp.s : ''

  return (
    <>
      <div className="flex items-center justify-between gap-1 px-5 py-4">
        <Link href={`/${lang}`} aria-label="Xustive">
          <Wordmark lang={lang} />
        </Link>
        <div className="flex items-center gap-1">
          <LangSwitcher current={lang} label={t.language} />
          <ThemeToggle
            current={theme}
            labels={{ system: t.themeSystem, light: t.themeLight, dark: t.themeDark }}
          />
        </div>
      </div>

      <main className="mx-auto max-w-5xl px-6 py-6">
        <h1 className="mb-1 text-2xl font-semibold">{tt.reverseTitle}</h1>
        <p className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
          {tt.reverseIntro}
        </p>
        <ReverseImage lang={lang} t={t} byUrl={u && s ? { u, s } : undefined} />
      </main>
    </>
  )
}
