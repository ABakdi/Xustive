'use client'

import { Mic, Square } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { SearchFailed, transcribe } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'

/**
 * Voice search ([[UI - Voice Search]], M3-T03) — inline, and live.
 *
 * Tap the microphone and the field itself becomes the recorder: the button turns red and pulses,
 * a small level meter shows the microphone is hearing you, and the words appear **in the search
 * box as you speak**. The transcription still happens on our own server and nothing else's
 * (M3-T03): a few times a second the audio so far is sent again — to the sidecar's fast model,
 * greedy — and the box shows the newest reading, so the text grows while you talk; on stop, one
 * last pass with the careful model gives the final words. The
 * result stays editable and is never submitted for you (M3-T03.6) — Darija transcription is
 * imperfect — and then, on stop, the search runs with them: the words were on screen the whole
 * time, and editing is one tap away on the results page. (Auto-submit on stop was asked for
 * after the live version shipped; the earlier rule was never to submit for the person.)
 *
 * The first version put all this behind a modal dialog with a timer in it and pasted the text at
 * the end. It looked like a phone call, not a search box, and when the server was off it failed
 * in a message only screen readers could hear. Errors are visible now, under the field.
 *
 * # Permissionless until asked, and released after
 *
 * The button renders only where `getUserMedia` and `MediaRecorder` exist (M3-T03.1) and asks for
 * the microphone on tap, never on load (M3-T03.2). On stop or cancel every track is stopped so
 * the browser's recording indicator clears (M3-T03.7). A 30-second cap ends a forgotten recording.
 */
const MAX_MS = 30_000
/** How often the audio so far is re-read while recording — as often as the fast model keeps up. */
const PARTIAL_EVERY_MS = 400

type Phase = 'idle' | 'recording' | 'finishing'

export type VoiceState = { phase: Phase; seconds: number; level: number; error: string }

