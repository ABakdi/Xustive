'use server'

import { cookies } from 'next/headers'

import { DENSITY_COOKIE, THEME_COOKIE, THEMES, type Theme } from './theme'

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
