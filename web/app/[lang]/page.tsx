import { notFound } from 'next/navigation'

import { SearchBox } from '@/components/search/SearchBox'
import { Wordmark } from '@/components/layout/Wordmark'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'

export default async function Home({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t = messages(lang)

  return (
    <main className="mx-auto flex min-h-dvh max-w-2xl flex-col items-center justify-center px-6">
      <div className="w-full -translate-y-12">
        <div className="mb-2 text-center">
          <Wordmark lang={lang} />
        </div>
        <p className="mb-8 text-center text-sm" style={{ color: 'var(--ink-muted)' }}>
          {t.tagline}
        </p>

        <SearchBox lang={lang} t={t} />

        <p className="mt-6 text-center text-sm" style={{ color: 'var(--ink-muted)' }}>
          🔒 {t.privacyLine}
        </p>
      </div>
    </main>
  )
}
