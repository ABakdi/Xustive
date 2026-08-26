import type { Messages } from '@/lib/i18n/messages'

/**
 * The weather card's body (M8-T05.3, T05.4, T05.6, T05.7).
 *
 * The forecast has been computed, serialised and sent since M1B, and never once drawn. This draws
 * it: the current conditions, a day strip, an hourly graph, and the week behind a toggle.
 *
 * **Server-rendered, no JavaScript.** The graph is an inline SVG polyline rather than a canvas
 * chart, and the week toggle is a `<details>` element. A charting library would cost more than the
 * page's whole bundle budget and would leave the no-JS path — which `scripts/no-js-check.sh`
 * enforces — showing an empty box.
 */

/** Shapes validated rather than trusted: `detail` is whatever the tool attached. */
type Day = { date: string; high: number; low: number; code: number }
type Hour = { time: string; temperature: number; precipitation_chance: number; code: number }

export function WeatherDetail({
  detail,
  t,
  locale,
}: {
  detail: unknown
  t: Messages
  locale: string
}) {
  const d = detail as
    | {
        days?: unknown
        hours?: unknown
        wind_kmh?: unknown
        humidity?: unknown
        feels_like?: unknown
        assumed_place?: unknown
        wilaya?: { ar?: string; fr?: string }
        source?: unknown
      }
    | undefined
  if (!d) return null

  const days = asArray<Day>(d.days, (x) => typeof x?.date === 'string')
  const hours = asArray<Hour>(d.hours, (x) => typeof x?.time === 'string')
  if (days.length === 0 && hours.length === 0) return null

  return (
    <div className="mt-3">
      {/* Which place this is for, and whether we guessed it. A reader shown the wrong city with no
          way to tell it was a guess has no reason to doubt it — and correcting it is one word. */}
      {d.assumed_place === true && (
        <p className="mb-2 text-xs" style={{ color: 'var(--fg-faint)' }} dir="auto">
          {t.weatherAssumed}
        </p>
      )}

      {hours.length > 0 && <HourlyGraph hours={hours} t={t} locale={locale} />}

      {days.length > 0 && (
        <>
          <ul className="mt-3 flex list-none flex-wrap gap-3 p-0">
            {days.slice(0, 3).map((day) => (
              <DayChip key={day.date} day={day} t={t} locale={locale} />
            ))}
          </ul>
          {days.length > 3 && (
            <details className="mt-2">
              <summary className="cursor-pointer text-sm" style={{ color: 'var(--fg-muted)' }}>
                {t.weatherWeek}
              </summary>
              <ul className="mt-2 flex list-none flex-wrap gap-3 p-0">
                {days.slice(3).map((day) => (
                  <DayChip key={day.date} day={day} t={t} locale={locale} />
                ))}
              </ul>
            </details>
          )}
        </>
      )}
    </div>
  )
}

function DayChip({ day, t, locale }: { day: Day; t: Messages; locale: string }) {
  return (
    <li className="flex min-w-16 flex-col items-center gap-0.5 text-sm">
      <span style={{ color: 'var(--fg-muted)' }} dir="auto">
        {weekday(day.date, locale)}
      </span>
      <span aria-hidden className="text-lg leading-none">
        {wmoGlyph(day.code)}
      </span>
      <span className="sr-only">{wmoLabel(day.code, t)}</span>
      <span>
        <bdi>
          {Math.round(day.high)}° / {Math.round(day.low)}°
        </bdi>
      </span>
    </li>
  )
}

/**
 * Temperature and rain chance over the next day, as one inline SVG.
 *
 * Drawn left-to-right regardless of the page direction. A temperature curve is a time axis, and
 * time does not mirror — an Arabic reader expects tomorrow to the right of today just as an
 * English one does, which is why this is not flipped with the text.
 */
