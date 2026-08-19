'use client'

import { useEffect, useRef, useState } from 'react'

/** A page heading and its explanatory lede. */
export function PageHead({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <>
      <h1 className="mb-2 text-2xl font-semibold tracking-tight">{title}</h1>
      {children ? (
        <p className="mb-6 text-[0.95rem]" style={{ color: 'var(--fg-muted)' }}>
          {children}
        </p>
      ) : null}
    </>
  )
}

/** A muted status line under a table ("12 rows. Refreshing every 10s."). */
export function StatusLine({ children }: { children: React.ReactNode }) {
  return (
    <p className="mb-3 text-sm" style={{ color: 'var(--fg-muted)' }}>
      {children}
    </p>
  )
}

/** A bordered, horizontally-scrollable table wrapper. Wide operator tables scroll on their own
 * axis rather than pushing the page sideways. */
export function Table({ head, children }: { head: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr style={{ color: 'var(--fg-muted)' }}>{head}</tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  )
}

export function Th({ children, num }: { children?: React.ReactNode; num?: boolean }) {
  return (
    <th
      className={`border-b px-3 py-2 font-medium ${num ? 'text-right' : 'text-left'}`}
      style={{ borderColor: 'var(--line)' }}
    >
      {children}
    </th>
  )
}

export function Td({
  children,
  num,
  warn,
  title,
}: {
  children?: React.ReactNode
  num?: boolean
  warn?: boolean
  title?: string
}) {
  return (
    <td
      title={title}
      className={`border-b px-3 py-2 ${num ? 'text-right tabular-nums' : 'text-left'} ${warn ? 'font-semibold' : ''}`}
      style={{ borderColor: 'var(--line)', color: warn ? 'var(--warn)' : undefined }}
    >
      {children}
    </td>
  )
}

/** A percentage cell, or an em-dash when the value is unknown. Amber (`warn`) when outside its band. */
export function pct(v: number | null | undefined): string {
  return v == null ? '—' : `${Math.round(v * 100)}%`
}

/** Poll `fn` on mount and every `ms`, aborting in-flight requests on unmount. */
export function usePoll<T>(fn: (signal: AbortSignal) => Promise<T>, ms: number) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  // Keep the latest `fn` without making it a dependency that restarts the interval each render.
  const fnRef = useRef(fn)
  fnRef.current = fn

  useEffect(() => {
    let alive = true
    const controller = new AbortController()
    async function tick() {
      try {
        const d = await fnRef.current(controller.signal)
        if (alive) {
          setData(d)
          setError(null)
        }
      } catch (e) {
        if (alive && (e as Error).name !== 'AbortError') setError((e as Error).message)
      } finally {
        if (alive) setLoading(false)
      }
    }
    tick()
    const id = setInterval(tick, ms)
    return () => {
      alive = false
      controller.abort()
      clearInterval(id)
    }
  }, [ms])

  return { data, error, loading }
}