export function VoiceButton({
  t,
  uiLang,
  size = 18,
  onInterim,
  onTranscript,
  onState,
}: {
  t: Messages
  uiLang: string
  size?: number
  /** A reading of the words so far, while still recording. Replaces the previous reading. */
  onInterim: (text: string) => void
  /** The final words; the box searches with them. */
  onTranscript: (text: string) => void
  /** What the field should show around the button: phase, elapsed seconds, mic level, error. */
  onState: (state: VoiceState) => void
}) {
  const [supported, setSupported] = useState(false)
  const [phase, setPhase] = useState<Phase>('idle')

  const recorder = useRef<MediaRecorder | null>(null)
  const stream = useRef<MediaStream | null>(null)
  const chunks = useRef<Blob[]>([])
  const inFlight = useRef<AbortController | null>(null)
  const ticker = useRef<ReturnType<typeof setInterval> | null>(null)
  const cap = useRef<ReturnType<typeof setTimeout> | null>(null)
  const audio = useRef<{ ctx: AudioContext; analyser: AnalyserNode; raf: number } | null>(null)
  const startedAt = useRef(0)
  const lastPartial = useRef(0)
  const cancelled = useRef(false)
  // The newest live reading. If the final pass fails, these are the words — they were on screen
  // already, and "unavailable" under a box full of text is a lie.
  const lastInterim = useRef('')
  const state = useRef<VoiceState>({ phase: 'idle', seconds: 0, level: 0, error: '' })

  const emit = useCallback(
    (patch: Partial<VoiceState>) => {
      state.current = { ...state.current, ...patch }
      onState(state.current)
    },
    [onState],
  )

  useEffect(() => {
    setSupported(
      typeof navigator !== 'undefined' &&
        !!navigator.mediaDevices?.getUserMedia &&
        typeof window !== 'undefined' &&
        'MediaRecorder' in window,
    )
  }, [])

  const release = useCallback(() => {
    if (ticker.current) clearInterval(ticker.current)
    if (cap.current) clearTimeout(cap.current)
    ticker.current = null
    cap.current = null
    if (audio.current) {
      cancelAnimationFrame(audio.current.raf)
      void audio.current.ctx.close().catch(() => undefined)
      audio.current = null
    }
    // Stopping the tracks is what clears the browser's mic indicator.
    stream.current?.getTracks().forEach((tr) => tr.stop())
    stream.current = null
    recorder.current = null
  }, [])

  useEffect(() => {
    return () => {
      inFlight.current?.abort()
      release()
    }
  }, [release])

  /** The audio so far, as one blob. WebM chunks from one recorder concatenate into a valid file. */
  function soFar(): Blob {
    return new Blob(chunks.current, { type: recorder.current?.mimeType || 'audio/webm' })
  }

  async function read(blob: Blob, final: boolean) {
    inFlight.current?.abort()
    const controller = new AbortController()
    inFlight.current = controller
    try {
      const out = await transcribe(blob, uiLang, controller.signal, !final)
      if (controller.signal.aborted) return
      const text = out.text.trim()
      if (final) {
        const words = text || lastInterim.current
        if (words) onTranscript(words)
        finish('')
      } else if (text) {
        lastInterim.current = text
        onInterim(text)
      }
    } catch (err) {
      if ((err as Error)?.name === 'AbortError') return
      const message =
        err instanceof SearchFailed && (err.status === 503 || err.status === 404)
          ? t.voiceUnavailable
          : t.voiceFailed
      // A failed partial is not worth interrupting for — the next one may land. A failed final
      // hands over the last live reading if there is one, and is the answer only when there is
      // nothing else to give.
      if (final) {
        if (lastInterim.current) {
          onTranscript(lastInterim.current)
          finish('')
        } else finish(message)
      }
      else if (err instanceof SearchFailed && (err.status === 503 || err.status === 404)) {
        // The server has no transcriber at all: stop early and say so, rather than record thirty
        // seconds into nothing.
        cancelled.current = false
        stopRecording(message)
      }
    } finally {
      if (inFlight.current === controller) inFlight.current = null
    }
  }

  function meter(s: MediaStream) {
    try {
      const ctx = new AudioContext()
      const analyser = ctx.createAnalyser()
      analyser.fftSize = 512
      ctx.createMediaStreamSource(s).connect(analyser)
      const buf = new Uint8Array(analyser.frequencyBinCount)
      const tick = () => {
        analyser.getByteTimeDomainData(buf)
        let sum = 0
        for (const v of buf) {
          const d = (v - 128) / 128
          sum += d * d
        }
        const rms = Math.sqrt(sum / buf.length)
        emit({ level: Math.min(1, rms * 4) })
        if (audio.current) audio.current.raf = requestAnimationFrame(tick)
      }
      audio.current = { ctx, analyser, raf: requestAnimationFrame(tick) }
    } catch {
      // No meter is fine; the words still arrive.
    }
  }

  async function start() {
    cancelled.current = false
    lastInterim.current = ''
    emit({ error: '', seconds: 0, level: 0 })
    try {
      const s = await navigator.mediaDevices.getUserMedia({ audio: true })
      stream.current = s
      chunks.current = []
      const rec = new MediaRecorder(s, pickMime())
      recorder.current = rec
      rec.ondataavailable = (e) => {
        if (e.data.size > 0) chunks.current.push(e.data)
        // A partial reading, when the last one is old enough and nothing is still being read.
        const now = Date.now()
        if (
          rec.state === 'recording' &&
          !inFlight.current &&
          now - lastPartial.current >= PARTIAL_EVERY_MS &&
          chunks.current.length > 0
        ) {
          lastPartial.current = now
          void read(soFar(), false)
        }
      }
      rec.onstop = () => {
        const blob = soFar()
        release()
        if (cancelled.current || blob.size === 0) {
          // A cancel keeps whatever message stopped it; an empty recording is its own message.
          finish(cancelled.current ? undefined : t.voiceFailed)
          return
        }
        setPhase('finishing')
        emit({ phase: 'finishing', level: 0 })
        void read(blob, true)
      }
      // Timesliced, so the audio arrives as it is spoken rather than all at the end.
      rec.start(200)
      startedAt.current = Date.now()
      lastPartial.current = Date.now()
      setPhase('recording')
      emit({ phase: 'recording' })
      meter(s)
      ticker.current = setInterval(
        () => emit({ seconds: Math.floor((Date.now() - startedAt.current) / 1000) }),
        250,
      )
      cap.current = setTimeout(() => stopRecording(), MAX_MS)
    } catch {
      // Permission denied, no device, or an insecure context. Guidance, not an alarm.
      release()
      finish(t.voicePermission)
    }
  }

  function stopRecording(errorAfter = '') {
    if (errorAfter) {
      // Stop without a final read: the server already said it cannot.
      cancelled.current = true
      inFlight.current?.abort()
      if (recorder.current?.state === 'recording') recorder.current.stop()
      else release()
      finish(errorAfter)
      return
    }
    if (recorder.current?.state === 'recording') recorder.current.stop()
  }

  function cancel() {
    cancelled.current = true
    inFlight.current?.abort()
    if (recorder.current?.state === 'recording') {
      recorder.current.stop() // onstop sees `cancelled`
    } else {
      release()
      finish('')
    }
  }

  /** Back to idle. `undefined` leaves the current message alone; a string replaces it. */
  function finish(error: string | undefined) {
    setPhase('idle')
    emit({ phase: 'idle', seconds: 0, level: 0, ...(error === undefined ? {} : { error }) })
  }

  useEffect(() => {
    if (phase !== 'recording') return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        cancel()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase])

  if (!supported) return null

  const recording = phase === 'recording'
  return (
    <button
      type="button"
      aria-label={recording ? t.voiceStop : t.voiceSearch}
      title={recording ? t.voiceStop : t.voiceSearch}
      aria-pressed={recording}
      className={`voice-button relative shrink-0 p-1 ${recording ? 'is-recording' : ''}`}
      style={{ color: recording ? '#fff' : 'var(--fg-faint)', borderRadius: '50%' }}
      disabled={phase === 'finishing'}
      onClick={() => (recording ? stopRecording() : void start())}
    >
      {recording ? <Square size={size - 4} aria-hidden fill="currentColor" /> : <Mic size={size} aria-hidden />}
    </button>
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
