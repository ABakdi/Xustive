import Link from 'next/link'

import type { Messages } from '@/lib/i18n/messages'

/**
 * The vertical tabs.
 *
 * A vertical is a saved filter over the one index, not a separate search — "News" is web documents
 * with a real date. The active vertical lives in the URL (`?v=news`) so a tab is a shareable link and
 * the back button works. A pure server component: it is a row of links, so it ships no JavaScript.
 *
 * Social and Short videos arrive with the connectors that feed them — a tab for an empty vertical
 * would be a dead end. Images and Videos landed with M9 and name themselves when empty.
 */
export function Verticals({
  lang,
  q,
  active,
  t,
}: {
  lang: string
  q: string
  active?: string
  t: Messages
}) {
  const tabs: { id: string; label: string }[] = [
    { id: 'all', label: t.verticalAll },
    { id: 'news', label: t.verticalNews },
    { id: 'files', label: t.verticalFiles },
    // Images and Videos (M9). Shown always, against the "as content arrives" rule above: the
    // operator asked for them, and the empty state names the vertical so an empty tab is honest
    // rather than indistinguishable from a broken one.
    { id: 'images', label: t.verticalImages },
    { id: 'videos', label: t.verticalVideos },
  ]
  const current = tabs.some((tab) => tab.id === active) ? active! : 'all'

  const href = (id: string) => {
    const p = new URLSearchParams({ q })
    if (id !== 'all') p.set('v', id)
    return `/${lang}/search?${p.toString()}`
  }

  return (
    <nav
      className="scroll-x bleed mb-5 flex gap-1 border-b text-sm"
      style={{ borderColor: 'var(--line)' }}
      aria-label={t.verticalAll}
    >
      {tabs.map((tab) => {
        const on = tab.id === current
        return (
          <Link
            key={tab.id}
            href={href(tab.id)}
            className="-mb-px shrink-0 whitespace-nowrap border-b-2 px-3 py-1.5 no-underline"
            aria-current={on ? 'page' : undefined}
            style={{
              borderBottomColor: on ? 'var(--accent)' : 'transparent',
              color: on ? 'var(--fg)' : 'var(--fg-muted)',
              fontWeight: on ? 600 : 400,
            }}
          >
            {tab.label}
          </Link>
        )
      })}
    </nav>
  )
}
