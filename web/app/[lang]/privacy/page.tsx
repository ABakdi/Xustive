import Link from 'next/link'
import { notFound } from 'next/navigation'

import { Wordmark } from '@/components/layout/Wordmark'
import { isLocale } from '@/lib/i18n/config'
import { messages } from '@/lib/i18n/messages'

/**
 * The privacy page.
 *
 * Its whole job is to state, plainly and per deployment mode, what the engine stores and what it
 * never stores (M7-T10.6, reconciling [[ADR-0018 - Anonymous Search History]]). The home page's
 * one-line claim links here for the detail behind it — a search engine that promises "never linked
 * to you" owes the reader the specifics of what that does and does not mean.
 */
export async function generateMetadata({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params
  if (!isLocale(lang)) return {}
  return { title: messages(lang).privacyTitle }
}

export default async function Privacy({ params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params
  if (!isLocale(lang)) notFound()
  const t = messages(lang)

  return (
    <main className="mx-auto max-w-xl px-[var(--pad)] py-16">
      {/* Wordmark is itself a link home — wrapping it in another Link nested <a> inside <a>
          (invalid HTML, hydration warnings). BUG-014. */}
      <Wordmark lang={lang} size="sm" />

      <h1 className="mt-10 text-2xl font-semibold">{t.privacyTitle}</h1>
      <p className="mt-3 text-base" style={{ color: 'var(--fg-muted)' }}>
        {t.privacyLead}
      </p>

      <div className="mt-8 space-y-5 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <p>{t.privacyStored}</p>
        <p>{t.privacyNotStored}</p>
        <p style={{ color: 'var(--fg-faint)' }}>{t.privacyModeNote}</p>
        {/* BOTH optional third-party egresses, documented plainly (ADR-0017/T08.3, BUG-037): what
            leaves, to whom, and that each is off unless the operator turns it on. Presenting the
            summariser as the only one, while federation also sends query text outward, was a lie
            by omission on the one page whose job is precision. */}
        <p style={{ color: 'var(--fg-faint)' }}>{t.privacyFederationNote}</p>
        <p style={{ color: 'var(--fg-faint)' }}>{t.privacyExternalNote}</p>
      </div>

      <p className="mt-10 text-sm">
        <Link href={`/${lang}`} style={{ color: 'var(--accent)' }}>
          {t.privacyBack}
        </Link>
      </p>
    </main>
  )
}
