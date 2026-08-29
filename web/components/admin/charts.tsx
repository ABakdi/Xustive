'use client'

import { useId, useMemo, useState } from 'react'

/**
 * The console's chart kit (M12). Plain SVG, no library: a few hundred lines that stay inside
 * the bundle budget and render in both themes from the `--viz-*` roles in `globals.css`.
 *
 * Built to the data-viz method: the form follows the data's job (a stat tile for one number, a
 * line for change over time, thin horizontal bars for magnitude, a meter for a ratio against a
 * limit); marks are thin (2px lines, ≤ 24px bars with 4px rounded data-ends, ≥ 8px markers with
 * a surface ring); the grid is a recessive hairline; text wears text tokens, never the series
 * colour; every chart with two or more series carries a legend, and every chart has a table
 * twin behind a toggle, so nothing is reachable only by hover. Line and bar charts carry a
 * hover layer — a crosshair that snaps to the nearest x and one tooltip listing every series.
 */

export type Series = { name: string; values: (number | null)[]; color?: string }

const SERIES = ['var(--viz-1)', 'var(--viz-2)', 'var(--viz-3)', 'var(--viz-4)']
const colorOf = (s: Series, i: number) => s.color ?? SERIES[i] ?? 'var(--viz-other)'

/** 1,284 · 12.9K · 4.2M — compact figures for tiles and axes. */
export function compact(n: number | null | undefined, digits = 1): string {
  if (n === null || n === undefined || Number.isNaN(n)) return '—'
  const a = Math.abs(n)
  if (a >= 1e9) return `${(n / 1e9).toFixed(digits)}B`
  if (a >= 1e6) return `${(n / 1e6).toFixed(digits)}M`
  if (a >= 1e4) return `${(n / 1e3).toFixed(digits)}K`
  if (Number.isInteger(n)) return n.toLocaleString('en')
  return n.toFixed(a < 10 ? 2 : 1)
}

/** Clean axis ticks: 0 / 1,000 / 2,000, never 0 / 743 / 1,486. */
function niceTicks(max: number, count = 4): number[] {
  if (max <= 0) return [0]
  const raw = max / count
  const mag = 10 ** Math.floor(Math.log10(raw))
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? mag
  const ticks: number[] = []
  for (let v = 0; v <= max + step * 0.001; v += step) ticks.push(Number(v.toFixed(6)))
  return ticks
}

// ------------------------------------------------------------------------------------------
// Stat tile

export function StatTile({
  label,
  value,
  unit,
  delta,
  deltaLabel,
  upIsGood = true,
  trend,
  status,
}: {
  label: string
  value: string | number
  unit?: string
  /** Signed change against a named period; colour = direction × whether up is good. */
  delta?: number | null
  deltaLabel?: string
  upIsGood?: boolean
  /** A short series in the de-emphasis hue, the current period in the accent. */
  trend?: (number | null)[]
  /** A status pairs an icon with its colour — never colour alone. */
  status?: 'good' | 'warning' | 'critical'
}) {
  const good = delta !== null && delta !== undefined && (delta >= 0) === upIsGood
  const icon = status === 'good' ? '●' : status === 'warning' ? '▲' : status === 'critical' ? '■' : null
  return (
    <div className="rounded border p-3" style={{ borderColor: 'var(--line)', background: 'var(--surface)' }}>
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs" style={{ color: 'var(--fg-faint)' }}>
          {label}
        </div>
        {icon && (
          <span className="text-[10px]" style={{ color: `var(--viz-${status})` }} aria-label={status}>
            {icon}
          </span>
        )}
      </div>
      <div className="mt-0.5 flex items-baseline gap-1.5">
        <span className="text-xl font-semibold" style={{ fontVariantNumeric: 'normal' }}>
          {typeof value === 'number' ? compact(value) : value}
        </span>
        {unit && <span className="text-xs" style={{ color: 'var(--fg-muted)' }}>{unit}</span>}
        {delta !== null && delta !== undefined && (
          <span className="ms-auto text-xs" style={{ color: good ? 'var(--viz-good)' : 'var(--viz-critical)' }}>
            {delta > 0 ? '+' : ''}
            {compact(delta)}
            {deltaLabel && <span style={{ color: 'var(--fg-faint)' }}> {deltaLabel}</span>}
          </span>
        )}
      </div>
      {trend && trend.some((v) => v !== null) && (
        <div className="mt-1.5">
          <Sparkline values={trend} />
        </div>
      )}
    </div>
  )
}

