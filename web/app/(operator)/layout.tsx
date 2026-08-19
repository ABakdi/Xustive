import type { Metadata } from 'next'
import type { ReactNode } from 'react'

import '../fonts.css'
import '../globals.css'
import { readDensity, readTheme, THEME_SCRIPT } from '@/lib/theme'

export const metadata: Metadata = {
  title: 'Xustive',
  robots: { index: false, follow: false },
}

/**
 * Root layout for the operator surfaces (`/admin`, `/bot`) — the pages that are not part of the
 * localised product. The app's real `<html>`/`<body>` live in the innermost layout that owns them,
 * and for localised routes that is `[lang]/layout`; these routes are outside `[lang]`, so this route
 * group provides their own. English and LTR, always. Theme and density are read server-side so the
 * first byte already carries them and there is no flash.
 */
export default async function OperatorLayout({ children }: { children: ReactNode }) {
  const [theme, density] = await Promise.all([readTheme(), readDensity()])
  return (
    // The pre-paint script rewrites data-theme before hydration; suppress the expected mismatch
    // on this node's own attributes (see [lang]/layout for the same reasoning).
    <html
      lang="en"
      dir="ltr"
      data-theme={theme}
      data-density={density}
      suppressHydrationWarning
    >
      <head>
        {/* Set the theme before paint, not in an effect — an effect is what causes the flash. */}
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>{children}</body>
    </html>
  )
}
