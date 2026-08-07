'use client'

import { Monitor, Moon, Sun } from 'lucide-react'
import { useRouter } from 'next/navigation'
import { useState, useTransition } from 'react'

import { setTheme } from '@/lib/prefs'
import type { Theme } from '@/lib/theme'

const ORDER: Theme[] = ['system', 'light', 'dark']
const ICON = { system: Monitor, light: Sun, dark: Moon }

/**
 * Cycles system → light → dark.
 *
 * The DOM attribute is set immediately so the change is instant, and the cookie is written in the
 * background so the *next* request server-renders the same thing. Waiting for the round trip
 * would make a colour toggle feel broken.
 */
export function ThemeToggle({ current, labels }: { current: Theme; labels: Record<Theme, string> }) {
  const [theme, setLocal] = useState<Theme>(current)
  const [, startTransition] = useTransition()
  const router = useRouter()

  const Icon = ICON[theme]

  return (
    <button
      type="button"
      className="ghost"
      // The label names the *current* state, not the action. A screen-reader user pressing this
      // repeatedly needs to know where they landed.
      aria-label={labels[theme]}
      title={labels[theme]}
      onClick={() => {
        const next = ORDER[(ORDER.indexOf(theme) + 1) % ORDER.length] as Theme
        setLocal(next)
        document.documentElement.dataset.theme =
          next === 'system'
            ? matchMedia('(prefers-color-scheme: dark)').matches
              ? 'dark'
              : 'light'
            : next
        startTransition(async () => {
          await setTheme(next)
          router.refresh()
        })
      }}
    >
      <Icon size={16} aria-hidden />
    </button>
  )
}
