'use client'

import { Rows2, Rows3 } from 'lucide-react'
import { useRouter } from 'next/navigation'
import { useState, useTransition } from 'react'

import { setDensity } from '@/lib/prefs'
import type { Density } from '@/lib/theme'

const ORDER: Density[] = ['comfortable', 'compact']
const ICON = { comfortable: Rows2, compact: Rows3 }

/**
 * Toggles comfortable ↔ compact.
 *
 * The tokens and the cookie already existed; only the control was missing, so the preference was
 * settable but not by a user. Built to mirror [`ThemeToggle`] exactly — same shape, same ghost
 * button, same optimistic write — because two adjacent controls that behave differently is worse
 * than either behaviour on its own.
 *
 * The attribute is set on the document immediately and the cookie written in the background, so
 * the change is instant and the *next* server render agrees with it. Waiting for the round trip
 * makes a layout toggle feel broken.
 *
 * # Why this matters more here than on most sites
 *
 * Compact is not a cosmetic preference for this audience. Arabic sets taller than Latin at the same
 * point size, so a result list that fits on one screen in French runs onto two in Arabic — and a
 * meaningful share of Algerian traffic is on small phones where that is the difference between
 * scanning results and scrolling through them.
 */
export function DensityToggle({
  current,
  labels,
}: {
  current: Density
  labels: Record<Density, string>
}) {
  const [density, setLocal] = useState<Density>(current)
  const [, startTransition] = useTransition()
  const router = useRouter()

  const Icon = ICON[density]

  return (
    <button
      type="button"
      className="ghost"
      // Names the current state, not the action — a screen-reader user pressing this repeatedly
      // needs to know where they landed, which is the same reasoning as ThemeToggle.
      aria-label={labels[density]}
      title={labels[density]}
      onClick={() => {
        const next = ORDER[(ORDER.indexOf(density) + 1) % ORDER.length] as Density
        setLocal(next)
        document.documentElement.dataset.density = next
        startTransition(async () => {
          await setDensity(next)
          router.refresh()
        })
      }}
    >
      <Icon size={16} aria-hidden />
    </button>
  )
}
