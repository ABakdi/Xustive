import { notFound } from 'next/navigation'

import { LangSwitcher } from '@/components/layout/LangSwitcher'
import { DensityToggle } from '@/components/layout/DensityToggle'
import { ThemeToggle } from '@/components/layout/ThemeToggle'
import { Wordmark } from '@/components/layout/Wordmark'
import { Toggle } from '@/components/ui/Toggle'
import { tools } from '@/lib/api'
import { isLocale } from '@/lib/i18n/config'
import { messages, type Messages } from '@/lib/i18n/messages'
import { setToolEnabled } from '@/lib/prefs'
import { readDensity, readTheme } from '@/lib/theme'
import { readDisabledTools } from '@/lib/tools'

export const metadata = { title: 'Xustive' }

/**
 * Preferences.
 *
 * Exists because an opt-out with no way back is not a preference, it is a trap. Dismissing a tool
 * from its card is the fast path; this is the only way to undo it.
 *
 * Entirely server-rendered, toggles included — each is a form posting to a Server Action, so the
 * page works with JavaScript disabled like the rest of the search path.
 */
export default async function Settings({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t: Messages = messages(lang)
  const [theme, density] = await Promise.all([readTheme(), readDensity()])

  const [inventory, disabled] = await Promise.all([tools(), readDisabledTools()])

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

      <main className="mx-auto max-w-xl px-[var(--pad)] pb-20">
        <div className="mb-8">
          <Wordmark lang={lang} />
        </div>

        <h1 className="m-0 text-lg" style={{ fontWeight: 550 }}>
          {t.settings}
        </h1>

        <h2 className="mt-8 mb-1 text-sm" style={{ fontWeight: 550 }}>
          {t.toolsHeading}
        </h2>
        <p className="mt-0 mb-4 text-xs" style={{ color: 'var(--fg-muted)' }}>
          {t.toolsNote}
        </p>

        {/* An empty inventory means the API is unreachable. Saying so beats rendering an empty
            box that looks like "you have no tools". */}
        {inventory.length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--fg-muted)' }}>
            {t.errorTitle}
          </p>
        ) : (
          <ul className="m-0 list-none p-0">
            {inventory.map((tool) => {
              const enabled = !disabled.has(tool.id)
              const name = (t as Record<string, string>)[tool.id] ?? tool.id
              async function toggle() {
                'use server'
                await setToolEnabled(tool.id, !enabled)
              }
              return (
                <li
                  key={tool.id}
                  className="flex items-center justify-between gap-4 border-b py-2.5"
                  style={{ borderColor: 'var(--line)' }}
                >
                  <span className="text-sm" id={`tool-${tool.id}`}>
                    {/* Falls back to the identifier when a tool has no translated name yet — a
                        row labelled with its id is legible; a blank row is not. */}
                    {name}
                    <span className="ms-2 text-xs" style={{ color: 'var(--fg-faint)' }}>
                      <bdi>!{tool.keyword}</bdi>
                    </span>
                  </span>
                  <form action={toggle}>
                    <Toggle
                      on={enabled}
                      onLabel={t.on}
                      offLabel={t.off}
                      describedBy={`tool-${tool.id}`}
                      accessibleLabel={`${enabled ? t.disable : t.enable}: ${name}`}
                    />
                  </form>
                </li>
              )
            })}
          </ul>
        )}
      </main>
    </>
  )
}
