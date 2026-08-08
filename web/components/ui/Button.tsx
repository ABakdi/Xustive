import Link from 'next/link'
import type { ComponentProps, ReactNode } from 'react'

/**
 * The button.
 *
 * # Why these are written rather than installed
 *
 * shadcn/ui is the reference for the *structure* here — the variant prop, the `focus-visible`
 * ring, keeping state off colour alone. It is not the source of the code, for one concrete
 * reason: its primitives are Radix components, and Radix components are client components.
 * Installing them would push `'use client'` into the result page, and the result list shipping as
 * markup is the single most valuable property this frontend has. A `<button>` in a form does not
 * need a runtime.
 *
 * So: no dependency, no `'use client'`, no `cva`. A `<button>` element with a class.
 *
 * The visual language is this project's, not shadcn's default. Radius is 2px, not 6px; the
 * emphasis variant fills with the foreground colour rather than a brand purple; there is no
 * shadow.
 */
type Variant = 'default' | 'emphasis' | 'quiet'

/** Variant classes. Deliberately a lookup, not a `cva` chain — there are three of them. */
const VARIANT: Record<Variant, string> = {
  default: 'chip',
  // Fill plus border, so the state survives a forced-colours mode that drops backgrounds.
  emphasis: 'chip chip-active',
  // No border, no fill. For controls that should be reachable but not draw the eye — dismissing a
  // tool card, for instance, where a bordered button on every card would make every answer look
  // provisional.
  quiet: 'btn-quiet',
}

type ButtonProps = ComponentProps<'button'> & {
  variant?: Variant
  children: ReactNode
}

export function Button({ variant = 'default', className = '', ...props }: ButtonProps) {
  return (
    <button
      // `type` defaults to `submit` in HTML, which is right inside the Server Action forms this
      // app uses and wrong everywhere else — so it is left to the caller rather than defaulted
      // here, where a wrong guess would silently submit a form.
      className={`${VARIANT[variant]} cursor-pointer ${className}`.trim()}
      {...props}
    />
  )
}

type LinkButtonProps = ComponentProps<typeof Link> & {
  variant?: Variant
  children: ReactNode
}

/**
 * A button that is a link.
 *
 * Separate rather than an `asChild` polymorph. Navigation and action are different things and
 * should not be one component with a prop: a link must be a real `<a>` so it opens in a new tab,
 * shows its destination on hover, and works when JavaScript does not.
 */
export function LinkButton({ variant = 'default', className = '', ...props }: LinkButtonProps) {
  return <Link className={`${VARIANT[variant]} no-underline ${className}`.trim()} {...props} />
}
