import type { ReactNode } from 'react'

/**
 * Root layout.
 *
 * Deliberately thin: the real layout is per-locale, because direction and language have to be on
 * <html> and that element is owned by the innermost layout that renders it.
 */
export default function RootLayout({ children }: { children: ReactNode }) {
  return children
}
