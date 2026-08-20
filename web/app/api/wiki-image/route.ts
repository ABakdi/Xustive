import { NextRequest, NextResponse } from 'next/server'

/**
 * Image proxy for the knowledge panel.
 *
 * The panel's thumbnail is fetched here, server-side, and streamed back — so a reader's IP and the
 * entity they looked at never reach Wikimedia. The host is allow-listed (Wikimedia only) so this
 * cannot be turned into an open proxy or an SSRF vector: the `u` parameter is not trusted, it is
 * validated against a fixed host set before any request is made.
 */

const ALLOWED_HOSTS = new Set(['upload.wikimedia.org'])
const UA = 'XustiveKnowledge/0.1 (+https://xustive.dz; contact via repository)'
const MAX_BYTES = 5 * 1024 * 1024
const FETCH_TIMEOUT_MS = 6000

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
    const res = await fetch(target.toString(), {
      headers: { 'User-Agent': UA },
      signal: controller.signal,
      next: { revalidate: 86400 },
    })
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
