import type { Viewport } from 'next'
import type { ReactNode } from 'react'

/**
 * The viewport, declared rather than inherited ([[UI - Responsive]] §2.9): `dvh` and the safe-area
 * insets both need `viewportFit: 'cover'`, and `maximumScale` is deliberately absent — pinching
 * to zoom is how people read on a phone and blocking it is an accessibility failure.
 */
export const viewport: Viewport = {
  width: 'device-width',
  initialScale: 1,
  viewportFit: 'cover',
}

/**
 * Root layout.
 *
 * Deliberately thin: the real layout is per-locale, because direction and language have to be on
 * <html> and that element is owned by the innermost layout that renders it.
 */
export default function RootLayout({ children }: { children: ReactNode }) {
  return children
}
