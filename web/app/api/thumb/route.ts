import { NextResponse } from 'next/server'

import { isProxyableUrl, verifyThumb } from '@/lib/thumb'

/**
 * The signed thumbnail proxy (M9-T02, ADR-0021).
 *
 * `/api/thumb?u=<url>&s=<hmac>`. The browser asks us; we ask the crawled host; the reader's
 * address and referrer never reach it. A missing or wrong signature is refused before any fetch,
 * so this cannot be turned into an open proxy or an SSRF vector by anyone who is not our own
 * renderer — and even our own renderer cannot make it fetch a private host.
 *
 * On upstream failure it answers a transparent pixel rather than an error: a grid with holes in
 * it reads as broken, and the tile's title still links to the page.
 */

const UA = 'XustiveThumb/0.1 (+https://xustive.dz; contact via repository)'
const MAX_BYTES = 5 * 1024 * 1024
const FETCH_TIMEOUT_MS = 4000
const MAX_REDIRECTS = 4

/** A 1×1 transparent GIF, the honest answer when the upstream image is not available. */
const PIXEL = Buffer.from('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7', 'base64')

function pixel() {
  return new NextResponse(PIXEL, {
    headers: {
      'Content-Type': 'image/gif',
      // Short: a host that is down now may be up in a minute.
      'Cache-Control': 'public, max-age=60',
      'Referrer-Policy': 'no-referrer',
      'X-Content-Type-Options': 'nosniff',
    },
  })
}

export async function GET(req: Request) {
  const { searchParams } = new URL(req.url)
  const u = searchParams.get('u') ?? ''
  const s = searchParams.get('s') ?? ''

  // Signature first, before the URL is even looked at. Anything unsigned is refused with no work
  // done, which is what keeps this from being a resource anyone can spend.
  if (!u || !s || !verifyThumb(u, s)) {
    return new NextResponse(null, { status: 403 })
  }
  // Re-checked even though the signer already checked it: the signer's rule could change, and a
  // 400 here is cheaper than a request into a private network.
  if (!isProxyableUrl(u)) {
    return new NextResponse(null, { status: 400 })
  }

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS)
  try {
    let current = new URL(u)
    let res = await fetch(current.toString(), {
      headers: { 'User-Agent': UA, Accept: 'image/*' },
      signal: controller.signal,
      redirect: 'manual',
      next: { revalidate: 86400 },
    })
    // Redirects by hand so every hop is re-validated — the same rule as wiki-image, and more
    // necessary here because the origin is the open web.
    for (let hop = 0; hop < MAX_REDIRECTS && res.status >= 300 && res.status < 400; hop++) {
      const location = res.headers.get('location')
      if (!location) break
      const next = new URL(location, current)
      if (!isProxyableUrl(next.toString())) {
        return new NextResponse(null, { status: 400 })
      }
      current = next
      res = await fetch(current.toString(), {
        headers: { 'User-Agent': UA, Accept: 'image/*' },
        signal: controller.signal,
        redirect: 'manual',
        next: { revalidate: 86400 },
      })
    }
    if (!res.ok) return pixel()

    const type = res.headers.get('content-type') ?? ''
    const length = Number(res.headers.get('content-length') ?? 0)
    if (!type.startsWith('image/') || length > MAX_BYTES) return pixel()
    const body = await res.arrayBuffer()
    if (body.byteLength > MAX_BYTES || body.byteLength === 0) return pixel()

    return new NextResponse(body, {
      headers: {
        'Content-Type': type,
        'Cache-Control': 'public, max-age=86400',
        'Referrer-Policy': 'no-referrer',
        'X-Content-Type-Options': 'nosniff',
      },
    })
  } catch {
    return pixel()
  } finally {
    clearTimeout(timer)
  }
}