export function Sparkline({ values, height = 24 }: { values: (number | null)[]; height?: number }) {
  const w = 120
  const pts = values.map((v, i) => [i, v] as const).filter((p): p is readonly [number, number] => p[1] !== null)
  if (pts.length < 2) return null
  const max = Math.max(...pts.map((p) => p[1]), 1)
  const x = (i: number) => (i / Math.max(values.length - 1, 1)) * (w - 4) + 2
  const y = (v: number) => height - 2 - (v / max) * (height - 4)
  const d = pts.map((p, k) => `${k ? 'L' : 'M'}${x(p[0]).toFixed(1)} ${y(p[1]).toFixed(1)}`).join(' ')
  const last = pts[pts.length - 1]!
  return (
    <svg width={w} height={height} viewBox={`0 0 ${w} ${height}`} aria-hidden className="block max-w-full">
      <path d={d} fill="none" stroke="var(--viz-other)" strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
      <circle cx={x(last[0])} cy={y(last[1])} r={3} fill="var(--viz-1)" stroke="var(--surface)" strokeWidth={2} />
    </svg>
  )
}

// ------------------------------------------------------------------------------------------
// Chart chrome: legend + table toggle

function TableToggle({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <button type="button" className="text-xs underline-offset-2 hover:underline" style={{ color: 'var(--fg-faint)' }} onClick={onToggle} aria-pressed={open}>
      {open ? 'Chart' : 'Table'}
    </button>
  )
}

function Legend({ series, hidden, onToggle }: { series: Series[]; hidden: Set<string>; onToggle: (n: string) => void }) {
  if (series.length < 2) return null
  return (
    <ul className="m-0 flex flex-wrap gap-3 p-0 text-xs" style={{ listStyle: 'none', color: 'var(--fg-muted)' }}>
      {series.map((s, i) => (
        <li key={s.name}>
          <button type="button" className="inline-flex items-center gap-1.5" onClick={() => onToggle(s.name)} aria-pressed={!hidden.has(s.name)} style={{ opacity: hidden.has(s.name) ? 0.4 : 1 }}>
            <span aria-hidden className="inline-block h-0.5 w-3 rounded" style={{ background: colorOf(s, i) }} />
            {s.name}
          </button>
        </li>
      ))}
    </ul>
  )
}

