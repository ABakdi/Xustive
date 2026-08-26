import 'server-only'

import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto'

/**
 * Signed thumbnail URLs (M9-T02, [[ADR-0021 - Proxied Thumbnails with Signed URLs]]).
 *
 * The results page decides which thumbnails to render, so it vouches for each one: an HMAC over
 * the upstream URL with a secret the browser never sees. The proxy route serves only what carries
 * a valid signature, which is what makes it not an open proxy — anyone can *call* it, but only
 * URLs our own render chose will fetch.
 *
 * `server-only` is load-bearing. Importing this from a client component would ship the secret.
 */

/**
 * The secret: configured, or random per process.
 *
 * Random is the safe default, not a fallback that weakens anything. Without configuration the
 * failure is "thumbnails signed by one process are refused by another" — visible, harmless, and
 * fixed by setting the variable — never "anyone can use the proxy". A multi-instance deployment
 * must set it; ADR-0021 says so.
 */
//
// Held on `globalThis`, not at module level. Next compiles server components and route handlers
// as separate bundles, each with its own instance of this module — so a module-level random
// value is *two* random values, and every signature the page produces is one the route refuses.
// The process is one process; `globalThis` is what the two bundles actually share.
declare global {
  // eslint-disable-next-line no-var
  var __xustiveThumbSecret: Buffer | undefined
}
const SECRET: Buffer = process.env.XUSTIVE_THUMB_SECRET
  ? Buffer.from(process.env.XUSTIVE_THUMB_SECRET, 'utf8')
  : (globalThis.__xustiveThumbSecret ??= randomBytes(32))

/** Hosts that may be proxied. Everything else is refused before any request is made. */
export function isProxyableUrl(raw: string): boolean {
  let u: URL
  try {
    u = new URL(raw)
  } catch {
    return false
  }
  if (u.protocol !== 'https:') return false
  if (u.username || u.password) return false
  const host = u.hostname.toLowerCase()
  // No IP literals and no private names. A crawled page can carry an <img> pointing anywhere,
  // including inside our own network, and a signature only proves *we* rendered it — it does not
  // make the destination safe. DNS rebinding is not fully closed by this; see ADR-0021.
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host)) return false
  if (host.startsWith('[') || host.includes(':')) return false
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local')) return false
  if (host.endsWith('.internal') || host.endsWith('.lan') || !host.includes('.')) return false
  return true
}

function hmac(url: string): string {
  return createHmac('sha256', SECRET).update(url).digest('base64url')
}

/** The same-origin URL to put in an `<img src>`, or `null` for a URL that must not be proxied. */
export function signThumb(upstream: string): string | null {
  if (!isProxyableUrl(upstream)) return null
  return `/api/thumb?u=${encodeURIComponent(upstream)}&s=${hmac(upstream)}`
}

/** Whether a signature was produced by this process for this URL. Constant-time. */
export function verifyThumb(upstream: string, signature: string): boolean {
  const expected = Buffer.from(hmac(upstream))
  const given = Buffer.from(signature)
  return expected.length === given.length && timingSafeEqual(expected, given)
}
