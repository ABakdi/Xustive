import type { InstantAnswer } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

import { CopyButton } from './CopyButton'
import { DismissTool } from './DismissTool'

/**
 * The instant-answer card.
 *
 * One frame for every tool. Consistency here is what makes an unfamiliar tool legible on sight.
 *
 * Carries the assert rule, because this is the engine answering rather than listing what someone
 * else published. Result cards never have it.
 *
 * Server-rendered: the answer is computed by the API and arrives with the search response, so it
 * is visible before any JavaScript runs. Only the copy button hydrates.
 */
export function ToolCard({
  answer,
  t,
  locale,
}: {
  answer: InstantAnswer
  t: Messages
  locale: string
}) {
  // `detail` is whatever the tool chose to attach, so it is validated rather than trusted: a
  // non-array here would throw during render and take the whole result page with it.
  const detail = answer.detail as
    | { alternatives?: unknown; administered?: unknown }
    | undefined
  const raw = detail?.alternatives
  const alternatives = Array.isArray(raw) ? raw.filter((a): a is string => typeof a === 'string') : []

  // Set by tools whose value is fixed by an authority rather than measured — fuel prices, which
  // the ARH changes with no announcement. Without this the number reads as a live quote, and the
  // gap between "this is the official price" and "this is today's price" is the whole difference.
  const administered = detail?.administered === true

  const stale = answer.as_of
    ? new Intl.DateTimeFormat(locale === 'ary' ? 'ar' : locale, {
        hour: '2-digit',
        minute: '2-digit',
        day: 'numeric',
        month: 'short',
      }).format(new Date(answer.as_of * 1000))
    : null

  return (
    <section
      className="assert group mb-7"
      aria-label={t[answer.tool as keyof Messages] ?? answer.tool}
    >
      {/* How the query was read. Not decoration: `20 dollar` taken as USD when the user meant
          Canadian is a wrong answer, and showing the interpretation makes that visible in half a
          second. Isolated because an expression inside RTL text is reordered into nonsense. */}
      <p className="m-0 text-xs" style={{ color: 'var(--fg-muted)' }}>
        <bdi>{answer.interpretation}</bdi>
      </p>

      <div className="mt-1 flex items-baseline gap-3">
        <p
          className="numeric m-0 text-2xl"
          style={{ fontWeight: 550, letterSpacing: '-0.015em' }}
        >
          <bdi>{answer.value}</bdi>
        </p>
        <CopyButton value={answer.value} label={t.copy} copied={t.copied} />
        <DismissTool tool={answer.tool} label={t.hideTool} />
      </div>

      {/* Other readings, when the tool says its answer is a guess among several.

          Shown inline rather than behind a control. The transliterator is the case that motivates
          this: Arabizi is genuinely ambiguous, the runner-up is often the one the user meant, and
          presenting a single reading as settled would be worse than admitting the choice. A tool
          that emits no alternatives renders nothing here. */}
      {alternatives.length > 0 && (
        <p className="mt-1.5 text-sm" style={{ color: 'var(--fg-muted)' }}>
          <span className="text-xs">{t.alternatives}: </span>
          <bdi>{alternatives.join(' · ')}</bdi>
        </p>
      )}

      {/* Only present when the value has a time dimension. Arithmetic has none; an exchange rate
          always does, and a rate shown without its age is the failure this whole component is
          built to prevent. */}
      {stale && (
        <p className="mt-1.5 text-xs" style={{ color: 'var(--fg-faint)' }}>
          {t.asOf} {stale}
        </p>
      )}

      {/* Mutually exclusive with the timestamp above by construction: an administered price has no
          `as_of`, because it was never measured. Saying so is the point — a reader who assumes the
          number was checked today would have no way to discover otherwise. */}
      {administered && (
        <p className="mt-1.5 text-xs" style={{ color: 'var(--fg-faint)' }}>
          {t.administered}
        </p>
      )}
    </section>
  )
}
