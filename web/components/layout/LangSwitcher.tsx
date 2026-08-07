'use client'

import { Languages } from 'lucide-react'
import { usePathname, useSearchParams } from 'next/navigation'
import Link from 'next/link'
import { useState } from 'react'

import { LOCALES, type Locale } from '@/lib/i18n/config'

const NAMES: Record<Locale, string> = {
  ar: 'العربية',
  ary: 'الدارجة',
  fr: 'Français',
  en: 'English',
}

/**
 * Switch interface language while staying on the same page.
 *
 * Each option is a real `<a>` to the same route under a different locale, so it works without
 * JavaScript and each language is a URL you can link to directly. Only the *disclosure* needs
 * script.
 *
 * The language is named in its own language — someone who cannot read the current interface still
 * has to be able to find theirs.
 */
export function LangSwitcher({ current, label }: { current: Locale; label: string }) {
  const [open, setOpen] = useState(false)
  const pathname = usePathname()
  const params = useSearchParams()

  const rest = pathname.split('/').slice(2).join('/')
  const query = params.toString()
  const href = (locale: Locale) => `/${locale}${rest ? `/${rest}` : ''}${query ? `?${query}` : ''}`

  return (
    <div className="relative">
      <button
        type="button"
        className="ghost"
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((v) => !v)}
        onBlur={() => setTimeout(() => setOpen(false), 140)}
      >
        <Languages size={16} aria-hidden />
        <span className="hidden sm:inline">{NAMES[current]}</span>
      </button>

      {open && (
        <ul
          role="menu"
          className="rise absolute mt-1 min-w-36 border py-1"
          style={{
            insetInlineEnd: 0,
            zIndex: 'var(--z-dropdown)' as unknown as number,
            // Opaque. A floating panel has to occlude what it covers, or it is text over text.
            background: 'var(--bg-sunk)',
            borderColor: 'var(--line-strong)',
            borderRadius: 'var(--radius)',
          }}
        >
          {LOCALES.map((locale) => (
            <li key={locale} role="none">
              <Link
                role="menuitem"
                href={href(locale)}
                lang={locale}
                dir={locale === 'ar' || locale === 'ary' ? 'rtl' : 'ltr'}
                className="flex px-3 py-2 text-sm"
                style={{
                  minBlockSize: '36px',
                  alignItems: 'center',
                  color: locale === current ? 'var(--accent)' : 'var(--fg)',
                }}
                {...(locale === current ? { 'aria-current': 'true' as const } : {})}
              >
                {NAMES[locale]}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
