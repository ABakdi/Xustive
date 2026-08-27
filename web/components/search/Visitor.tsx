'use client'

import { useEffect } from 'react'

/**
 * The first-party visitor and session cookies ([[ADR-0030]], M11-T01.3).
 *
 * `xv` is a random id kept a year; `xs` lives for the browser session. Set here, by our own
 * page, on first paint — and read only by our own servers, which write them into the search
 * events. Nothing about them reaches a third party (ADR-0029 rule 2). Mounted only when the
 * operator has collection on, so a deployment that keeps nothing sets nothing.
 */
export function Visitor() {
  useEffect(() => {
    try {
      const has = (name: string) => document.cookie.split(';').some((c) => c.trim().startsWith(`${name}=`))
      if (!has('xv')) document.cookie = `xv=${ulid()}; Max-Age=31536000; Path=/; SameSite=Lax`
      if (!has('xs')) document.cookie = `xs=${ulid()}; Path=/; SameSite=Lax`
    } catch {
      // No cookies, no record — the search still works.
    }
  }, [])
  return null
}

/** A ULID-shaped token: 26 Crockford-base32 characters, time first, random after. */
function ulid(): string {
  const A = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'
  let t = Date.now()
  let out = ''
  for (let i = 0; i < 10; i++) {
    out = A[t % 32] + out
    t = Math.floor(t / 32)
  }
  const r = new Uint8Array(16)
  crypto.getRandomValues(r)
  for (let i = 0; i < 16; i++) out += A.charAt((r[i] ?? 0) % 32)
  return out
}
