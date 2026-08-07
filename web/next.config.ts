import type { NextConfig } from 'next'

/**
 * The API is a separate Rust process; this never proxies it with added logic.
 *
 * The rewrite exists so the browser talks to one origin — which is what lets the CSP stay
 * `default-src 'self'` and keeps a cross-origin preflight off the suggestion path, where a
 * per-keystroke OPTIONS round trip would cost more than the request itself.
 */
const API = process.env.XUSTIVE_API_URL ?? 'http://127.0.0.1:8080'

const config: NextConfig = {
  reactStrictMode: true,
  poweredByHeader: false,
  // Fewer bytes and one less thing that can differ between dev and prod.
  compress: true,

  async rewrites() {
    return [{ source: '/api/v1/:path*', destination: `${API}/api/v1/:path*` }]
  },

  async headers() {
    return [
      {
        source: '/:path*',
        headers: [
          // Search result links carry the query in the referring URL. Without this, every
          // destination site learns what the user searched for — the single largest privacy
          // leak a search engine can have, and free to prevent.
          { key: 'Referrer-Policy', value: 'no-referrer' },
          { key: 'X-Content-Type-Options', value: 'nosniff' },
          { key: 'Cross-Origin-Opener-Policy', value: 'same-origin' },
          { key: 'Permissions-Policy', value: 'geolocation=(), microphone=(self), camera=(self)' },
        ],
      },
    ]
  },
}

export default config
