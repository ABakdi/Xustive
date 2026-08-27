'use client'

import { Camera, ImagePlus, Search } from 'lucide-react'
import { useRouter } from 'next/navigation'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import {
  imageSearch,
  imageSearchByUrl,
  imageSearchWeb,
  SearchFailed,
  type ImageHit,
  type ImageSearchAnswer,
} from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'
import { prepareImage, takeHandOff } from '@/lib/image-prep'

/**
 * Reverse image search — the page ([[Milestone 10 - Reverse Image Search]] T04).
 *
 * A picture in, pictures out. The query image stays in the browser (an object URL, shown small
 * at the top, never stored); the prepared bytes go once to the web tier, which asks the API and
 * signs the thumbnails. Three groups render as grids — the same picture, similar pictures, and,
 * a moment later, the web by description — and two chip rows narrow all three **without a second
 * request**: the whole bounded set is here, so a chip is a filter over what is on screen and its
 * count is exact. The chips are whatever the results are, not a menu: a kind or a format that no
 * result has is not offered.
 *
 * The web group is fetched after the local groups paint, from the words the picture was
 * described with (ADR-0028). When federation is off it is absent, not empty.
 */
type Phase = 'idle' | 'reading' | 'searching' | 'done' | 'unavailable' | 'failed'

const t_ = (t: Messages, key: string, fallback: string) =>
  (t as unknown as Record<string, string | undefined>)[key] ?? fallback

const styleName = (t: Messages, id: string) => t_(t, `style_${id}`, id.replace(/_/g, ' '))
const subjectName = (t: Messages, id: string) => t_(t, `subject_${id}`, id.replace(/_/g, ' '))

