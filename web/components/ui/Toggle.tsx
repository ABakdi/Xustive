import { Button } from './Button'

/**
 * An on/off control that submits.
 *
 * shadcn's Switch is a Radix component backed by a hidden input and a `role="switch"` div. This is
 * a `<button>` inside a form instead, because the setting it drives is stored server-side in a
 * cookie: pressing it posts a Server Action, and the new state comes back in the re-render. There
 * is no client state to keep, so there is nothing for a client component to do.
 *
 * That also makes it work with JavaScript disabled, which is the whole reason the preference is a
 * cookie rather than `localStorage`.
 *
 * # Why not `role="switch"`
 *
 * A `role="switch"` promises a control that flips in place. This one submits a form and the page
 * re-renders, which is a button. `aria-pressed` says the same thing honestly: a toggle button
 * whose state is announced, without claiming an interaction model it does not implement.
 */
export function Toggle({
  on,
  onLabel,
  offLabel,
  describedBy,
  accessibleLabel,
}: {
  on: boolean
  onLabel: string
  offLabel: string
  /** Optional id of the element naming what is being toggled. */
  describedBy?: string
  /** Full label, since "On" alone does not say what is on. */
  accessibleLabel: string
}) {
  return (
    <Button
      type="submit"
      variant={on ? 'emphasis' : 'default'}
      // The state, not the action. A toggle labelled with what pressing it *does* is ambiguous
      // about what is true now, which is the more important of the two — and `aria-pressed`
      // carries the state to a screen reader either way.
      aria-pressed={on}
      aria-label={accessibleLabel}
      aria-describedby={describedBy}
    >
      {on ? onLabel : offLabel}
    </Button>
  )
}
