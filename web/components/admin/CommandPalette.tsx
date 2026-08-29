'use client'

import { useRouter } from 'next/navigation'
import { useEffect, useMemo, useRef, useState } from 'react'

import { replayDlq, setCrawlPaused, setIntegration, setLogLevel } from '@/lib/admin'

/**
 * ⌘K / Ctrl-K (M12-T04.1): every page and the actions an operator reaches for, two keystrokes
 * from anywhere. `/` opens it too when nothing has focus; Esc closes. Type to filter; ↑↓ to
 * move; Enter to go or do. A destructive action (replay the dead letters) asks for a second
 * Enter. Prefix with `?` to jump to the documents page with a search.
 */
type Item = { id: string; label: string; hint?: string; run: () => Promise<void> | void; danger?: boolean }

const PAGES: [string, string, string][] = [
  ['Overview', '/admin', 'is anything wrong, and since when'],
  ['Live', '/admin/live', 'the crawler as it runs'],
  ['Documents', '/admin/documents', 'what has been collected'],
  ['Sources', '/admin/sources', 'the seed list'],
  ['Source health', '/admin/sources/health', 'per-source quality and policy'],
  ['Discovery yield', '/admin/discovery', 'per-channel funnel'],
  ['Weak coverage', '/admin/weak-coverage', 'gaps to fill'],
  ['Evaluation', '/admin/evaluation', 'the golden set over time'],
  ['Integrations', '/admin/integrations', 'federation, external models, budgets'],
  ['Searches & hits', '/admin/searches', 'what people searched and opened'],
  ['Anonymous signals', '/admin/interaction', 'k-anonymous counters'],
  ['Media & voice', '/admin/media', 'OCR, image search, STT'],
  ['Compute', '/admin/compute', 'device, ranking weights, summaries, logging'],
  ['Configuration', '/admin/config', 'the effective config'],
  ['Index queue', '/admin/queue', 'backlog & dead letters'],
  ['Maintenance', '/admin/maintenance', 'takedowns'],
]

export function CommandPalette() {
  const router = useRouter()
  const [open, setOpen] = useState(false)
  const [q, setQ] = useState('')
  const [cursor, setCursor] = useState(0)
  const [armed, setArmed] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement | null)?.closest('input, textarea, select, [contenteditable]')
      if ((e.key === 'k' && (e.metaKey || e.ctrlKey)) || (e.key === '/' && !typing)) {
        e.preventDefault()
        setOpen(true)
        setQ('')
        setCursor(0)
        setArmed(null)
        setNote(null)
        setTimeout(() => input.current?.focus(), 0)
      } else if (e.key === 'Escape' && open) {
        setOpen(false)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  const items: Item[] = useMemo(() => {
    const go = (href: string) => () => {
      router.push(href)
      setOpen(false)
    }
    const pages: Item[] = PAGES.map(([label, href, hint]) => ({ id: href, label, hint, run: go(href) }))
    const actions: Item[] = [
      { id: 'pause', label: 'Pause crawler', hint: 'stops new fetches; in-flight ones finish', run: async () => { await setCrawlPaused(true); setNote('Crawler paused.') } },
      { id: 'resume', label: 'Resume crawler', run: async () => { await setCrawlPaused(false); setNote('Crawler resumed.') } },
      { id: 'fed-on', label: 'Federation on', hint: 'query-time SearXNG', run: async () => { await setIntegration('federation', true); setNote('Federation on.') } },
      { id: 'fed-off', label: 'Federation off', run: async () => { await setIntegration('federation', false); setNote('Federation off.') } },
      { id: 'debug', label: 'Raise logs to debug (15 min)', hint: 'auto-reverts', run: async () => { await setLogLevel('debug'); setNote('Logs at debug for 15 minutes.') } },
      { id: 'dlq', label: 'Replay all dead letters', hint: 'fix the cause first', danger: true, run: async () => { const r = await replayDlq(); setNote(`Replayed ${(r as { replayed?: number }).replayed ?? ''} dead letters.`) } },
    ]
    const jump: Item[] = q.startsWith('?') && q.length > 1
      ? [{ id: 'jump', label: `Search documents for “${q.slice(1).trim()}”`, run: go(`/admin/documents?q=${encodeURIComponent(q.slice(1).trim())}`) }]
      : []
    const needle = q.startsWith('?') ? '' : q.trim().toLowerCase()
    const all = [...jump, ...pages, ...actions]
    return needle ? all.filter((i) => `${i.label} ${i.hint ?? ''}`.toLowerCase().includes(needle)) : all
  }, [q, router])

  useEffect(() => setCursor(0), [q])
  if (!open) return null

  const choose = async (it: Item) => {
    if (it.danger && armed !== it.id) {
      setArmed(it.id)
      setNote('Press Enter again to confirm.')
      return
    }
    try {
      await it.run()
    } catch (e) {
      setNote((e as Error).message)
    }
    setArmed(null)
  }

  return (
    <div className="fixed inset-0 flex items-start justify-center p-6" style={{ background: 'rgba(0,0,0,0.45)', zIndex: 'var(--z-modal)' as unknown as number }} onClick={() => setOpen(false)} role="dialog" aria-label="Command palette">
      <div className="mt-[10dvh] w-full max-w-lg rounded-lg border" style={{ borderColor: 'var(--line-strong)', background: 'var(--bg)', boxShadow: 'var(--shadow-pop)' }} onClick={(e) => e.stopPropagation()}>
        <input
          ref={input}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setCursor((c) => Math.min(items.length - 1, c + 1)) }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setCursor((c) => Math.max(0, c - 1)) }
            else if (e.key === 'Enter') { e.preventDefault(); const it = items[cursor]; if (it) void choose(it) }
          }}
          placeholder="Go to a page, run an action, or ?query to search documents"
          className="w-full border-b bg-transparent px-4 py-3 text-sm outline-none"
          style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
          aria-label="Command"
        />
        <ul className="m-0 max-h-[50dvh] list-none overflow-y-auto p-1">
          {items.map((it, i) => (
            <li key={it.id}>
              <button
                type="button"
                className="flex w-full items-baseline justify-between gap-3 rounded px-3 py-2 text-start text-sm"
                style={{ background: i === cursor ? 'var(--accent-wash)' : 'transparent', color: it.danger ? 'var(--viz-critical)' : 'var(--fg)' }}
                onMouseEnter={() => setCursor(i)}
                onClick={() => void choose(it)}
              >
                <span>{it.label}{armed === it.id ? ' — confirm?' : ''}</span>
                {it.hint && <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>{it.hint}</span>}
              </button>
            </li>
          ))}
          {items.length === 0 && <li className="px-3 py-2 text-sm" style={{ color: 'var(--fg-faint)' }}>Nothing matches.</li>}
        </ul>
        <div className="flex items-center justify-between border-t px-3 py-1.5 text-xs" style={{ borderColor: 'var(--line)', color: 'var(--fg-faint)' }}>
          <span>{note ?? '↑↓ move · Enter go · Esc close'}</span>
          <span>⌘K</span>
        </div>
      </div>
    </div>
  )
}
