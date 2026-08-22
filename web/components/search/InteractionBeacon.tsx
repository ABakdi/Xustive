'use client'

import { useEffect, useRef } from 'react'

/**
 * The anonymous click beacon ([[Interaction Signals]], M6-T03.3).
 *
 * One delegated click listener around the whole result list — not a handler per link — so the
 * server-rendered result markup stays untouched: every anchor keeps its real `href`, there is no
 * redirect and no `ping` attribute. When a result link is clicked, this reads the anchor's
 * `data-doc` and fires `navigator.sendBeacon('/api/v1/interaction', {t, d})`.
 *
 * # Why it never gets in the way
 *
 * `sendBeacon` is fire-and-forget: it queues the request and returns immediately, so navigation is
 * never delayed or gated on it. If the beacon is blocked, `sendBeacon` is missing, or JavaScript is
 * off entirely, the link still works and nothing is recorded — that is the whole degradation story
 * (M6-T03.4). No query text is ever in the request; only the opaque token and the document id.
 *
 * Renders nothing and wraps its children (the result list). Absent from the tree when there is no
 * `token` (interaction signals off), so there is then no listener at all.
 */
export function InteractionBeacon({
  token,
  children,
}: {
  token: string
  children: React.ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const root = ref.current
    if (!root || typeof navigator === 'undefined' || !navigator.sendBeacon) return

    function onClick(e: MouseEvent) {
      // Find the result anchor the click landed on (or inside).
      const anchor = (e.target as HTMLElement | null)?.closest('a[data-doc]')
      const doc = anchor?.getAttribute('data-doc')
      if (!doc) return
      try {
        const body = new Blob([JSON.stringify({ t: token, d: doc })], {
          type: 'application/json',
        })
        navigator.sendBeacon('/api/v1/interaction', body)
      } catch {
        // Beacon failures are never surfaced — the click's real job is the navigation, which the
        // browser is already doing.
      }
    }

    // Capture phase, so it still fires when the anchor handles the event and navigates away.
    root.addEventListener('click', onClick, { capture: true })
    return () => root.removeEventListener('click', onClick, { capture: true })
  }, [token])

  return <div ref={ref}>{children}</div>
}
