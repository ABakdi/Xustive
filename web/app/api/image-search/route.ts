import { NextRequest, NextResponse } from 'next/server'

import { signThumb, verifyThumb } from '@/lib/thumb'

/**
 * The web tier's half of reverse image search ([[Milestone 10 - Reverse Image Search]]).
 *
 * The API answers with upstream image URLs; only a server can sign them for the thumbnail proxy
 * (ADR-0021), and the results page is a client island. So the island talks to this route, which:
 *
 * - `POST` (raw image body): forwards to `POST /api/v1/search/image` and signs every thumbnail;
 * - `GET ?u=<url>&s=<sig>`: the URL leg (M10-T03.5) — a picture already on the Images tab. The
 *   bytes are fetched through our own thumbnail proxy, which applies its rules (https, no
 *   private hosts, 5 MB, 4 s, image/* only), then posted like an upload. Nothing is uploaded and
 *   the reader never leaves the tab;
 * - `GET ?web=<description>`: the web group, forwarded to `GET /api/v1/search/image/web` and
 *   signed. Words only — the picture never goes this way (ADR-0028).
 *
 * The image bytes pass through in memory and are not logged, cached or stored.
 */
const API = process.env.XUSTIVE_API_URL ?? 'http://127.0.0.1:8080'
const MAX_BYTES = 5 * 1024 * 1024

type Hit = { url: string; thumb_url?: string; [k: string]: unknown }

function sign(hits: Hit[]): Hit[] {
  return hits.flatMap((h) => {
    const signed = signThumb(h.thumb_url ?? h.url)
    // An image the proxy will not serve is not shown: a tile with a broken picture is noise.
    return signed ? [{ ...h, thumb: signed }] : []
  })
}

async function forward(body: ArrayBuffer, type: string): Promise<NextResponse> {
  const res = await fetch(`${API}/api/v1/search/image`, {
    method: 'POST',
    headers: { 'Content-Type': type || 'application/octet-stream' },
    body,
    cache: 'no-store',
  })
  if (!res.ok) return new NextResponse(null, { status: res.status === 503 ? 503 : 502 })
  const data = (await res.json()) as { images: Hit[] } & Record<string, unknown>
  return NextResponse.json({ ...data, images: sign(data.images ?? []) }, { headers: { 'Cache-Control': 'no-store' } })
}

export async function POST(req: NextRequest) {
  const body = await req.arrayBuffer()
  if (body.byteLength === 0) return new NextResponse(null, { status: 400 })
  if (body.byteLength > MAX_BYTES) return new NextResponse(null, { status: 413 })
  return forward(body, req.headers.get('content-type') ?? '')
}

export async function GET(req: NextRequest) {
  const { searchParams } = req.nextUrl
  const web = searchParams.get('web')
  if (web !== null) {
    const res = await fetch(`${API}/api/v1/search/image/web?q=${encodeURIComponent(web)}`, {
      cache: 'no-store',
    })
    if (!res.ok) return new NextResponse(null, { status: res.status >= 500 ? 502 : res.status })
    const data = (await res.json()) as { images: Hit[]; federation: boolean }
    return NextResponse.json({ ...data, images: sign(data.images ?? []) }, { headers: { 'Cache-Control': 'no-store' } })
  }

  const u = searchParams.get('u') ?? ''
  const s = searchParams.get('s') ?? ''
  // Signed by a page we rendered, or nothing: this must not become a way to make the server
  // fetch arbitrary URLs.
  if (!u || !s || !verifyThumb(u, s)) return new NextResponse(null, { status: 403 })
  const origin = req.nextUrl.origin
  const thumb = await fetch(`${origin}/api/thumb?u=${encodeURIComponent(u)}&s=${encodeURIComponent(s)}`, {
    cache: 'no-store',
  })
  const type = thumb.headers.get('content-type') ?? ''
  if (!thumb.ok || !type.startsWith('image/')) return new NextResponse(null, { status: 502 })
  const bytes = await thumb.arrayBuffer()
  // The proxy answers a 1×1 pixel for anything it would not serve; that is not a query image.
  if (bytes.byteLength < 200) return new NextResponse(null, { status: 502 })
  return forward(bytes, type)
}