export function ReverseImage({
  lang,
  t,
  byUrl,
}: {
  lang: string
  t: Messages
  /** A picture already on the Images tab, as its signed thumbnail URL (`?u=&s=`). */
  byUrl?: { u: string; s: string }
}) {
  const router = useRouter()
  const fileInput = useRef<HTMLInputElement>(null)
  const cameraInput = useRef<HTMLInputElement>(null)
  const abort = useRef<AbortController | null>(null)

  const [phase, setPhase] = useState<Phase>('idle')
  const [preview, setPreview] = useState<string | null>(null)
  const [answer, setAnswer] = useState<ImageSearchAnswer | null>(null)
  const [web, setWeb] = useState<{ images: ImageHit[]; federation: boolean } | 'loading' | null>(null)
  const [kind, setKind] = useState<string | null>(null)
  const [format, setFormat] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)

  // Two lifetimes, two effects. The preview's object URL dies when it is replaced; the in-flight
  // request dies only when the page does. Tying both to `preview` aborted every search the
  // moment its preview appeared, and an AbortError is the one error the search ignores — so the
  // page sat on "Searching…" forever.
  useEffect(() => {
    return () => {
      if (preview?.startsWith('blob:')) URL.revokeObjectURL(preview)
    }
  }, [preview])
  useEffect(() => () => abort.current?.abort(), [])

  const settle = useCallback(
    async (work: Promise<ImageSearchAnswer>, controller: AbortController) => {
      setPhase('searching')
      setAnswer(null)
      setWeb(null)
      setKind(null)
      setFormat(null)
      try {
        const out = await work
        if (controller.signal.aborted) return
        setAnswer(out)
        setPhase('done')
        // The web group, after the local ones are on screen — by the words, never the picture.
        if (out.query.web_query) {
          setWeb('loading')
          imageSearchWeb(out.query.web_query, controller.signal)
            .then((w) => {
              if (!controller.signal.aborted) setWeb(w)
            })
            .catch(() => {
              if (!controller.signal.aborted) setWeb({ images: [], federation: false })
            })
        }
      } catch (error) {
        if ((error as Error)?.name === 'AbortError') return
        setPhase(error instanceof SearchFailed && error.status === 503 ? 'unavailable' : 'failed')
      }
    },
    [],
  )

  const run = useCallback(
    async (file: Blob) => {
      abort.current?.abort()
      const controller = new AbortController()
      abort.current = controller
      setPhase('reading')
      try {
        const prepared = await prepareImage(file)
        const url = URL.createObjectURL(prepared)
        setPreview((old) => {
          if (old) URL.revokeObjectURL(old)
          return url
        })
        await settle(imageSearch(prepared, controller.signal), controller)
      } catch (error) {
        if ((error as Error)?.name === 'AbortError') return
        setPhase('failed')
      }
    },
    [settle],
  )

  // Two ways in without a file: a picture the previous page handed off, or one already on the
  // Images tab named by its signed URL.
  useEffect(() => {
    if (byUrl) {
      const controller = new AbortController()
      abort.current = controller
      setPreview(`/api/thumb?u=${encodeURIComponent(byUrl.u)}&s=${encodeURIComponent(byUrl.s)}`)
      void settle(imageSearchByUrl(byUrl.u, byUrl.s, controller.signal), controller)
      return
    }
    void takeHandOff().then((blob) => {
      if (blob) void run(blob)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const onFiles = useCallback(
    (files: FileList | null) => {
      const file = files?.[0]
      if (file && file.type.startsWith('image/')) void run(file)
    },
    [run],
  )

  useEffect(() => {
    function onPaste(e: ClipboardEvent) {
      const item = Array.from(e.clipboardData?.items ?? []).find((i) => i.type.startsWith('image/'))
      const file = item?.getAsFile()
      if (file) void run(file)
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [run])

  // The set on screen: local groups plus the web group once it arrives.
  const all: ImageHit[] = useMemo(() => {
    const local = answer?.images ?? []
    const remote = web && web !== 'loading' ? web.images : []
    return [...local, ...remote]
  }, [answer, web])

  const kinds = useMemo(() => countBy(all, (i) => i.style), [all])
  const formats = useMemo(() => countBy(all, (i) => i.ext), [all])

  const shown = all.filter((i) => (!kind || i.style === kind) && (!format || i.ext === format))
  const groups: { id: ImageHit['group']; title: string; empty: string }[] = [
    { id: 'same', title: t_(t, 'reverseSame', 'The same picture'), empty: t_(t, 'reverseNoneSame', '') },
    { id: 'similar', title: t_(t, 'reverseSimilar', 'Similar pictures'), empty: t_(t, 'reverseNoneSimilar', '') },
    { id: 'web', title: t_(t, 'reverseWeb', 'From the web, by description'), empty: t_(t, 'reverseNoneWeb', '') },
  ]

  return (
    <section aria-label={t_(t, 'reverseTitle', 'Search with a picture')}>
      {/* The way in: a drop zone with two buttons; paste works anywhere on the page. */}
      <div
        className="rounded-lg border-2 border-dashed p-5 text-center"
        style={{ borderColor: dragging ? 'var(--accent)' : 'var(--line-strong)', background: 'var(--bg-sunk)' }}
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
      >
        <div className="flex flex-wrap items-center justify-center gap-4">
          {preview && (
            // The query image: an object URL (or our own proxy), shown small, never uploaded twice.
            // eslint-disable-next-line @next/next/no-img-element
            <img src={preview} alt="" className="max-h-28 rounded" style={{ maxWidth: 160, objectFit: 'contain' }} />
          )}
          <div className="flex flex-col items-center gap-2">
            <p className="m-0 text-sm" style={{ color: 'var(--fg-muted)' }}>
              {t_(t, 'reverseDrop', 'Drop a picture here, or paste one')}
            </p>
            <div className="flex gap-2">
              <button type="button" className="chip chip-active cursor-pointer" onClick={() => fileInput.current?.click()}>
                <ImagePlus size={14} aria-hidden /> {preview ? t_(t, 'reverseAnother', 'Another picture') : t_(t, 'reverseChoose', 'Choose a picture')}
              </button>
              <button type="button" className="chip cursor-pointer" onClick={() => cameraInput.current?.click()}>
                <Camera size={14} aria-hidden /> {t_(t, 'reverseCamera', 'Take a photo')}
              </button>
            </div>
          </div>
        </div>
        {/* Password managers inject attributes onto inputs before React hydrates; not ours to fix. */}
        <input ref={fileInput} type="file" accept="image/*" className="sr-only" suppressHydrationWarning onChange={(e) => onFiles(e.target.files)} />
        <input
          ref={cameraInput}
          type="file"
          accept="image/*"
          capture="environment"
          className="sr-only"
          suppressHydrationWarning
          onChange={(e) => onFiles(e.target.files)}
        />
      </div>
      <p className="m-0 mt-2 text-xs" style={{ color: 'var(--fg-faint)' }}>
        {t_(t, 'reversePrivacy', '')}
      </p>

      {(phase === 'reading' || phase === 'searching') && (
        <p className="mt-6 text-sm" role="status" aria-live="polite" style={{ color: 'var(--fg-muted)' }}>
          {phase === 'reading' ? t_(t, 'reverseUploading', 'Reading the picture…') : t_(t, 'reverseSearching', 'Searching…')}
        </p>
      )}
      {phase === 'unavailable' && (
        <p className="mt-6 text-sm" role="status" style={{ color: 'var(--fg-muted)' }}>
          {t_(t, 'reverseUnavailable', '')}{' '}
          <a href={`/${lang}/tools/ocr`} className="underline">
            {t_(t, 'reverseReadText', 'Read the picture’s text')}
          </a>
        </p>
      )}
      {phase === 'failed' && (
        <p className="mt-6 text-sm" role="status" style={{ color: 'var(--danger, #c0392b)' }}>
          {t_(t, 'reverseFailed', 'Could not read that picture.')}
        </p>
      )}

      {phase === 'done' && answer && (
        <div className="mt-6">
          {/* What the picture is, in words: its kind, its format, its subjects. */}
          {(answer.query.style || answer.query.labels.length > 0) && (
            <p className="m-0 mb-3 text-sm" dir="auto">
              <span style={{ color: 'var(--fg-muted)' }}>{t_(t, 'reverseLooksLike', 'Looks like')}: </span>
              {[
                ...(answer.query.style ? [styleName(t, answer.query.style)] : []),
                ...answer.query.labels.map((l) => subjectName(t, l)),
              ].join(' · ')}
              {answer.query.ext && <span style={{ color: 'var(--fg-faint)' }}> · {answer.query.ext}</span>}
            </p>
          )}

          <Chips label={t_(t, 'reverseKind', 'Kind')} all={t_(t, 'reverseAll', 'All')} items={kinds} value={kind} onChange={setKind} name={(id) => styleName(t, id)} />
          <Chips label={t_(t, 'reverseType', 'Format')} all={t_(t, 'reverseAll', 'All')} items={formats} value={format} onChange={setFormat} name={(id) => id} />

          {groups.map((g) => {
            if (g.id === 'web' && (!answer.query.web_query || (web && web !== 'loading' && !web.federation))) return null
            const tiles = shown.filter((i) => i.group === g.id)
            const loading = g.id === 'web' && web === 'loading'
            return (
              <section key={g.id} className="mt-6" aria-label={g.title}>
                <h2 className="m-0 mb-3 text-base font-semibold" dir="auto">
                  {g.title}
                  {g.id === 'web' && answer.query.web_query && (
                    <span className="ms-2 text-xs font-normal" style={{ color: 'var(--fg-faint)' }}>
                      “{answer.query.labels.map((l) => subjectName(t, l)).join(' ')}”
                    </span>
                  )}
                </h2>
                {loading ? (
                  <p className="m-0 text-sm" style={{ color: 'var(--fg-muted)' }}>{t_(t, 'reverseSearching', 'Searching…')}</p>
                ) : tiles.length === 0 ? (
                  <p className="m-0 text-sm" style={{ color: 'var(--fg-muted)' }}>{g.empty}</p>
                ) : (
                  <ul className="m-0 grid list-none gap-3 p-0" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))' }}>
                    {tiles.map((i) => (
                      <li key={`${i.group}:${i.url}`} className="min-w-0">
                        <a href={i.page.url} className="block no-underline" rel="noopener noreferrer nofollow" dir="auto">
                          {/* eslint-disable-next-line @next/next/no-img-element */}
                          <img src={i.thumb} alt="" loading="lazy" referrerPolicy="no-referrer" className="aspect-[4/3] w-full rounded object-cover" style={{ background: 'var(--bg-sunk)' }} />
                          <span className="mt-1 block truncate text-xs" style={{ color: 'var(--fg)' }}>
                            <bdi>{i.page.title || hostOf(i.page.url)}</bdi>
                          </span>
                          <span className="block truncate text-[11px]" style={{ color: 'var(--fg-faint)' }}>
                            {hostOf(i.page.url)}
                            {i.ext && ` · ${i.ext}`}
                            {i.style && ` · ${styleName(t, i.style)}`}
                            {i.page.from_web && ` · ${t_(t, 'fromTheWeb', 'from the web')}`}
                          </span>
                        </a>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            )
          })}
        </div>
      )}
      {/* A second search box would be a different page; the wordmark above links home, and this
          takes text the reader may prefer. */}
      {phase === 'done' && answer && answer.query.labels.length > 0 && (
        <p className="mt-8 text-sm">
          <button
            type="button"
            className="chip cursor-pointer"
            onClick={() => router.push(`/${lang}/search?q=${encodeURIComponent(answer.query.labels.map((l) => subjectName(t, l)).join(' '))}&v=images`)}
          >
            <Search size={14} aria-hidden /> {answer.query.labels.map((l) => subjectName(t, l)).join(' ')}
          </button>
        </p>
      )}
    </section>
  )
}

function Chips({
  label,
  all,
  items,
  value,
  onChange,
  name,
}: {
  label: string
  all: string
  items: [string, number][]
  value: string | null
  onChange: (v: string | null) => void
  name: (id: string) => string
}) {
  if (items.length === 0) return null
  const total = items.reduce((n, [, c]) => n + c, 0)
  const chip = (id: string | null, text: string, count: number) => {
    const on = value === id
    return (
      <button
        key={id ?? '*'}
        type="button"
        aria-pressed={on}
        onClick={() => onChange(on ? null : id)}
        className="rounded-[var(--radius-pill)] border px-2.5 py-1 text-xs transition-colors"
        style={{
          borderColor: on ? 'var(--accent)' : 'var(--line)',
          background: on ? 'var(--accent-wash)' : 'transparent',
          color: on ? 'var(--accent)' : 'var(--fg)',
        }}
      >
        <bdi>{text}</bdi>
        <span className="ms-1 numeric" style={{ color: 'var(--fg-muted)' }}>
          {count}
        </span>
      </button>
    )
  }
  return (
    <p className="m-0 mb-2 flex flex-wrap items-center gap-2 text-sm" dir="auto">
      <span style={{ color: 'var(--fg-muted)' }}>{label}</span>
      {chip(null, all, total)}
      {items.map(([id, c]) => chip(id, name(id), c))}
    </p>
  )
}

function countBy(items: ImageHit[], key: (i: ImageHit) => string | undefined): [string, number][] {
  const m = new Map<string, number>()
  for (const i of items) {
    const k = key(i)
    if (k) m.set(k, (m.get(k) ?? 0) + 1)
  }
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
}

function hostOf(url: string) {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return ''
  }
}
