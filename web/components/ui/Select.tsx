import type { ComponentProps } from 'react'

/**
 * A labelled select.
 *
 * A **native** `<select>`, not a listbox built from divs. shadcn's Select is a Radix listbox —
 * good work, and the wrong choice here for three reasons that matter more than the styling it
 * buys:
 *
 * 1. It is a client component. This app renders results as markup and the language pickers on a
 *    translation card are not worth pulling a runtime onto the page for.
 * 2. A native select is the control every mobile browser already renders well, using the
 *    platform's own wheel or sheet. A custom listbox on a phone is uniformly worse.
 * 3. It works with JavaScript disabled, which a div-based listbox cannot.
 *
 * The trade is real: the option list cannot be styled. That is a price worth paying for a control
 * that behaves correctly on every device the engine is meant to reach.
 */
export function Select({
  label,
  className = '',
  ...props
}: ComponentProps<'select'> & { label: string }) {
  return (
    <label className="flex items-center gap-1.5">
      {/* Visible, not a placeholder. A select whose only label is its current value gives a
          screen-reader user no way to know what the value means. */}
      <span className="text-xs" style={{ color: 'var(--fg-faint)' }}>
        {label}
      </span>
      <select className={`chip cursor-pointer ${className}`.trim()} {...props} />
    </label>
  )
}
