import { cookies } from 'next/headers'

export const THEMES = ['system', 'light', 'dark'] as const
export type Theme = (typeof THEMES)[number]

export const DENSITIES = ['comfortable', 'compact'] as const
export type Density = (typeof DENSITIES)[number]

export const THEME_COOKIE = 'xustive-theme'
export const DENSITY_COOKIE = 'xustive-density'

/**
 * Read the theme server-side.
 *
 * Resolved before the first byte so there is no flash of the wrong theme — the single most common
 * dark-mode defect, and one that reads as a bug every time. `system` is reconciled against
 * `prefers-color-scheme` by a tiny pre-paint script; everything else is decided here.
 */
export async function readTheme(): Promise<Theme> {
  const value = (await cookies()).get(THEME_COOKIE)?.value
  return THEMES.includes(value as Theme) ? (value as Theme) : 'system'
}

export async function readDensity(): Promise<Density> {
  const value = (await cookies()).get(DENSITY_COOKIE)?.value
  return DENSITIES.includes(value as Density) ? (value as Density) : 'comfortable'
}

/**
 * Pre-paint script that resolves `system` against the OS preference.
 *
 * The only inline script on the page, and it needs a CSP hash rather than `unsafe-inline`.
 * It runs before paint deliberately: doing this in an effect is what causes the flash.
 */
export const THEME_SCRIPT = `(function(){try{var t=document.documentElement.dataset.theme;if(t==='system'||!t){document.documentElement.dataset.theme=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'}}catch(e){}})()`
