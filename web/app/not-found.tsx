import Link from 'next/link'

import './globals.css'

/**
 * The global not-found. The root layout is deliberately thin (it renders `{children}` so the real
 * `<html>` can live in the innermost layout), which means an unmatched route has no layout to
 * supply those tags — so this page carries its own. Without it, a bad URL fails with Next's
 * "missing root layout tags" runtime error instead of showing a 404.
 */
export default function NotFound() {
  return (
    <html lang="en">
      <body>
        <main
          style={{
            minHeight: '100dvh',
            display: 'grid',
            placeItems: 'center',
            padding: '2rem',
            color: 'var(--fg)',
            background: 'var(--bg)',
            textAlign: 'center',
          }}
        >
          <div>
            <p style={{ fontSize: '3rem', fontWeight: 600, margin: 0 }}>404</p>
            <p style={{ color: 'var(--fg-muted)', margin: '0.5rem 0 1.5rem' }}>
              This page does not exist.
            </p>
            <Link href="/" style={{ color: 'var(--accent)' }}>
              Go home
            </Link>
          </div>
        </main>
      </body>
    </html>
  )
}
