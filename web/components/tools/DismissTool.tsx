import { Button } from '@/components/ui/Button'
import { setToolEnabled } from '@/lib/prefs'

/**
 * "Don't show this again" on a tool card.
 *
 * Put on the card rather than only in settings because that is where the reader is at the moment
 * they are annoyed. A control that requires finding a settings page first is a control most
 * people never use, and an instant answer they did not want is a small recurring irritation
 * rather than a thing worth going looking for.
 *
 * A real `<form>` with a Server Action, not a click handler. The card is server-rendered so it
 * works without JavaScript; a dismiss button that needed JavaScript would be the one part of it
 * that did not.
 */
export function DismissTool({ tool, label }: { tool: string; label: string }) {
  async function dismiss() {
    'use server'
    await setToolEnabled(tool, false)
  }

  return (
    <form action={dismiss} className="contents">
      {/* Quiet, and only legible on hover or focus. It is a control for the rare case where a
          tool is unwanted, and drawing the eye to it on every card would make every answer look
          provisional. Never hidden from a keyboard: `focus-visible` brings it back. */}
      <Button
        type="submit"
        variant="quiet"
        className="opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
      >
        {label}
      </Button>
    </form>
  )
}
