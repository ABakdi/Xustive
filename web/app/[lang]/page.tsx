import { notFound } from 'next/navigation'

import { LangSwitcher } from '@/components/layout/LangSwitcher'
import { SearchBox } from '@/components/search/SearchBox'
import { DensityToggle } from '@/components/layout/DensityToggle'
import { ThemeToggle } from '@/components/layout/ThemeToggle'
import { Wordmark } from '@/components/layout/Wordmark'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'
import { readDensity, readTheme } from '@/lib/theme'

export default async function Home({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t = messages(lang)
  const [theme, density] = await Promise.all([readTheme(), readDensity()])

  return (
    <>
      <div className="flex items-center justify-end gap-1 px-5 py-4">
        <LangSwitcher current={lang} label={t.language} />
        <ThemeToggle
          current={theme}
          labels={{ system: t.themeSystem, light: t.themeLight, dark: t.themeDark }}
        />
<DensityToggle
              current={density}
              labels={{ comfortable: t.densityComfortable, compact: t.densityCompact }}
            />
      </div>

      <main className="mx-auto flex max-w-xl flex-col justify-center px-6" style={{ minBlockSize: '76dvh' }}>
        <div className="w-full">
          <div className="mb-2.5">
            <Wordmark lang={lang} />
          </div>
          <p className="mb-9 text-sm" style={{ color: 'var(--fg-muted)' }}>
            {t.tagline}
          </p>

          <SearchBox lang={lang} t={t} />

          <p className="mt-5 text-xs" style={{ color: 'var(--fg-faint)' }}>
            {t.privacyLine}
          </p>
        </div>
      </main>
    </>
  )
}