function DataTable({ labels, series, format }: { labels: string[]; series: Series[]; format?: (n: number) => string }) {
  const f = format ?? ((n: number) => compact(n))
  return (
    <div className="overflow-x-auto">
      <div className="scroll-x">
        <table className="w-full text-xs" style={{ fontVariantNumeric: 'tabular-nums' }}>
          <thead>
            <tr style={{ color: 'var(--fg-faint)' }}>
              <th className="py-1 text-start font-normal"></th>
              {series.map((s) => <th key={s.name} className="py-1 text-end font-normal">{s.name}</th>)}
            </tr>
          </thead>
          <tbody>
            {labels.map((l, i) => (
              <tr key={l + i} style={{ borderTop: '1px solid var(--line)' }}>
                <td className="py-1 text-start">{l}</td>
                {series.map((s) => <td key={s.name} className="py-1 text-end">{s.values[i] === null || s.values[i] === undefined ? '—' : f(s.values[i] as number)}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

// ------------------------------------------------------------------------------------------
// Line chart — change over time, one or more series, crosshair + one tooltip

export function LineChart({
  title,
  labels,
  series,
  height = 160,
  format,
  area = false,
  unit,
}: {
  title?: string
  labels: string[]
  series: Series[]
  height?: number
  format?: (n: number) => string
  /** A wash under a single series. */
  area?: boolean
  unit?: string
}) {
  const id = useId()
  const [table, setTable] = useState(false)
  const [hidden, setHidden] = useState<Set<string>>(new Set())
  const [hover, setHover] = useState<number | null>(null)
  const f = format ?? ((n: number) => compact(n))
  const shown = series.filter((s) => !hidden.has(s.name))
  const W = 600
  const padL = 40
  const padR = 12
  const padT = 8
  const padB = 22
  const plotH = height - padT - padB
  const max = Math.max(1, ...shown.flatMap((s) => s.values.filter((v): v is number => v !== null)))
  const ticks = useMemo(() => niceTicks(max), [max])
  const top = ticks[ticks.length - 1] ?? max
  const n = labels.length
  const x = (i: number) => padL + (n > 1 ? (i / (n - 1)) * (W - padL - padR) : 0)
  const y = (v: number) => padT + plotH - (v / top) * plotH
  const path = (s: Series) => {
    let d = ''
    let pen = false
    s.values.forEach((v, i) => {
      if (v === null) {
        pen = false
        return
      }
      d += `${pen ? 'L' : 'M'}${x(i).toFixed(1)} ${y(v).toFixed(1)} `
      pen = true
    })
    return d
  }
  const nearest = (clientX: number, rect: DOMRect) => {
    const px = ((clientX - rect.left) / rect.width) * W
    let best = 0
    let bd = Infinity
    for (let i = 0; i < n; i++) {
      const d = Math.abs(x(i) - px)
      if (d < bd) {
        bd = d
        best = i
      }
    }
    return best
  }
  // Label the x axis sparingly: first, last, and a few between.
  const xLabelEvery = Math.max(1, Math.ceil(n / 6))

  return (
    <figure className="m-0 rounded border p-3" style={{ borderColor: 'var(--line)', background: 'var(--surface)' }}>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        {title && <figcaption className="text-sm font-medium">{title}</figcaption>}
        <div className="flex items-center gap-3">
          <Legend series={series} hidden={hidden} onToggle={(nm) => setHidden((h) => { const c = new Set(h); if (c.has(nm)) c.delete(nm); else c.add(nm); return c })} />
          <TableToggle open={table} onToggle={() => setTable((t) => !t)} />
        </div>
      </div>
      {table ? (
        <DataTable labels={labels} series={series} format={f} />
      ) : (
        <div className="relative">
          <svg
            viewBox={`0 0 ${W} ${height}`}
            className="block w-full"
            style={{ height }}
            role="img"
            aria-labelledby={`${id}-t`}
            onPointerMove={(e) => setHover(nearest(e.clientX, e.currentTarget.getBoundingClientRect()))}
            onPointerLeave={() => setHover(null)}
          >
            <title id={`${id}-t`}>{title ?? series.map((s) => s.name).join(', ')}</title>
            {ticks.map((t) => (
              <g key={t}>
                <line x1={padL} x2={W - padR} y1={y(t)} y2={y(t)} stroke="var(--viz-grid)" strokeWidth={1} />
                <text x={padL - 6} y={y(t) + 3} textAnchor="end" fontSize={10} fill="var(--fg-faint)" style={{ fontVariantNumeric: 'tabular-nums' }}>
                  {f(t)}
                </text>
              </g>
            ))}
            <line x1={padL} x2={W - padR} y1={y(0)} y2={y(0)} stroke="var(--viz-axis)" strokeWidth={1} />
            {labels.map((l, i) =>
              i % xLabelEvery === 0 || i === n - 1 ? (
                <text key={l + i} x={x(i)} y={height - 6} textAnchor={i === 0 ? 'start' : i === n - 1 ? 'end' : 'middle'} fontSize={10} fill="var(--fg-faint)">
                  {l}
                </text>
              ) : null,
            )}
            {shown.map((s, i) => {
              const c = colorOf(s, series.indexOf(s))
              const d = path(s)
              return (
                <g key={s.name}>
                  {area && shown.length === 1 && d && (
                    <path d={`${d}L${x(n - 1)} ${y(0)} L${x(0)} ${y(0)} Z`} fill={c} opacity="var(--viz-area-alpha, 0.1)" />
                  )}
                  <path d={d} fill="none" stroke={c} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
                  {hover !== null && s.values[hover] !== null && s.values[hover] !== undefined && (
                    <circle cx={x(hover)} cy={y(s.values[hover] as number)} r={4} fill={c} stroke="var(--surface)" strokeWidth={2} />
                  )}
                  {i === 0 && null}
                </g>
              )
            })}
            {hover !== null && <line x1={x(hover)} x2={x(hover)} y1={padT} y2={padT + plotH} stroke="var(--viz-axis)" strokeWidth={1} />}
          </svg>
          {hover !== null && (
            <div className="viz-tip" style={{ insetInlineStart: `${(x(hover) / W) * 100}%`, top: 0, transform: hover > n / 2 ? 'translateX(-105%)' : 'translateX(8px)' }}>
              <div style={{ color: 'var(--fg-faint)' }}>{labels[hover]}</div>
              {shown.map((s) => (
                <div key={s.name} className="flex items-center gap-2">
                  <span aria-hidden className="inline-block h-0.5 w-3 rounded" style={{ background: colorOf(s, series.indexOf(s)) }} />
                  <strong style={{ fontVariantNumeric: 'tabular-nums' }}>{s.values[hover] === null || s.values[hover] === undefined ? '—' : f(s.values[hover] as number)}</strong>
                  {unit && <span style={{ color: 'var(--fg-faint)' }}>{unit}</span>}
                  {series.length > 1 && <span style={{ color: 'var(--fg-muted)' }}>{s.name}</span>}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </figure>
  )
}

// ------------------------------------------------------------------------------------------
// Horizontal bars — magnitude, one series, thin, rounded data-end, value at the tip

export function Bars({
  title,
  items,
  format,
  max: maxIn,
  color = 'var(--viz-1)',
  onPick,
}: {
  title?: string
  items: { label: string; value: number; hint?: string }[]
  format?: (n: number) => string
  max?: number
  color?: string
  /** A bar is a control when the row implies an action (open the query, the page). */
  onPick?: (item: { label: string; value: number }) => void
}) {
  const [table, setTable] = useState(false)
  const f = format ?? ((n: number) => compact(n))
  const max = Math.max(1, maxIn ?? Math.max(...items.map((i) => i.value)))
  if (items.length === 0) return <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>Nothing in this window.</p>
  return (
    <figure className="m-0 rounded border p-3" style={{ borderColor: 'var(--line)', background: 'var(--surface)' }}>
      <div className="mb-2 flex items-center justify-between gap-2">
        {title && <figcaption className="text-sm font-medium">{title}</figcaption>}
        <TableToggle open={table} onToggle={() => setTable((t) => !t)} />
      </div>
      {table ? (
        <DataTable labels={items.map((i) => i.label)} series={[{ name: title ?? 'value', values: items.map((i) => i.value) }]} format={f} />
      ) : (
        <ul className="m-0 flex flex-col gap-1.5 p-0" style={{ listStyle: 'none' }}>
          {items.map((it) => {
            const w = Math.max(2, (it.value / max) * 100)
            const inner = (
              <>
                <span className="w-2/5 min-w-0 truncate text-xs" dir="auto" title={it.hint ?? it.label}>
                  {it.label}
                </span>
                <span className="relative h-3 flex-1 rounded-e" style={{ background: 'transparent' }}>
                  <span className="absolute inset-y-0 start-0 rounded-e" style={{ width: `${w}%`, background: color, maxHeight: 24 }} />
                </span>
                <span className="w-12 text-end text-xs" style={{ fontVariantNumeric: 'tabular-nums', color: 'var(--fg)' }}>
                  {f(it.value)}
                </span>
              </>
            )
            return (
              <li key={it.label} className="flex items-center gap-2">
                {onPick ? (
                  <button type="button" className="flex w-full items-center gap-2 text-start hover:opacity-80" onClick={() => onPick(it)}>
                    {inner}
                  </button>
                ) : (
                  inner
                )}
              </li>
            )
          })}
        </ul>
      )}
    </figure>
  )
}

// ------------------------------------------------------------------------------------------
// Meter — one ratio against a limit; the fill carries severity, the track is the same ramp

export function Meter({ label, value, max, unit, warnAt = 0.7, critAt = 0.9 }: { label: string; value: number; max: number; unit?: string; warnAt?: number; critAt?: number }) {
  const r = max > 0 ? Math.min(1, value / max) : 0
  const tone = r >= critAt ? 'var(--viz-critical)' : r >= warnAt ? 'var(--viz-warning)' : 'var(--viz-1)'
  return (
    <div>
      <div className="flex items-baseline justify-between text-xs">
        <span style={{ color: 'var(--fg-muted)' }}>{label}</span>
        <span style={{ fontVariantNumeric: 'tabular-nums' }}>
          {compact(value)}
          {unit ? ` ${unit}` : ''} <span style={{ color: 'var(--fg-faint)' }}>/ {compact(max)}{unit ? ` ${unit}` : ''}</span>
        </span>
      </div>
      <div className="mt-1 h-2 w-full overflow-hidden rounded" style={{ background: 'var(--viz-seq-1)' }} role="meter" aria-valuenow={value} aria-valuemin={0} aria-valuemax={max} aria-label={label}>
        <div className="h-full rounded" style={{ width: `${r * 100}%`, background: tone }} />
      </div>
    </div>
  )
}
