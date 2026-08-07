'use client'

import { Search, X } from 'lucide-react'
import { useRouter } from 'next/navigation'
import { useEffect, useId, useRef, useState } from 'react'

import { suggest, type Suggestion } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

const DEBOUNCE_MS = 90
const MIN_PREFIX = 2

/**
 * The search box, with suggestions.
 *
 * A real `<form method="get">` wrapping a real `<input>`, so it submits and navigates without any
 * JavaScript at all. Everything below is enhancement layered on top of something that already
 * works — not a replacement for it.
 *
 * The suggestion list is an ARIA combobox because that is what it is: a list that appears under an
 * input and answers arrow keys. A screen reader not told about it reads the input as empty while
 * sighted users see eight options.
 *
 * Nothing about a prefix is stored client-side. There is no local history, because the most
 * private thing a search engine holds is what you started to type and then deleted.
 */
export function SearchBox({
  lang,
  t,
  initialQuery = '',
  compact = false,
}: {
  lang: string
  t: Messages
  initialQuery?: string
  compact?: boolean
}) {
  const router = useRouter()
  const listId = useId()
  const inputRef = useRef<HTMLInputElement>(null)

  const [value, setValue] = useState(initialQuery)
  const [items, setItems] = useState<Suggestion[]>([])
  const [active, setActive] = useState(-1)
  const [open, setOpen] = useState(false)
  // What was actually typed, so arrowing through options and back returns it.
  const typedRef = useRef(initialQuery)

  // The URL is the source of truth for the query. A back-navigation must move the input.
  useEffect(() => {
    setValue(initialQuery)
    typedRef.current = initialQuery
  }, [initialQuery])

  useEffect(() => {
    const prefix = value.trim()
    if (prefix.length < MIN_PREFIX || !open) {
      setItems([])
      return
    }
    const controller = new AbortController()
    // Debounced so a fast typist sends one request per pause rather than one per key, and short
    // enough that the list still feels attached to the keyboard.
    const timer = setTimeout(() => {
      suggest(prefix, controller.signal)
        .then(setItems)
        // Including aborts. A suggestion box that shows an error is worse than one showing
        // nothing: the user is mid-keystroke and did not ask a question.
        .catch(() => setItems([]))
    }, DEBOUNCE_MS)

    return () => {
      clearTimeout(timer)
      controller.abort()
    }
  }, [value, open])

  function close() {
    setOpen(false)
    setActive(-1)
  }

  function highlight(index: number) {
    setActive(index)
    setValue(index >= 0 ? (items[index]?.text ?? typedRef.current) : typedRef.current)
  }

  function submit(query: string) {
    const q = query.trim()
    if (!q) return
    close()
    router.push(`/${lang}/search?q=${encodeURIComponent(q)}`)
  }

  return (
    <form
      role="search"
      action={`/${lang}/search`}
      method="get"
      className="relative w-full"
      onSubmit={(e) => {
        // Only intercept when JavaScript is running; without it the form submits normally.
        e.preventDefault()
        submit(active >= 0 ? (items[active]?.text ?? value) : value)
      }}
    >
      {/* A rectangle. A pill-shaped search field is the most recognisable "generic web app"
          signal there is, and this is a search engine. */}
      <div className="field" style={{ minBlockSize: compact ? '38px' : '48px' }}>
        <Search
          size={compact ? 16 : 18}
          aria-hidden
          className="shrink-0"
          style={{ color: 'var(--fg-faint)' }}
        />
        <input
          ref={inputRef}
          type="search"
          name="q"
          dir="auto"
          value={value}
          autoComplete="off"
          spellCheck={false}
          enterKeyHint="search"
          maxLength={512}
          aria-label={t.searchLabel}
          role="combobox"
          aria-expanded={open && items.length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={active >= 0 ? `${listId}-${active}` : undefined}
          placeholder={t.searchPlaceholder}
          style={{ paddingBlock: compact ? '0.5rem' : '0.75rem' }}
          onChange={(e) => {
            typedRef.current = e.target.value
            setValue(e.target.value)
            setOpen(true)
            setActive(-1)
          }}
          onFocus={() => setOpen(true)}
          // Delayed so a click on a suggestion registers before the list closes.
          onBlur={() => setTimeout(close, 120)}
          onKeyDown={(e) => {
            if (!open || items.length === 0) return
            // Arrow semantics do not flip in RTL: down is still further down the list. Only the
            // horizontal axis mirrors, and this list has none.
            if (e.key === 'ArrowDown') {
              e.preventDefault()
              highlight(active + 1 >= items.length ? -1 : active + 1)
            } else if (e.key === 'ArrowUp') {
              e.preventDefault()
              highlight(active - 1 < -1 ? items.length - 1 : active - 1)
            } else if (e.key === 'Escape') {
              e.preventDefault()
              setValue(typedRef.current)
              close()
            } else if (e.key === 'Tab') {
              close()
            }
          }}
        />
        {value && (
          <button
            type="button"
            aria-label="Clear"
            className="shrink-0 p-1"
            style={{ color: 'var(--fg-faint)', borderRadius: 'var(--radius)' }}
            onClick={() => {
              typedRef.current = ''
              setValue('')
              setItems([])
              inputRef.current?.focus()
            }}
          >
            <X size={18} aria-hidden />
          </button>
        )}
      </div>

      {open && items.length > 0 && (
        <ul
          id={listId}
          role="listbox"
          className="rise absolute mt-1 overflow-y-auto border py-1"
          style={{
            insetInlineStart: 0,
            insetInlineEnd: 0,
            zIndex: 'var(--z-dropdown)' as unknown as number,
            // An opaque surface, not the page background. A floating panel has to occlude what it
            // covers or it is just text drawn over other text.
            background: 'var(--bg-sunk)',
            borderColor: 'var(--line-strong)',
            borderRadius: 'var(--radius)',
            maxBlockSize: '60vh',
          }}
        >
          {items.map((item, i) => (
            <li
              key={item.text}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              dir="auto"
              className="flex cursor-pointer items-center px-4 text-start text-base"
              style={{
                minBlockSize: '40px',
                background: i === active ? 'var(--accent-wash)' : 'transparent',
              }}
              // mousedown, not click: blur fires first on click and would close the list before
              // the selection registered.
              onMouseDown={(e) => {
                e.preventDefault()
                submit(item.text)
              }}
              onMouseEnter={() => setActive(i)}
            >
              {item.text}
            </li>
          ))}
        </ul>
      )}
    </form>
  )
}
