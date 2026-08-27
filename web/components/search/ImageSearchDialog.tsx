'use client'

import { X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { ReverseImage } from '@/components/search/ReverseImage'
import type { Messages } from '@/lib/i18n/messages'

/** The event the camera control raises; the dialog listens for it wherever it is mounted. */
export const OPEN_IMAGE_SEARCH = 'xustive:image-search'

/**
 * Search with a picture, in place (M10, revised): the camera icon opens this dialog over the
 * page instead of leaving for a tool page. Inside it the reverse-image island runs with the
 * text read too — similar pictures, and the picture's text with results for it, selectable
 * and searchable. Closing it (the button, Esc, a click outside) drops everything: the picture
 * was never anywhere but here.
 *
 * Without JavaScript the camera control is still a link to `/tools/ocr`, so the flow exists.
 */
export function ImageSearchDialog({ lang, t }: { lang: string; t: Messages }) {
  const ref = useRef<HTMLDialogElement>(null)
  // Mounted only while open, so a new session starts clean and nothing lingers when closed.
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const onOpen = () => setOpen(true)
    window.addEventListener(OPEN_IMAGE_SEARCH, onOpen)
    return () => window.removeEventListener(OPEN_IMAGE_SEARCH, onOpen)
  }, [])

  // The element exists only after `open` has committed; a timer from the event handler ran
  // before that and showed nothing.
  useEffect(() => {
    if (open && ref.current && !ref.current.open) ref.current.showModal()
  }, [open])

  const close = () => {
    ref.current?.close()
    setOpen(false)
  }

  if (!open) return null
  const tt = t as unknown as Record<string, string>
  return (
    <dialog
      ref={ref}
      className="w-[min(96vw,64rem)] rounded-lg border p-0"
      // Centred explicitly: the global reset zeroes margins, which takes a modal to the corner.
      style={{ borderColor: 'var(--line-strong)', background: 'var(--bg)', color: 'var(--fg)', maxHeight: '90vh', margin: 'auto' }}
      onCancel={(e) => {
        e.preventDefault()
        close()
      }}
      onClick={(e) => {
        // A click on the backdrop — outside the panel — closes.
        if (e.target === ref.current) close()
      }}
      aria-label={tt.reverseTitle}
    >
      <div className="max-h-[90vh] overflow-y-auto p-5">
        <div className="mb-3 flex items-start justify-between gap-3">
          <div>
            <h2 className="m-0 text-lg font-semibold">{tt.reverseTitle}</h2>
            <p className="m-0 mt-0.5 text-xs" style={{ color: 'var(--fg-muted)' }}>{tt.reverseIntro}</p>
          </div>
          <button type="button" className="shrink-0 p-1" aria-label={tt.reverseClose} onClick={close} style={{ color: 'var(--fg-faint)', borderRadius: 'var(--radius)' }}>
            <X size={18} aria-hidden />
          </button>
        </div>
        <ReverseImage lang={lang} t={t} withText />
      </div>
    </dialog>
  )
}
