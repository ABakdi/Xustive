import { notFound } from 'next/navigation'

import { LangSwitcher } from '@/components/layout/LangSwitcher'
import { ThemeToggle } from '@/components/layout/ThemeToggle'
import { Wordmark } from '@/components/layout/Wordmark'
import { ImageOcr } from '@/components/tools/ImageOcr'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'
import { readTheme } from '@/lib/theme'

/**
 * The standalone image-to-text tool, and the landing spot for "search by image".
 *
 * A server component wrapping the one client island ([[ImageOcr]]) — the page shell, header and
 * copy are static HTML, so the tool is described and reachable even before the bundle loads. The
 * interactive part (file handling, OCR, the editable result) is the island.
 */
export default async function OcrToolPage({
  params,
}: {
  params: Promise<{ lang: string }>
}) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t = messages(lang)
  const theme = await readTheme()

  return (
    <>
      <div className="flex items-center justify-between gap-1 px-5 py-4">
        {/* The wordmark is itself the link home — wrapping it in another <a> nested two anchors
            and broke hydration on every visit. */}
        <Wordmark lang={lang} />
        <div className="flex items-center gap-1">
          <LangSwitcher current={lang} label={t.language} />
          <ThemeToggle
            current={theme}
            labels={{ system: t.themeSystem, light: t.themeLight, dark: t.themeDark }}
          />
        </div>
      </div>

      <main className="mx-auto max-w-xl px-6 py-6">
        <h1 className="mb-1 text-2xl font-semibold">{t.ocrTitle}</h1>
        <p className="mb-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
          {t.ocrIntro}
        </p>

        <ImageOcr lang={lang} t={t} />
      </main>
    </>
  )
}
