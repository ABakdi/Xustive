'use client'

import { useEffect, useMemo, useRef, useState } from 'react'

import { evaluate, normalise, render } from '@/lib/calc'
import type { Messages } from '@/lib/i18n/messages'

import { CopyButton } from './CopyButton'

/**
 * The calculator, as a tool rather than an answer (M13 follow-up): the expression the reader
 * typed is loaded into a working calculator — keypad, keyboard, live result — so the next
 * calculation does not need another search. Evaluated locally (`lib/calc`), the same grammar
 * the API's tool reads, so the number shown for the query matches what the API answered.
 */
const KEYS: string[][] = [
  ['(', ')', '%', '⌫'],
  ['7', '8', '9', '÷'],
  ['4', '5', '6', '×'],
  ['1', '2', '3', '−'],
  ['0', '.', '^', '+'],
  ['√', 'C', '='],
]

export function Calculator({ initial, t }: { initial: string; t: Messages }) {
  const [expr, setExpr] = useState(initial)
  const [committed, setCommitted] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => setExpr(initial), [initial])

  const live = useMemo(() => {
    const v = evaluate(expr)
    return v === null ? null : render(v)
  }, [expr])

  const press = (k: string) => {
    setCommitted(null)
    if (k === 'C') return setExpr('')
    if (k === '⌫') return setExpr((e) => e.slice(0, -1))
    if (k === '=') {
      if (live !== null) {
        setCommitted(live)
        setExpr(live)
      }
      return
    }
    if (k === '√') return setExpr((e) => e + '√(')
    setExpr((e) => e + k)
    input.current?.focus()
  }

  return (
    <div className="mt-2 max-w-sm">
      <input
        ref={input}
        value={expr}
        onChange={(e) => {
          setCommitted(null)
          setExpr(e.target.value)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            press('=')
          }
        }}
        dir="ltr"
        inputMode="decimal"
        aria-label={t.calculator}
        className="numeric w-full rounded-lg border px-3 py-2 text-end text-lg"
        style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
      />
      <div className="mt-1 flex items-baseline justify-between gap-3">
        <p className="numeric m-0 text-2xl" style={{ fontWeight: 550, letterSpacing: '-0.015em', minHeight: '1.5em' }} dir="ltr" aria-live="polite">
          {live ?? (expr.trim() ? '…' : '')}
        </p>
        {live !== null && <CopyButton value={live} label={t.copy} copied={t.copied} />}
      </div>
      <div className="mt-2 grid gap-1.5" style={{ gridTemplateColumns: 'repeat(4, minmax(0, 1fr))' }} role="group" aria-label={t.calculator} dir="ltr">
        {KEYS.flat().map((k) => {
          const op = '÷×−+^%()√'.includes(k)
          const act = k === '=' || k === 'C' || k === '⌫'
          return (
            <button
              key={k}
              type="button"
              onClick={() => press(k)}
              aria-label={k === '⌫' ? t.calcBackspace : k === 'C' ? t.calcClear : k === '=' ? t.calcEquals : k}
              className="numeric cursor-pointer rounded-lg border py-2 text-base"
              style={{
                gridColumn: k === '=' ? 'span 2' : undefined,
                borderColor: 'var(--line)',
                background: k === '=' ? 'var(--accent)' : op || act ? 'var(--bg-subtle, var(--bg))' : 'var(--bg)',
                color: k === '=' ? 'var(--bg)' : 'var(--fg)',
                fontWeight: act ? 550 : 450,
              }}
            >
              {k}
            </button>
          )
        })}
      </div>
      {committed !== null && (
        <p className="mt-1 text-xs" style={{ color: 'var(--fg-faint)' }} dir="ltr">
          = {committed}
        </p>
      )}
      <p className="sr-only">{normalise(expr)}</p>
    </div>
  )
}
