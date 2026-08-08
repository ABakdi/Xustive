'use server'

import { cookies } from 'next/headers'

import { DENSITY_COOKIE, THEME_COOKIE, THEMES, type Theme } from './theme'
import { TOOLS_COOKIE, parseDisabled, serialiseDisabled } from './tools'

/**
 * Preferences are cookies, set by a Server Action.
 *
 * Not `localStorage`: the theme has to be known before the first byte, and anything the server
 * cannot read produces a flash of the wrong theme. That flash is the most common dark-mode defect
 * and it reads as a bug every time.
 *
 * A year, `lax`, and no `httpOnly` — the pre-paint script reads the resolved value back. None of
 * this is sensitive; it is a colour preference.
 */
const OPTIONS = {
  maxAge: 60 * 60 * 24 * 365,
  sameSite: 'lax',
  path: '/',
} as const

export async function setTheme(theme: Theme) {
  if (!THEMES.includes(theme)) return
  ;(await cookies()).set(THEME_COOKIE, theme, OPTIONS)
}

export async function setDensity(density: string) {
  if (density !== 'comfortable' && density !== 'compact') return
  ;(await cookies()).set(DENSITY_COOKIE, density, OPTIONS)
}

/**
 * Switch one instant-answer tool off, or back on.
 *
 * A Server Action rather than a click handler, so the dismiss control on a card is a real form
 * button and works with JavaScript disabled — which is the whole no-JS path, and the tool cards
 * are server-rendered precisely so they survive it.
 *
 * Reads the current set and writes the whole thing back, because a cookie is a single value and
 * there is no partial update.
 */
export async function setToolEnabled(tool: string, enabled: boolean) {
  // Validated against the same shape the API guarantees. This value comes from a form post, so
  // it is untrusted input on its way into a header.
  if (!/^[a-z-]{1,32}$/.test(tool)) return

  const jar = await cookies()
  const disabled = parseDisabled(jar.get(TOOLS_COOKIE)?.value)
  if (enabled) {
    disabled.delete(tool)
  } else {
    disabled.add(tool)
  }
  jar.set(TOOLS_COOKIE, serialiseDisabled(disabled), OPTIONS)
}
