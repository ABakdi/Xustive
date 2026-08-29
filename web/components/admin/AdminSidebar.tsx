'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'
import { useEffect, useState } from 'react'

import { getIntegrations, getMedia, getQueue, getStatus } from '@/lib/admin'

/** Sidebar sections, in order. Real URLs, one per section — bookmarkable and each loads on its own. */
const SECTIONS: { group: string; label: string; href: string }[] = [
  { group: '', label: 'Overview', href: '/admin' },
  { group: 'CRAWLER', label: 'Live', href: '/admin/live' },
  { group: 'CRAWLER', label: 'Documents', href: '/admin/documents' },
  { group: 'CRAWLER', label: 'Sources', href: '/admin/sources' },
  { group: 'CRAWLER', label: 'Source health', href: '/admin/sources/health' },
  { group: 'CRAWLER', label: 'Discovery yield', href: '/admin/discovery' },
  { group: 'CRAWLER', label: 'Weak coverage', href: '/admin/weak-coverage' },
  { group: 'SEARCH', label: 'Evaluation', href: '/admin/evaluation' },
  { group: 'SEARCH', label: 'Integrations', href: '/admin/integrations' },
  { group: 'SEARCH', label: 'Searches & hits', href: '/admin/searches' },
  { group: 'SEARCH', label: 'Anonymous signals', href: '/admin/interaction' },
  { group: 'SEARCH', label: 'Media & voice', href: '/admin/media' },
  { group: 'SYSTEM', label: 'Compute', href: '/admin/compute' },
  { group: 'SYSTEM', label: 'Configuration', href: '/admin/config' },
  { group: 'SYSTEM', label: 'Index queue', href: '/admin/queue' },
  { group: 'SYSTEM', label: 'Maintenance', href: '/admin/maintenance' },
]

/** A dot per page whose subject has a state worth a glance (M12-T04.2). */
function useDots() {
  const [dots, setDots] = useState<Record<string, 'on' | 'warn' | 'critical' | 'off'>>({})
  useEffect(() => {
    let alive = true
    const tick = async () => {
      const next: Record<string, 'on' | 'warn' | 'critical' | 'off'> = {}
      try {
        const st = await getStatus()
        next['/admin/live'] = st.unavailable ? 'off' : st.paused ? 'warn' : 'on'
      } catch {
        next['/admin/live'] = 'off'
      }
      try {
        const q = await getQueue()
        const pct = q.capacity?.redis_pct
        next['/admin/queue'] = pct == null ? 'on' : pct >= 85 ? 'critical' : pct >= 80 ? 'warn' : 'on'
        if ((q.dead_count ?? 0) > 0 && next['/admin/queue'] === 'on') next['/admin/queue'] = 'warn'
      } catch {
        next['/admin/queue'] = 'off'
      }
      try {
        const i = await getIntegrations()
        next['/admin/integrations'] = i.federation?.enabled ? (i.federation.reachable_from_api ? 'on' : 'warn') : 'off'
      } catch {
        next['/admin/integrations'] = 'off'
      }
      try {
        const m = await getMedia()
        const up = (m.ocr?.healthy ?? true) && (!m.stt?.enabled || m.stt.healthy) && (!m.vector?.enabled || m.vector.qdrant_reachable)
        next['/admin/media'] = up ? 'on' : 'warn'
      } catch {
        next['/admin/media'] = 'off'
      }
      if (alive) setDots(next)
    }
    void tick()
    const id = setInterval(tick, 60_000)
    return () => {
      alive = false
      clearInterval(id)
    }
  }, [])
  return dots
}

export function AdminSidebar() {
  const dots = useDots()
  const path = usePathname()
  let lastGroup: string | null = null
  return (
    <nav className="scroll-x bleed -mt-1 flex gap-1 border-b pb-1 md:sticky md:top-4 md:mt-0 md:flex-col md:gap-px md:overflow-visible md:border-0 md:pb-0 md:[margin-inline:0] md:[padding-inline:0] md:self-start"
      style={{ borderColor: 'var(--line)' }}
    >
      {SECTIONS.map((s) => {
        // /admin must match exactly; the others match themselves (not their string-prefixes).
        const active = s.href === '/admin' ? path === '/admin' : path === s.href
        const header =
          s.group && s.group !== lastGroup ? (
            <div
              key={`g-${s.group}`}
              className="hidden px-2 text-[0.6875rem] tracking-[0.08em] md:mt-4 md:block"
              style={{ color: 'var(--fg-faint)' }}
            >
              {s.group}
            </div>
          ) : null
        lastGroup = s.group || lastGroup
        return (
          <div key={s.href} className="shrink-0">
            {header}
            <Link
              href={s.href}
              className="block whitespace-nowrap border-b-2 border-l-0 px-2.5 py-1.5 text-sm no-underline md:border-b-0 md:border-l-2"
              style={{
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
                borderLeftColor: active ? 'var(--accent)' : 'transparent',
                borderBottomColor: active ? 'var(--accent)' : 'transparent',
                fontWeight: active ? 600 : 400,
              }}
            >
              {s.label}
            {dots[s.href] && (
              <span aria-hidden className="ms-2 inline-block h-1.5 w-1.5 rounded-full align-middle" style={{ background: dots[s.href] === 'on' ? 'var(--viz-good)' : dots[s.href] === 'warn' ? 'var(--viz-warning)' : dots[s.href] === 'critical' ? 'var(--viz-critical)' : 'var(--fg-faint)' }} />
            )}
            </Link>
          </div>
        )
      })}
    </nav>
  )
}
