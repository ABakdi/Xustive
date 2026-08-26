import { NextRequest, NextResponse } from 'next/server'

/**
 * Image proxy for the knowledge panel.
 *
 * The panel's thumbnail is fetched here, server-side, and streamed back — so a reader's IP and the
 * entity they looked at never reach Wikimedia. The host is allow-listed (Wikimedia only) so this
 * cannot be turned into an open proxy or an SSRF vector: the `u` parameter is not trusted, it is
 * validated against a fixed host set before any request is made.
 */

// Widened by one host for M8: the knowledge harvester stores Commons `Special:FilePath` URLs,
// which redirect to `upload.wikimedia.org` and survive the re-uploads that break a hardcoded
// upload URL. ADR-0019 requires this list to grow one named host at a time, never by pattern.
const ALLOWED_HOSTS = new Set(['upload.wikimedia.org', 'commons.wikimedia.org'])
const UA = 'XustiveKnowledge/0.1 (+https://xustive.dz; contact via repository)'
const MAX_BYTES = 5 * 1024 * 1024
const FETCH_TIMEOUT_MS = 6000
/** Enough for Special:FilePath's one hop, with room to spare; not enough to loop. */
const MAX_REDIRECTS = 5

export async function GET(req: NextRequest) {
  const raw = req.nextUrl.searchParams.get('u')
  if (!raw) return new NextResponse(null, { status: 400 })

  let target: URL
  try {
    target = new URL(raw)
  } catch {
    return new NextResponse(null, { status: 400 })
  }
  if (target.protocol !== 'https:' || !ALLOWED_HOSTS.has(target.hostname)) {
    return new NextResponse(null, { status: 400 })
  }

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS)
  try {
    // Redirects are followed by hand so every hop is re-validated against the allowlist.
    // `Special:FilePath` exists to redirect, so this stopped being hypothetical the moment
    // commons.wikimedia.org was added: following blindly would let a redirect chain reach any
    // host at all, which is the SSRF shape this route was written to prevent
    // ([[Security and Privacy]] §4).
    let current = target
    let res = await fetch(current.toString(), {
      headers: { 'User-Agent': UA },
      signal: controller.signal,
      redirect: 'manual',
      next: { revalidate: 86400 },
    })
    for (let hop = 0; hop < MAX_REDIRECTS && res.status >= 300 && res.status < 400; hop++) {
      const location = res.headers.get('location')
      if (!location) break
      const next = new URL(location, current)
      if (next.protocol !== 'https:' || !ALLOWED_HOSTS.has(next.hostname)) {
        return new NextResponse(null, { status: 400 })
      }
      current = next
      res = await fetch(current.toString(), {
        headers: { 'User-Agent': UA },
        signal: controller.signal,
        redirect: 'manual',
        next: { revalidate: 86400 },
      })
    }
    if (!res.ok) return new NextResponse(null, { status: 502 })

    const type = res.headers.get('content-type') ?? ''
    const length = Number(res.headers.get('content-length') ?? 0)
    if (!type.startsWith('image/') || length > MAX_BYTES) {
      return new NextResponse(null, { status: 502 })
    }
    const body = await res.arrayBuffer()
    if (body.byteLength > MAX_BYTES) return new NextResponse(null, { status: 502 })

    return new NextResponse(body, {
      headers: {
        'Content-Type': type,
        'Cache-Control': 'public, max-age=86400',
        'Referrer-Policy': 'no-referrer',
      },
    })
  } catch {
    return new NextResponse(null, { status: 502 })
  } finally {
    clearTimeout(timer)
  }
}
