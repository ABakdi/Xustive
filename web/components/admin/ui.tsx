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

/** One status dot for the whole console: an icon shape *and* a colour, never colour alone. */
export function Status({
  state,
  label,
  detail,
}: {
  state: 'on' | 'off' | 'warn' | 'critical'
  label: string
  detail?: string
}) {
  const color =
    state === 'on' ? 'var(--viz-good)' : state === 'warn' ? 'var(--viz-warning)' : state === 'critical' ? 'var(--viz-critical)' : 'var(--fg-faint)'
  const glyph = state === 'on' ? '●' : state === 'warn' ? '▲' : state === 'critical' ? '■' : '○'
  return (
    <div className="flex items-center gap-2 rounded border px-3 py-2 text-sm" style={{ borderColor: 'var(--line)', background: 'var(--surface)' }}>
      <span aria-hidden className="text-[10px]" style={{ color }}>
        {glyph}
      </span>
      <span className="sr-only">{state}</span>
      <span className="font-medium">{label}</span>
      {detail && <span style={{ color: 'var(--fg-muted)' }}>{detail}</span>}
    </div>
  )
}

/** A titled block with the one-line hint that says what to do with it. */
export function Section({ title, hint, children, actions }: { title: string; hint?: string; children: React.ReactNode; actions?: React.ReactNode }) {
  return (
    <section className="mb-8">
      <div className="mb-2 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h2 className="m-0 text-base font-semibold">{title}</h2>
          {hint && <p className="m-0 text-xs" style={{ color: 'var(--fg-faint)' }}>{hint}</p>}
        </div>
        {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
      </div>
      {children}
    </section>
  )
}

/** A switch that says what it is doing: idle, saving, saved, or why it failed. */
export function Toggle({
  label,
  checked,
  onChange,
  disabled,
  hint,
}: {
  label: string
  checked: boolean
  onChange: (next: boolean) => Promise<void> | void
  disabled?: boolean
  hint?: string
}) {
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  return (
    <label className="flex items-center gap-3 text-sm">
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled || busy}
        onClick={async () => {
          setBusy(true)
          setNote(null)
          try {
            await onChange(!checked)
          } catch (e) {
            setNote((e as Error).message)
          } finally {
            setBusy(false)
          }
        }}
        className="relative h-5 w-9 shrink-0 rounded-full transition-colors"
        style={{ background: checked ? 'var(--accent)' : 'var(--line-strong)', opacity: disabled ? 0.5 : 1 }}
      >
        <span className="absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform" style={{ insetInlineStart: 2, transform: checked ? 'translateX(16px)' : 'none' }} />
      </button>
      <span>
        <span className="font-medium">{label}</span>
        {hint && <span className="ms-2 text-xs" style={{ color: 'var(--fg-faint)' }}>{hint}</span>}
        {busy && <span className="ms-2 text-xs" style={{ color: 'var(--fg-faint)' }}>saving…</span>}
        {note && <span className="ms-2 text-xs" style={{ color: 'var(--viz-critical)' }}>{note}</span>}
      </span>
    </label>
  )
}

/** A quiet action button; `danger` for the ones that delete. */
export function Action({ children, onClick, danger, disabled, busy }: { children: React.ReactNode; onClick: () => void | Promise<void>; danger?: boolean; disabled?: boolean; busy?: boolean }) {
  return (
    <button
      type="button"
      className={`chip cursor-pointer ${danger ? '' : 'chip-active'}`}
      disabled={disabled || busy}
      onClick={() => void onClick()}
      style={danger ? { color: 'var(--viz-critical)' } : undefined}
    >
      {busy ? '…' : children}
    </button>
  )
}
