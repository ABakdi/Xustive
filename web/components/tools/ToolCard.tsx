import type { InstantAnswer } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

import { CopyButton } from './CopyButton'

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
  const stale = answer.as_of
    ? new Intl.DateTimeFormat(locale === 'ary' ? 'ar' : locale, {
        hour: '2-digit',
        minute: '2-digit',
        day: 'numeric',
        month: 'short',
      }).format(new Date(answer.as_of * 1000))
    : null

  return (
    <section className="assert mb-7" aria-label={t[answer.tool as keyof Messages] ?? answer.tool}>
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
      </div>

      {/* Only present when the value has a time dimension. Arithmetic has none; an exchange rate
          always does, and a rate shown without its age is the failure this whole component is
          built to prevent. */}
      {stale && (
        <p className="mt-1.5 text-xs" style={{ color: 'var(--fg-faint)' }}>
          {t.asOf} {stale}
        </p>
      )}
    </section>
  )
}
