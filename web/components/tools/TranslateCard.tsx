'use client'

import { useCallback, useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/Button'
import { Select } from '@/components/ui/Select'
import type { TranslateLanguage as Language } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

type Detail = { text?: string; from?: string | null; to?: string; pending?: boolean }

/**
 * The translation card.
 *
 * The **one** client component in the instant-answer path, and the only one that genuinely needs
 * to be. Everything else on a results page is server-rendered; this streams token by token and
 * has to be cancellable, neither of which a form post can do.
 *
 * Without JavaScript there is no card at all rather than a broken one — see the server component
 * that decides this, which renders the plain answer instead. A translation is a request the reader
 * makes deliberately, so the degradation is honest: the feature is absent, not silently wrong.
 *
 * # Cancellation is not decorative
 *
 * Aborting the fetch closes the connection, which drops the receiver in the API, which stops the
 * model worker on its next token. With two slots on a 4 GB card, a generation nobody is reading is
 * half the capacity. Changing the target language, editing the text, or leaving the page all abort.
 */
export function TranslateCard({
  detail,
  t,
  uiLang,
  languages,
}: {
  detail: Detail
  t: Messages
  uiLang: string
  languages: Language[]
}) {
  const [to, setTo] = useState(detail.to ?? 'ar')
  const [from, setFrom] = useState(detail.from ?? '')
  const [out, setOut] = useState('')
  const [state, setState] = useState<'idle' | 'running' | 'done' | 'failed' | 'truncated'>('idle')
  const abort = useRef<AbortController | null>(null)

  const text = detail.text ?? ''
  const name = (l: Language) =>
    uiLang === 'ar' || uiLang === 'ary' ? l.name_ar : uiLang === 'en' ? l.name_en : l.name_fr

  const run = useCallback(async () => {
    // Any previous run is abandoned before a new one starts. Two streams writing into one box
    // would interleave their tokens into nonsense.
    abort.current?.abort()
    const controller = new AbortController()
    abort.current = controller

    setOut('')
    setState('running')

    try {
      const res = await fetch('/api/v1/translate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        // A POST body, never a query string. This is the most sensitive field the service
        // handles, and a URL ends up in referrers, histories and access logs.
        body: JSON.stringify({ text, from: from || null, to }),
        signal: controller.signal,
      })
      if (!res.ok || !res.body) {
        setState('failed')
        return
      }

      const reader = res.body.getReader()
      const decoder = new TextDecoder()
      let buffer = ''

      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        // A chunk boundary can fall anywhere, including mid-frame and mid-character. `stream:true`
        // holds a partial character back; the buffer holds a partial frame back.
        buffer += decoder.decode(value, { stream: true })

        // SSE separates messages with a blank line. The trailing fragment stays buffered.
        const frames = buffer.split('\n\n')
        buffer = frames.pop() ?? ''

        for (const frame of frames) {
          const line = frame.split('\n').find((l) => l.startsWith('data:'))
          if (!line) continue // a keep-alive comment
          let payload: { type?: string; text?: string; truncated?: boolean }
          try {
            payload = JSON.parse(line.slice(5).trim())
          } catch {
            continue
          }
          if (payload.type === 'delta' && payload.text) {
            setOut((prev) => prev + payload.text)
          } else if (payload.type === 'done') {
            setState(payload.truncated ? 'truncated' : 'done')
            return
          } else if (payload.type === 'error') {
            setState('failed')
            return
          }
        }
      }
      // The stream ended without a terminal frame.
      setState((s) => (s === 'running' ? 'failed' : s))
    } catch (error) {
      // An abort is the expected end of a cancelled run, not a failure to report.
      if ((error as Error)?.name !== 'AbortError') setState('failed')
    }
  }, [text, from, to])

  useEffect(() => {
    if (!text) return
    void run()
    // Aborts on unmount and before any re-run, which is what makes leaving the page free a model
    // slot rather than leave one generating into nothing.
    return () => abort.current?.abort()
  }, [run, text])

  const target = languages.find((l) => l.code === to)
  const approximate = target?.approximate ?? false

  return (
    <section className="assert group mb-7" aria-label={t.translate}>
      <p className="m-0 text-xs" style={{ color: 'var(--fg-muted)' }}>
        <bdi>{text}</bdi>
      </p>

      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
        <Select
          label={t.translateFrom}
          value={from}
          onChange={(e) => setFrom(e.target.value)}
        >
          <option value="">{t.translateAuto}</option>
          {languages.map((l) => (
            <option key={l.code} value={l.code}>
              {name(l)}
            </option>
          ))}
        </Select>
        <Select label={t.translateTo} value={to} onChange={(e) => setTo(e.target.value)}>
          {languages.map((l) => (
            <option key={l.code} value={l.code}>
              {name(l)}
            </option>
          ))}
        </Select>
        {state === 'running' && (
          <Button
            type="button"
            onClick={() => {
              abort.current?.abort()
              setState('done')
            }}
          >
            {t.stop}
          </Button>
        )}
      </div>

      {/* `aria-live="polite"` rather than `assertive`: a screen reader should not re-announce on
          every token. `dir="auto"` because the output direction follows the target language, not
          the interface — translating into Arabic from an English UI is the ordinary case. */}
      <p
        className="mt-2.5 mb-0 text-lg leading-relaxed"
        style={{ fontWeight: 500, minBlockSize: '1.6em' }}
        dir="auto"
        aria-live="polite"
        aria-busy={state === 'running'}
      >
        <bdi>{out}</bdi>
        {state === 'running' && !out && (
          <span style={{ color: 'var(--fg-faint)' }}>{t.translating}</span>
        )}
      </p>

      <p className="mt-2 mb-0 text-xs" style={{ color: 'var(--fg-faint)' }}>
        {/* Stated on every translation, not only the doubtful ones. A 3B model translates well
            between its major languages and poorly between rare pairs, and it has no way to signal
            which case it is in — so the card never presents output as authoritative. */}
        {approximate ? t.translateApprox : t.translateLocal}
        {state === 'truncated' && ` · ${t.translateTruncated}`}
        {state === 'failed' && ` · ${t.translateFailed}`}
      </p>
    </section>
  )
}
