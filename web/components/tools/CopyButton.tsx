'use client'

import { Check, Copy } from 'lucide-react'
import { useState } from 'react'

/**
 * Copies the answer alone, never the frame.
 *
 * The only interactive part of a tool card today, so it is the only thing that hydrates — the
 * answer itself is server-rendered and readable before any JavaScript runs.
 */
export function CopyButton({
  value,
  label,
  copied,
}: {
  value: string
  label: string
  copied: string
}) {
  const [done, setDone] = useState(false)

  return (
    <button
      type="button"
      className="ghost shrink-0"
      // The label announces the *result* once copied, because a screen-reader user gets no
      // visual confirmation from the icon changing.
      aria-label={done ? copied : label}
      title={done ? copied : label}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value)
          setDone(true)
          setTimeout(() => setDone(false), 1600)
        } catch {
          // A clipboard permission refusal is not worth an error state. The answer is selectable
          // text; the button is a convenience on top of that.
        }
      }}
    >
      {done ? <Check size={15} aria-hidden /> : <Copy size={15} aria-hidden />}
    </button>
  )
}
