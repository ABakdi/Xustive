'use client'

import { useEffect, useRef, useState } from 'react'

/**
 * A banner shown when the browser loses its connection (M1B-T03.6, the offline state).
 *
 * Offline is a *client* condition — the server that would render an error page is the very thing
 * that cannot be reached — so this is the one piece of state the Server Components cannot own. It
 * listens to the browser's own `online`/`offline` events and `navigator.onLine`, which is exactly
 * what those signals are for.
 *
 * The query is never lost: this is a banner over the page, not a replacement for it. Whatever
 * results are already on screen stay, and a search typed while offline simply waits — the form
 * still submits when the connection returns, because the page never navigated away.
 *
 * It shows a brief "back online" confirmation on reconnect rather than vanishing silently, so the
 * transition is legible: a banner that disappears the instant you glance away leaves you unsure
 * whether anything changed.
 */
export function OfflineBanner({
  offline,
  hint,
  backOnline,
}: {
  offline: string
  hint: string
  backOnline: string
}) {
  // `online` starts true and is corrected in the effect. Rendering "offline" during SSR would flash
  // the banner for every reader on the first paint, since the server cannot know the connection
  // state — so the initial value is the common case and the effect fixes the rare one.
  const [online, setOnline] = useState(true)
  const [showBack, setShowBack] = useState(false)
  const wasOffline = useRef(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  useEffect(() => {
    setOnline(navigator.onLine)

    const goOffline = () => {
      wasOffline.current = true
      setOnline(false)
      setShowBack(false)
    }
    const goOnline = () => {
      setOnline(true)
      // Only confirm a *recovery* — not the first `online` event on a page that was never offline.
      if (wasOffline.current) {
        wasOffline.current = false
        setShowBack(true)
        clearTimeout(timer.current)
        timer.current = setTimeout(() => setShowBack(false), 3000)
      }
    }

    window.addEventListener('offline', goOffline)
    window.addEventListener('online', goOnline)
    return () => {
      window.removeEventListener('offline', goOffline)
      window.removeEventListener('online', goOnline)
      clearTimeout(timer.current)
    }
  }, [])

  if (online && !showBack) return null

  const recovered = online && showBack
  return (
    <div
      // Assertive while offline (the user needs to know now); polite for the recovery note.
      role="status"
      aria-live={recovered ? 'polite' : 'assertive'}
      className="fixed inset-x-0 top-0 z-toast px-4 py-2 text-center text-sm"
      style={{
        background: recovered ? 'var(--accent-wash)' : 'var(--warn)',
        color: recovered ? 'var(--accent)' : 'var(--accent-fg)',
      }}
    >
      {recovered ? (
        backOnline
      ) : (
        <>
          <strong>{offline}</strong>
          <span className="ms-2" style={{ opacity: 0.9 }}>
            {hint}
          </span>
        </>
      )}
    </div>
  )
}