function HourlyGraph({ hours, t, locale }: { hours: Hour[]; t: Messages; locale: string }) {
  const points = hours.slice(0, 24)
  const first = points[0]
  const last = points[points.length - 1]
  if (points.length < 2 || !first || !last) return null

  const W = 280
  const H = 64
  const PAD = 6
  const temps = points.map((p) => p.temperature)
  const min = Math.min(...temps)
  const max = Math.max(...temps)
  // A flat day would divide by zero and collapse the curve onto one edge.
  const span = max - min || 1

  const x = (i: number) => PAD + (i * (W - PAD * 2)) / (points.length - 1)
  const y = (v: number) => PAD + ((max - v) * (H - PAD * 2)) / span

  const line = points.map((p, i) => `${x(i).toFixed(1)},${y(p.temperature).toFixed(1)}`).join(' ')
  const rain = points.filter((p) => p.precipitation_chance >= 30)

  return (
    <figure className="m-0">
      <figcaption className="mb-1 text-xs" style={{ color: 'var(--fg-faint)' }} dir="auto">
        {t.weatherNext24}
      </figcaption>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width="100%"
        height={H}
        role="img"
        aria-label={`${t.weatherNext24}: ${Math.round(min)}° – ${Math.round(max)}°`}
        style={{ direction: 'ltr' }}
      >
        {/* Rain hours as bars behind the curve, so the two series read at once without a legend. */}
        {rain.map((p) => {
          const i = points.indexOf(p)
          return (
            <rect
              key={p.time}
              x={x(i) - 3}
              y={PAD}
              width={6}
              height={H - PAD * 2}
              fill="var(--accent)"
              opacity={Math.min(0.28, p.precipitation_chance / 400)}
            />
          )
        })}
        <polyline
          points={line}
          fill="none"
          stroke="var(--accent)"
          strokeWidth={1.75}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      </svg>
      <p className="mt-0.5 flex justify-between text-xs" style={{ color: 'var(--fg-faint)', direction: 'ltr' }}>
        <bdi>{hourLabel(first.time, locale)}</bdi>
        <bdi>
          {Math.round(min)}° – {Math.round(max)}°
        </bdi>
        <bdi>{hourLabel(last.time, locale)}</bdi>
      </p>
    </figure>
  )
}

function asArray<T>(value: unknown, ok: (x: T) => boolean): T[] {
  return Array.isArray(value) ? (value as T[]).filter((x) => x && ok(x)) : []
}

function weekday(date: string, locale: string) {
  const d = new Date(`${date}T12:00:00Z`)
  if (Number.isNaN(d.getTime())) return date
  return new Intl.DateTimeFormat(locale, { weekday: 'short', timeZone: 'UTC' }).format(d)
}

function hourLabel(time: string, locale: string) {
  const d = new Date(`${time}:00Z`)
  if (Number.isNaN(d.getTime())) return time
  return new Intl.DateTimeFormat(locale, { hour: 'numeric', timeZone: 'UTC' }).format(d)
}

/**
 * WMO code to a glyph (M8-T05.7).
 *
 * Text glyphs rather than drawn icons: they inherit the reader's font and colour, cost no bytes,
 * and cannot be the thing that fails to load. The M1B note asked for custom line icons; these are
 * the honest interim, and they are already better than the bare code the card used to carry.
 */
function wmoGlyph(code: number): string {
  if (code === 0) return '☀'
  if (code <= 2) return '⛅'
  if (code === 3) return '☁'
  if (code <= 48) return '🌫'
  if (code <= 57) return '🌦'
  if (code <= 67) return '🌧'
  if (code <= 77) return '❄'
  if (code <= 82) return '🌧'
  if (code <= 86) return '❄'
  return '⛈'
}

function wmoLabel(code: number, t: Messages): string {
  if (code === 0) return t.wmoClear
  if (code <= 2) return t.wmoPartly
  if (code === 3) return t.wmoCloudy
  if (code <= 48) return t.wmoFog
  if (code <= 67) return t.wmoRain
  if (code <= 77) return t.wmoSnow
  if (code <= 82) return t.wmoRain
  if (code <= 86) return t.wmoSnow
  return t.wmoStorm
}
