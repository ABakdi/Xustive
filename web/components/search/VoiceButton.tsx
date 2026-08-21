'use client'

import { Loader2, Mic } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { SearchFailed, transcribe } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

/**
 * Voice search ([[UI - Voice Search]], M3-T03).
 *
 * A microphone button that records a short clip, transcribes it on our own server, and drops the
 * result **into the search box, editable and not auto-submitted** (M3-T03.6). The last step is the
 * whole ethic of it: Darija transcription is imperfect, so the person confirms the words before
 * searching rather than being sent somewhere on the model's guess.
 *
 * # Progressive, permissionless-until-asked
 *
 * The button renders only where `getUserMedia` and `MediaRecorder` exist (M3-T03.1), and asks for
 * the microphone **on tap, never on load** (M3-T03.2). On completion or cancel every track is
 * stopped so the browser's recording indicator clears (M3-T03.7) — a mic left live after a search
 * is both a privacy problem and the kind of thing that makes people distrust a page.
 *
 * # Bounds
 *
 * A 30-second hard cap (announced), so a forgotten recording cannot run forever. Transcription is
 * cancellable by closing the overlay.
 */
const MAX_MS = 30_000

type Phase = 'idle' | 'recording' | 'transcribing'

export function VoiceButton({
  t,
  uiLang,
  onTranscript,
}: {
  t: Messages
  uiLang: string
  onTranscript: (text: string) => void
}) {
  const [supported, setSupported] = useState(false)
  const [phase, setPhase] = useState<Phase>('idle')
  const [error, setError] = useState<string>('')
  const [elapsed, setElapsed] = useState(0)

  const recorder = useRef<MediaRecorder | null>(null)
  const stream = useRef<MediaStream | null>(null)
  const chunks = useRef<Blob[]>([])
  const abort = useRef<AbortController | null>(null)
  const timer = useRef<ReturnType<typeof setInterval> | null>(null)
  const cap = useRef<ReturnType<typeof setTimeout> | null>(null)
  const dialogRef = useRef<HTMLDialogElement>(null)
  // Set on cancel so the recorder's `stop` handler knows not to transcribe.
  const cancelled = useRef(false)

  // Capability detection runs in an effect, so the server render (which cannot know) and the first
  // client render agree — the button appears only after mount, and only where it can work.
  useEffect(() => {
    setSupported(
      typeof navigator !== 'undefined' &&
        !!navigator.mediaDevices?.getUserMedia &&
        typeof window !== 'undefined' &&
        'MediaRecorder' in window,
    )
  }, [])

  const cleanup = useCallback(() => {
    if (timer.current) clearInterval(timer.current)
    if (cap.current) clearTimeout(cap.current)
    timer.current = null
    cap.current = null
    // Stopping the tracks is what clears the browser's mic indicator.
    stream.current?.getTracks().forEach((tr) => tr.stop())
    stream.current = null
    recorder.current = null
  }, [])

  useEffect(() => {
    return () => {
      abort.current?.abort()
      cleanup()
    }
  }, [cleanup])

  async function send(blob: Blob) {
    setPhase('transcribing')
    abort.current?.abort()
    const controller = new AbortController()
    abort.current = controller
    try {
      const out = await transcribe(blob, uiLang, controller.signal)
      if (controller.signal.aborted) return
      // The text goes to the box; the person edits and submits. We never submit for them.
      if (out.text.trim()) onTranscript(out.text.trim())
      close()
    } catch (err) {
      if ((err as Error)?.name === 'AbortError') return
      setError(err instanceof SearchFailed && err.status === 503 ? t.voiceUnavailable : t.voiceFailed)
      setPhase('idle')
    }
  }

  async function start() {
    setError('')
    cancelled.current = false
    try {
      const s = await navigator.mediaDevices.getUserMedia({ audio: true })
      stream.current = s
      chunks.current = []
      const rec = new MediaRecorder(s, pickMime())
      recorder.current = rec
      rec.ondataavailable = (e) => {
        if (e.data.size > 0) chunks.current.push(e.data)
      }
      rec.onstop = () => {
        const blob = new Blob(chunks.current, { type: rec.mimeType || 'audio/webm' })
        cleanup()
        if (cancelled.current || blob.size === 0) {
          if (cancelled.current) close()
          return
        }
        void send(blob)
      }
      rec.start()
      setPhase('recording')
      dialogRef.current?.showModal()

      setElapsed(0)
      const startedAt = Date.now()
      timer.current = setInterval(() => setElapsed(Date.now() - startedAt), 200)
      // Hard cap: stop cleanly at 30 s rather than recording forever.
      cap.current = setTimeout(() => stop(), MAX_MS)
    } catch {
      // Permission denied, no device, or insecure context. A denied permission is not an error to
      // shout about — show the guidance and stay idle.
      setError(t.voicePermission)
      setPhase('idle')
      cleanup()
    }
  }

  function stop() {
    if (recorder.current?.state === 'recording') recorder.current.stop()
  }

  function cancel() {
    cancelled.current = true
    abort.current?.abort()
    if (recorder.current?.state === 'recording') {
      recorder.current.stop() // onstop sees `cancelled` and closes
    } else {
      cleanup()
      close()
    }
  }

  function close() {
    setPhase('idle')
    setElapsed(0)
    if (dialogRef.current?.open) dialogRef.current.close()
  }

  if (!supported) return null

  const seconds = Math.floor(elapsed / 1000)

  return (
    <>
      <button
        type="button"
        aria-label={t.voiceSearch}
        title={t.voiceSearch}
        className="shrink-0 p-1"
        style={{ color: 'var(--fg-faint)', borderRadius: 'var(--radius)' }}
        onClick={() => void start()}
      >
        <Mic size={18} aria-hidden />
      </button>

      {error && (
        <span role="status" className="sr-only">
          {error}
        </span>
      )}

      <dialog
        ref={dialogRef}
        className="rounded border p-0"
        style={{ borderColor: 'var(--line-strong)', background: 'var(--bg)', color: 'var(--fg)' }}
        onCancel={(e) => {
          // Esc closes the dialog — treat it as cancel so the mic is released.
          e.preventDefault()
          cancel()
        }}
        aria-label={t.voiceSearch}
      >
        <div className="flex min-w-[280px] flex-col items-center gap-4 p-6 text-center">
          <p className="m-0 flex items-center gap-2 text-lg font-medium" aria-live="polite">
            {phase === 'transcribing' ? (
              <>
                <Loader2 size={18} aria-hidden className="animate-spin" />
                {t.voiceTranscribing}
              </>
            ) : (
              <>
                <span
                  aria-hidden
                  className="motion-safe:animate-pulse"
                  style={{ inlineSize: 12, blockSize: 12, borderRadius: '50%', background: '#c0392b', display: 'inline-block' }}
                />
                {t.voiceListening}
                <span className="tabular-nums" style={{ color: 'var(--fg-faint)' }}>
                  {seconds}s
                </span>
              </>
            )}
          </p>

          <p className="m-0 text-xs" style={{ color: 'var(--fg-faint)' }}>
            {t.voiceHint}
          </p>

          <div className="flex items-center gap-2">
            {phase === 'recording' && (
              <button
                type="button"
                className="chip chip-active cursor-pointer"
                onClick={() => stop()}
              >
                {t.voiceStop}
              </button>
            )}
            <button type="button" className="chip cursor-pointer" onClick={() => cancel()}>
              {t.voiceCancel}
            </button>
          </div>
        </div>
      </dialog>
    </>
  )
}

/** Prefer Opus/WebM, the widely-supported efficient codec; fall back to the browser default. */
function pickMime(): MediaRecorderOptions {
  const prefs = ['audio/webm;codecs=opus', 'audio/webm', 'audio/ogg;codecs=opus', 'audio/mp4']
  for (const mimeType of prefs) {
    if (typeof MediaRecorder !== 'undefined' && MediaRecorder.isTypeSupported(mimeType)) {
      return { mimeType, audioBitsPerSecond: 24_000 }
    }
  }
  return {}
}
