'use client'

import Link from 'next/link'
import { usePathname } from 'next/navigation'

/** Sidebar sections, in order. Real URLs, one per section — bookmarkable and each loads on its own. */
const SECTIONS: { group: string; label: string; href: string }[] = [
  { group: '', label: 'Overview', href: '/admin' },
  { group: 'CRAWLER', label: 'Live', href: '/admin/live' },
  { group: 'CRAWLER', label: 'Documents', href: '/admin/documents' },
  { group: 'CRAWLER', label: 'Sources', href: '/admin/sources' },
  { group: 'CRAWLER', label: 'Source health', href: '/admin/sources/health' },
  { group: 'CRAWLER', label: 'Discovery yield', href: '/admin/discovery' },
  { group: 'CRAWLER', label: 'Weak coverage', href: '/admin/weak-coverage' },
  { group: 'SYSTEM', label: 'Compute', href: '/admin/compute' },
]

export function AdminSidebar() {
  const path = usePathname()
  let lastGroup: string | null = null
  return (
    <nav className="flex flex-col gap-px sticky top-4 self-start">
      {SECTIONS.map((s) => {
        // /admin must match exactly; the others match themselves (not their string-prefixes).
        const active = s.href === '/admin' ? path === '/admin' : path === s.href
        const header =
          s.group && s.group !== lastGroup ? (
            <div
              key={`g-${s.group}`}
              className="mt-4 px-2 text-[0.6875rem] tracking-[0.08em]"
              style={{ color: 'var(--fg-faint)' }}
            >
              {s.group}
            </div>
          ) : null
        lastGroup = s.group || lastGroup
        return (
          <div key={s.href}>
            {header}
            <Link
              href={s.href}
              className="block border-l-2 px-2.5 py-1.5 text-sm no-underline"
              style={{
                color: active ? 'var(--fg)' : 'var(--fg-muted)',
                borderLeftColor: active ? 'var(--accent)' : 'transparent',
                fontWeight: active ? 600 : 400,
              }}
            >
              {s.label}
            </Link>
          </div>
        )
      })}
    </nav>
  )
}
