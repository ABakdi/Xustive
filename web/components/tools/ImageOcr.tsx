'use client'

import { Camera, Copy, Images, Loader2, Search, Upload } from 'lucide-react'
import Link from 'next/link'
import { useRouter } from 'next/navigation'
import { useCallback, useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/Button'
import {
  imageSearch,
  ocrImage,
  SearchFailed,
  type OcrResult,
  type SimilarResult,
} from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

/**
 * Image → text, and the front of the "photograph → search" (Lens-style) flow.
 *
 * The whole design turns on one rule: the recognised text is **never** searched automatically. It
 * lands in an editable box and waits for a deliberate tap on "Search this" ([[UI - Image Search]]
 * M3-T06.4). OCR is imperfect, especially on Arabic signage, and auto-submitting its guess would
 * send people to results for a word the image never contained.
 *
 * # Downscale and EXIF strip happen here, before upload
 *
 * A phone photo is 4–12 MP and carries EXIF — including GPS. `createImageBitmap(..., {
 * imageOrientation: 'from-image' })` applies the orientation tag and then the canvas re-encode drops
 * *all* metadata, so what leaves the browser is upright, ≤ 2048 px, and location-free (M3-T06.2). The
 * server never receives the coordinates, which is the point: they cannot leak what they never get.
 *
 * # Privacy
 *
 * The image is posted as a raw body, read in memory on the server, and never stored. Nothing about
 * it is kept client-side either — no history, no cache.
 */
const MAX_DIM = 2048

export function ImageOcr({ lang, t }: { lang: string; t: Messages }) {
  const router = useRouter()
  const fileInput = useRef<HTMLInputElement>(null)
  const cameraInput = useRef<HTMLInputElement>(null)
  const abort = useRef<AbortController | null>(null)

  const [preview, setPreview] = useState<string | null>(null)
  const [state, setState] = useState<'idle' | 'reading' | 'done' | 'failed'>('idle')
  const [result, setResult] = useState<OcrResult | null>(null)
  const [text, setText] = useState('')
  const [copied, setCopied] = useState(false)
  const [dragging, setDragging] = useState(false)
  // The downscaled, EXIF-stripped blob kept for the visual-similarity search, so "Find similar"
  // reuses exactly what was OCR'd rather than re-preparing the original.
  const preparedRef = useRef<Blob | null>(null)
  const [similar, setSimilar] = useState<SimilarResult[] | null>(null)
  const [simState, setSimState] = useState<
    'idle' | 'searching' | 'done' | 'unavailable' | 'failed'
  >('idle')

  // Revoke the last preview object URL when it is replaced or the component unmounts — an image
  // blob URL held forever is a real leak on a page someone keeps open.
  useEffect(() => {
    return () => {
      if (preview) URL.revokeObjectURL(preview)
      abort.current?.abort()
    }
  }, [preview])

  const run = useCallback(
    async (file: Blob) => {
      abort.current?.abort()
      const controller = new AbortController()
      abort.current = controller

      setState('reading')
      setResult(null)
      setText('')
      setCopied(false)
      setSimilar(null)
      setSimState('idle')

      try {
        const prepared = await downscale(file)
        preparedRef.current = prepared
        const url = URL.createObjectURL(prepared)
        setPreview((old) => {
          if (old) URL.revokeObjectURL(old)
          return url
        })

        const out = await ocrImage(prepared, controller.signal)
        if (controller.signal.aborted) return
        setResult(out)
        setText(out.text)
        setState('done')
      } catch (error) {
        if ((error as Error)?.name === 'AbortError') return
        setState('failed')
      }
    },
    [],
  )

  const onFiles = useCallback(
    (files: FileList | null) => {
      const file = files?.[0]
      if (file && file.type.startsWith('image/')) void run(file)
    },
    [run],
  )

  // Paste an image from the clipboard anywhere on the page — the fastest path for a screenshot.
  useEffect(() => {
    function onPaste(e: ClipboardEvent) {
      const item = Array.from(e.clipboardData?.items ?? []).find((i) =>
        i.type.startsWith('image/'),
      )
      const file = item?.getAsFile()
      if (file) void run(file)
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [run])

  function copy() {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }

  function searchThis() {
    const q = text.trim()
    if (q) router.push(`/${lang}/search?q=${encodeURIComponent(q)}`)
  }

  async function findSimilar() {
    const image = preparedRef.current
    if (!image) return
    setSimState('searching')
    setSimilar(null)
    try {
      const out = await imageSearch(image)
      setSimilar(out.results)
      setSimState('done')
    } catch (error) {
      // A 503 is the feature being off or the vector services being down — a distinct, honest state,
      // not a failure the user caused.
      if (error instanceof SearchFailed && error.status === 503) {
        setSimState('unavailable')
      } else {
        setSimState('failed')
      }
    }
  }

  const lowConfidence = result != null && (!result.usable || result.confidence < 60)

  return (
    <section aria-label={t.ocrTitle}>
      {/* The drop zone doubles as the input surface and the preview. Keyboard users reach the two
          buttons inside it; the zone itself is a drop target, an enhancement over them. */}
      <div
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault()
          setDragging(false)
          onFiles(e.dataTransfer.files)
        }}
        className="flex flex-col items-center justify-center rounded border border-dashed px-6 py-10 text-center"
        style={{
          borderColor: dragging ? 'var(--accent)' : 'var(--line-strong)',
          background: dragging ? 'var(--accent-wash)' : 'var(--bg-sunk)',
          minBlockSize: '200px',
        }}
      >
        {preview ? (
          // eslint-disable-next-line @next/next/no-img-element -- a local blob URL, never remote
          <img
            src={preview}
            alt=""
            className="max-h-64 max-w-full rounded object-contain"
            style={{ border: '1px solid var(--line)' }}
          />
        ) : (
          <p className="mb-4 text-base" style={{ color: 'var(--fg-faint)' }}>
            {t.ocrDrop}
          </p>
        )}

        <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
          <Button type="button" onClick={() => fileInput.current?.click()}>
            <Upload size={16} aria-hidden className="me-1 inline" />
            {t.ocrChoose}
          </Button>
          <Button type="button" onClick={() => cameraInput.current?.click()}>
            <Camera size={16} aria-hidden className="me-1 inline" />
            {t.ocrCamera}
          </Button>
        </div>

        {/* Two inputs: the plain picker, and one with `capture` so a phone opens the camera. Both
            are visually hidden and driven by the buttons above. */}
        <input
          ref={fileInput}
          type="file"
          accept="image/*"
          className="sr-only"
          onChange={(e) => onFiles(e.target.files)}
        />
        <input
          ref={cameraInput}
          type="file"
          accept="image/*"
          capture="environment"
          className="sr-only"
          onChange={(e) => onFiles(e.target.files)}
        />
      </div>

      <p className="mt-2 mb-0 text-xs" style={{ color: 'var(--fg-faint)' }}>
        {t.ocrPrivacy}
      </p>

      {state === 'reading' && (
        <p
          className="mt-4 flex items-center gap-2 text-base"
          style={{ color: 'var(--fg-faint)' }}
          aria-live="polite"
        >
          <Loader2 size={16} aria-hidden className="animate-spin" />
          {t.ocrReading}
        </p>
      )}

      {state === 'failed' && (
        <p className="mt-4 text-base" style={{ color: 'var(--negative, #c0392b)' }} aria-live="polite">
          {t.ocrFailed}
        </p>
      )}

      {state === 'done' && (
        <div className="mt-5">
          {result && result.text ? (
            <>
              <label className="mb-1 block text-sm font-medium" htmlFor="ocr-text">
                {t.ocrResult}
              </label>
              <textarea
                id="ocr-text"
                value={text}
                onChange={(e) => setText(e.target.value)}
                rows={5}
                dir="auto"
                className="w-full resize-y rounded border px-3 py-2 text-base"
                style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
              />

              {lowConfidence && (
                <p className="mt-1 mb-0 text-xs" style={{ color: 'var(--fg-faint)' }}>
                  {t.ocrLowConfidence}
                </p>
              )}
              {result.backend === 'unlimited' && (
                <p className="mt-1 mb-0 text-xs" style={{ color: 'var(--fg-faint)' }}>
                  {t.ocrEnhanced}
                </p>
              )}

              <div className="mt-3 flex flex-wrap items-center gap-2">
                <Button type="button" onClick={searchThis} disabled={!text.trim()}>
                  <Search size={16} aria-hidden className="me-1 inline" />
                  {t.ocrSearchThis}
                </Button>
                <Button type="button" onClick={copy} disabled={!text.trim()}>
                  <Copy size={16} aria-hidden className="me-1 inline" />
                  {copied ? t.copied : t.copy}
                </Button>
              </div>
            </>
          ) : (
            <p className="text-base" style={{ color: 'var(--fg-faint)' }} aria-live="polite">
              {t.ocrEmpty}
            </p>
          )}

          {/* Visual similarity — the other half of Lens. Available whether or not OCR read any
              text, because "find pages with this image" does not depend on the image having text. */}
          <div className="mt-3">
            <Button type="button" variant="quiet" onClick={() => void findSimilar()}>
              <Images size={16} aria-hidden className="me-1 inline" />
              {t.ocrFindSimilar}
            </Button>
          </div>

          {simState === 'searching' && (
            <p
              className="mt-3 flex items-center gap-2 text-sm"
              style={{ color: 'var(--fg-faint)' }}
              aria-live="polite"
            >
              <Loader2 size={16} aria-hidden className="animate-spin" />
              {t.ocrFindSimilar}
            </p>
          )}
          {simState === 'unavailable' && (
            <p className="mt-3 text-sm" style={{ color: 'var(--fg-faint)' }} aria-live="polite">
              {t.ocrSimilarUnavailable}
            </p>
          )}
          {simState === 'failed' && (
            <p className="mt-3 text-sm" style={{ color: 'var(--fg-faint)' }} aria-live="polite">
              {t.ocrFailed}
            </p>
          )}
          {simState === 'done' && similar && (
            <div className="mt-4">
              {similar.length === 0 ? (
                <p className="text-sm" style={{ color: 'var(--fg-faint)' }} aria-live="polite">
                  {t.ocrNoSimilar}
                </p>
              ) : (
                <>
                  <h2 className="mb-2 text-sm font-medium">{t.ocrSimilarResults}</h2>
                  <ul className="m-0 list-none p-0">
                    {similar.map((r) => (
                      <li key={r.id} className="mb-3">
                        <Link
                          href={r.url}
                          className="text-base font-medium no-underline hover:underline"
                          style={{ color: 'var(--accent, #1a5fb4)' }}
                          dir="auto"
                        >
                          {r.title || r.display_url}
                        </Link>
                        <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--fg-faint)' }}>
                          <span dir="ltr">{r.display_url}</span>
                          <span aria-hidden>·</span>
                          <span>{simLabel(r.score, t)}</span>
                        </div>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  )
}

/** A qualitative similarity label — never a raw score ([[UI - Image Search]] M3-T06.5). */
function simLabel(score: number, t: Messages): string {
  if (score >= 0.92) return t.similarityVery
  if (score >= 0.82) return t.similaritySimilar
  return t.similarityRelated
}

/**
 * Downscale to ≤ MAX_DIM on the long edge and drop all metadata.
 *
 * `imageOrientation: 'from-image'` bakes the EXIF rotation into the pixels so the server sees an
 * upright image; the canvas export then carries no EXIF at all, GPS included. Images already within
 * bounds are still re-encoded — that is what strips the metadata — but never enlarged, since the
 * server upscales small crops itself for OCR.
 */
async function downscale(file: Blob): Promise<Blob> {
  const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' })
  const longEdge = Math.max(bitmap.width, bitmap.height)
  const scale = Math.min(1, MAX_DIM / longEdge)
  const width = Math.round(bitmap.width * scale)
  const height = Math.round(bitmap.height * scale)

  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    bitmap.close()
    return file
  }
  ctx.drawImage(bitmap, 0, 0, width, height)
  bitmap.close()

  return new Promise<Blob>((resolve) => {
    canvas.toBlob(
      // Fall back to the original bytes if the browser cannot encode, rather than failing the flow.
      (blob) => resolve(blob ?? file),
      'image/jpeg',
      0.92,
    )
  })
}
