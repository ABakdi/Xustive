'use client'

import Link from 'next/link'

import { getStatus } from '@/lib/admin'
import { PageHead, usePoll } from '@/components/admin/ui'

function Tile({ n, label }: { n: number | string; label: string }) {
  return (
    <div className="flex flex-col gap-0.5 border px-4 py-3" style={{ borderColor: 'var(--line)', minInlineSize: '120px' }}>
      <span className="text-2xl font-medium tabular-nums">{n}</span>
      <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>
        {label}
      </span>
    </div>
  )
}

const LINKS: [string, string, string][] = [
  ['Live', '/admin/live', 'the crawler as it runs'],
  ['Documents', '/admin/documents', 'what has been collected'],
  ['Sources', '/admin/sources', 'the seed list'],
  ['Source health', '/admin/sources/health', 'per-source quality'],
  ['Discovery yield', '/admin/discovery', 'per-channel funnel'],
  ['Weak coverage', '/admin/weak-coverage', 'gaps to fill'],
  ['Compute', '/admin/compute', 'device & politeness'],
]

export default function OverviewPage() {
  const { data: s, error } = usePoll(getStatus, 5_000)
  return (
    <>
      <PageHead title="Overview">
        Is anything wrong, in one screen. Every number that could be <em>unknown</em> says so rather
        than showing zero, because a zero and an unreachable dependency look identical.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          Could not reach the API: {error}
        </p>
      ) : null}

      <div className="mb-8 flex flex-wrap gap-3">
        <Tile n={s ? (s.unavailable ? 'unknown' : s.state) : '…'} label="crawler state" />
        <Tile n={s?.indexed ?? 0} label="indexed (session)" />
        <Tile n={s?.waiting ?? 0} label="frontier waiting" />
        <Tile n={s?.inflight ?? 0} label="in flight" />
        <Tile n={s?.failed ?? 0} label="failed" />
      </div>

      <h2 className="mb-3 text-lg font-semibold">Sections</h2>
      <div className="grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))' }}>
        {LINKS.map(([label, href, hint]) => (
          <Link
            key={href}
            href={href}
            className="block border px-4 py-3 no-underline"
            style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
          >
            <div className="font-medium">{label}</div>
            <div className="text-sm" style={{ color: 'var(--fg-muted)' }}>
              {hint}
            </div>
          </Link>
        ))}
      </div>
    </>
  )
}
