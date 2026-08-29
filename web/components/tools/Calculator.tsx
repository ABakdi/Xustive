'use client'

import { useEffect, useMemo, useRef, useState } from 'react'

import { evaluate, render, type AngleMode } from '@/lib/calc'
import type { Messages } from '@/lib/i18n/messages'

import { CopyButton } from './CopyButton'

/**
 * The scientific calculator, as a tool rather than an answer: the expression the reader typed
 * is loaded into a working calculator — a scientific keypad, keyboard input, a degree/radian
 * mode, live evaluation on every keystroke — so the next calculation does not need another
 * search. Evaluated locally (`lib/calc`, real and complex), the same grammar the API's tool
 * reads; where the API's engine knows more (mixed units, bases) its answer is shown until the
 * expression is edited.
 */
type Key = { k: string; ins?: string; label?: string; wide?: boolean; kind?: 'op' | 'fn' | 'act' | 'eq' }
const ROWS: Key[][] = [
  [{ k: 'sin', ins: 'sin(', kind: 'fn' }, { k: 'cos', ins: 'cos(', kind: 'fn' }, { k: 'tan', ins: 'tan(', kind: 'fn' }, { k: 'ln', ins: 'ln(', kind: 'fn' }, { k: 'log', ins: 'log(', kind: 'fn' }],
  [{ k: 'asin', ins: 'asin(', kind: 'fn' }, { k: 'acos', ins: 'acos(', kind: 'fn' }, { k: 'atan', ins: 'atan(', kind: 'fn' }, { k: 'x²', ins: '^2', kind: 'fn' }, { k: 'xʸ', ins: '^', kind: 'fn' }],
  [{ k: '(', kind: 'op' }, { k: ')', kind: 'op' }, { k: 'π', kind: 'fn' }, { k: 'e', kind: 'fn' }, { k: 'i', kind: 'fn' }],
  [{ k: '7' }, { k: '8' }, { k: '9' }, { k: '÷', kind: 'op' }, { k: '√', ins: '√(', kind: 'fn' }],
  [{ k: '4' }, { k: '5' }, { k: '6' }, { k: '×', kind: 'op' }, { k: 'n!', ins: '!', kind: 'fn' }],
  [{ k: '1' }, { k: '2' }, { k: '3' }, { k: '−', kind: 'op' }, { k: '%', kind: 'op' }],
  [{ k: '0' }, { k: '.' }, { k: '°', kind: 'op' }, { k: '+', kind: 'op' }, { k: '⌫', kind: 'act' }],
  [{ k: 'C', kind: 'act' }, { k: 'mod', ins: ' mod ', kind: 'fn' }, { k: '=', kind: 'eq', wide: true }, { k: 'DEG', kind: 'act' }],
]

export function Calculator({ initial, fallback, t }: { initial: string; fallback?: string; t: Messages }) {
  const [expr, setExpr] = useState(initial)
  const [mode, setMode] = useState<AngleMode>('deg')
  const [committed, setCommitted] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => setExpr(initial), [initial])

  const live = useMemo(() => {
    const v = evaluate(expr, mode)
    if (v) return render(v)
    // The API's engine read more than this one can (units, bases): keep its answer while the
    // expression is the one it answered.
    return expr === initial && fallback ? fallback : null
  }, [expr, mode, initial, fallback])

  const press = (key: Key) => {
    setCommitted(null)
    if (key.k === 'C') return setExpr('')
    if (key.k === '⌫') return setExpr((e) => e.slice(0, -1))
    if (key.k === 'DEG') return setMode((m) => (m === 'deg' ? 'rad' : 'deg'))
    if (key.k === '=') {
      if (live !== null) {
        setCommitted(live)
        setExpr(live.replace(/^approx\. /, ''))
      }
      return
    }
    setExpr((e) => e + (key.ins ?? key.k))
    input.current?.focus()
  }

  return (
    <div className="mt-2 max-w-md">
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
            press({ k: '=' })
          }
        }}
        dir="ltr"
        inputMode="text"
        autoComplete="off"
        spellCheck={false}
        aria-label={t.calculator}
        className="numeric w-full rounded-lg border px-3 py-2 text-end text-lg"
        style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
      />
      <div className="mt-1 flex items-baseline justify-between gap-3">
        <p className="numeric m-0 text-2xl" style={{ fontWeight: 550, letterSpacing: '-0.015em', minHeight: '1.5em' }} dir="ltr" aria-live="polite">
          {live ?? (expr.trim() ? '…' : '')}
        </p>
        <span className="flex items-center gap-2">
          <span className="text-xs" style={{ color: 'var(--fg-faint)' }} title={mode === 'deg' ? 'degrees' : 'radians'}>
            {mode === 'deg' ? 'DEG' : 'RAD'}
          </span>
          {live !== null && <CopyButton value={live} label={t.copy} copied={t.copied} />}
        </span>
      </div>
      <div className="mt-2 grid gap-1.5" style={{ gridTemplateColumns: 'repeat(5, minmax(0, 1fr))' }} role="group" aria-label={t.calculator} dir="ltr">
        {ROWS.flat().map((key) => {
          const isEq = key.kind === 'eq'
          const isMode = key.k === 'DEG'
          const label = isMode ? (mode === 'deg' ? 'DEG' : 'RAD') : key.k
          return (
            <button
              key={key.k}
              type="button"
              onClick={() => press(key)}
              aria-label={key.k === '⌫' ? t.calcBackspace : key.k === 'C' ? t.calcClear : isEq ? t.calcEquals : label}
              aria-pressed={isMode ? mode === 'rad' : undefined}
              className={`${key.kind === 'fn' ? '' : 'numeric'} cursor-pointer rounded-lg border py-2 text-sm`}
              style={{
                gridColumn: key.wide ? 'span 2' : undefined,
                borderColor: isEq ? 'var(--accent)' : 'var(--line)',
                background: isEq ? 'var(--accent)' : key.kind ? 'var(--bg-subtle, var(--bg))' : 'var(--bg)',
                color: isEq ? 'var(--bg)' : key.kind === 'fn' ? 'var(--fg-muted)' : 'var(--fg)',
                fontWeight: key.kind === 'act' || isEq ? 550 : 450,
              }}
            >
              {label}
            </button>
          )
        })}
      </div>
      {committed !== null && (
        <p className="mt-1 text-xs" style={{ color: 'var(--fg-faint)' }} dir="ltr">
          = {committed}
        </p>
      )}
    </div>
  )
}
