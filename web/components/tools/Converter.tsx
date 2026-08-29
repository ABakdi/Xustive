'use client'

import { useMemo, useState } from 'react'

import { convert, DIMENSION_LABEL, findUnit, trim, UNITS, unitLabel, type Dimension, type Unit } from '@/lib/units'
import type { Messages } from '@/lib/i18n/messages'

import { CopyButton } from './CopyButton'

/**
 * The unit converter, as a tool: the amount and the two units the API read from the query are
 * loaded into editable controls — an amount box, two unit menus grouped by dimension, a swap —
 * and the result follows every change. The table mirrors the API's, so the first number shown
 * is the one the API answered.
 */
export function Converter({
  amount: amount0,
  from: from0,
  to: to0,
  t,
  locale,
}: {
  amount: string
  from: string
  to: string
  t: Messages
  locale: string
}) {
  const [amount, setAmount] = useState(amount0)
  const [from, setFrom] = useState(findUnit(from0)?.name ?? 'metre')
  const [to, setTo] = useState(findUnit(to0)?.name ?? 'foot')
  const fromUnit: Unit = findUnit(from) ?? UNITS[0]!
  const toUnit: Unit = findUnit(to) ?? UNITS[5]!
  const dims = useMemo(() => Array.from(new Set(UNITS.map((u) => u.dimension))) as Dimension[], [])
  const dl = (d: Dimension) => (locale === 'fr' ? DIMENSION_LABEL[d].fr : locale === 'ar' || locale === 'ary' ? DIMENSION_LABEL[d].ar : DIMENSION_LABEL[d].en)

  const value = Number(amount.replace(/[٠-٩]/g, (d) => String(d.charCodeAt(0) - 0x0660)).replace(',', '.'))
  const result = Number.isFinite(value) ? convert(value, fromUnit, toUnit) : null
  const shown = result === null ? null : `${trim(result)} ${unitLabel(toUnit, locale)}`

  const pickFrom = (name: string) => {
    const u = findUnit(name)
    if (!u) return
    setFrom(u.name)
    // Keep the pair inside one dimension: a target of another kind becomes the first sibling.
    if (toUnit.dimension !== u.dimension) {
      const sib = UNITS.find((x) => x.dimension === u.dimension && x.name !== u.name) ?? u
      setTo(sib.name)
    }
  }

  const select = (v: string, onChange: (name: string) => void, label: string, only?: Dimension) => (
    <select
      value={v}
      onChange={(e) => onChange(e.target.value)}
      aria-label={label}
      className="rounded-lg border px-2 py-2 text-sm"
      style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minWidth: 0 }}
    >
      {dims
        .filter((d) => !only || d === only)
        .map((d) => (
          <optgroup key={d} label={dl(d)}>
            {UNITS.filter((u) => u.dimension === d).map((u) => (
              <option key={u.name} value={u.name}>
                {unitLabel(u, locale)}
              </option>
            ))}
          </optgroup>
        ))}
    </select>
  )

  return (
    <div className="mt-2 max-w-md">
      <div className="grid items-center gap-2" style={{ gridTemplateColumns: 'minmax(90px, 1fr) minmax(0, 1.4fr) auto minmax(0, 1.4fr)' }}>
        <input
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          inputMode="decimal"
          dir="ltr"
          aria-label={t.convAmount}
          className="numeric rounded-lg border px-3 py-2 text-end text-base"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)', minWidth: 0 }}
        />
        {select(fromUnit.name, pickFrom, t.convFrom)}
        <button
          type="button"
          onClick={() => {
            setFrom(toUnit.name)
            setTo(fromUnit.name)
          }}
          aria-label={t.convSwap}
          title={t.convSwap}
          className="cursor-pointer rounded-lg border px-2 py-2 text-sm"
          style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
        >
          ⇄
        </button>
        {select(toUnit.name, (n) => setTo(n), t.convTo, fromUnit.dimension)}
      </div>
      <div className="mt-2 flex items-baseline gap-3">
        <p className="numeric m-0 text-2xl" style={{ fontWeight: 550, letterSpacing: '-0.015em', minHeight: '1.5em' }} aria-live="polite">
          <bdi>{shown ?? '…'}</bdi>
        </p>
        {shown && <CopyButton value={shown} label={t.copy} copied={t.copied} />}
      </div>
      {result !== null && Number.isFinite(value) && value !== 0 && fromUnit.dimension !== 'temperature' && (
        <p className="mt-1 text-xs" style={{ color: 'var(--fg-faint)' }}>
          <bdi>1 {unitLabel(fromUnit, locale)} = {trim(convert(1, fromUnit, toUnit) ?? 0)} {unitLabel(toUnit, locale)}</bdi>
        </p>
      )}
    </div>
  )
}
