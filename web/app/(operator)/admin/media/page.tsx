'use client'

import { getMedia, type MediaStatus } from '@/lib/admin'
import { PageHead, usePoll } from '@/components/admin/ui'

/** A green/amber status dot with a label — up, down, or unknown. */
function Dot({ ok, label }: { ok: boolean | undefined; label: string }) {
  const color = ok === undefined ? 'var(--fg-faint)' : ok ? 'var(--ok, #2e7d32)' : 'var(--warn, #b26a00)'
  return (
    <span className="inline-flex items-center gap-2">
      <span
        aria-hidden
        style={{ inlineSize: 9, blockSize: 9, borderRadius: '50%', background: color, display: 'inline-block' }}
      />
      <span>{label}</span>
    </span>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-b py-2" style={{ borderColor: 'var(--line)' }}>
      <span className="text-sm" style={{ color: 'var(--fg-muted)' }}>
        {label}
      </span>
      <span className="text-sm font-medium">{children}</span>
    </div>
  )
}

export default function MediaPage() {
  const { data, error } = usePoll<MediaStatus>(getMedia, 10_000)

  const ocr = data?.ocr
  const vector = data?.vector
  const stt = data?.stt

  return (
    <>
      <PageHead title="Image AI">
        The OCR engine and image-similarity stack. Read-only status: which engine is selected and
        whether the optional sidecars are up. Selection is set in <code>[media]</code> and{' '}
        <code>[vector]</code> config and takes effect on restart.
      </PageHead>

      {error ? (
        <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>
          Could not reach the API: {error}
        </p>
      ) : null}

      <section className="mb-8 max-w-xl">
        <h2 className="mb-2 text-base font-semibold">OCR</h2>
        <Row label="Engine">{ocr?.backend ?? '…'}</Row>
        <Row label="Status">
          {ocr ? (
            <Dot
              ok={ocr.healthy}
              label={
                ocr.backend === 'tesseract'
                  ? 'in-process (always ready)'
                  : ocr.healthy
                    ? 'sidecar up'
                    : 'sidecar down — falling back to tesseract'
              }
            />
          ) : (
            '…'
          )}
        </Row>
        {ocr && ocr.backend !== 'tesseract' ? (
          <Row label="Sidecar">
            <code className="text-xs">{ocr.sidecar_endpoint}</code>
          </Row>
        ) : null}
      </section>

      <section className="max-w-xl">
        <h2 className="mb-2 text-base font-semibold">Image similarity</h2>
        {!vector ? (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
            …
          </p>
        ) : vector.enabled === false ? (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
            Disabled. Set <code>[vector] enabled = true</code> and provide a CLIP embedder to turn on
            &ldquo;search by image&rdquo; similarity.
          </p>
        ) : (
          <>
            <Row label="Qdrant">
              <Dot ok={vector.qdrant_reachable} label={vector.qdrant_reachable ? 'reachable' : 'unreachable'} />
            </Row>
            <Row label="CLIP embedder">
              <Dot ok={vector.embedder_healthy} label={vector.embedder_healthy ? 'up' : 'down'} />
            </Row>
            <Row label="Collection">
              <code className="text-xs">{vector.collection}</code>
            </Row>
            <Row label="Image vectors">
              {vector.image_vectors == null ? 'unknown' : vector.image_vectors.toLocaleString()}
            </Row>
          </>
        )}
      </section>

      <section className="mt-8 max-w-xl">
        <h2 className="mb-2 text-base font-semibold">Voice</h2>
        {!stt ? (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
            …
          </p>
        ) : stt.enabled === false ? (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
            Disabled. Set <code>[stt] enabled = true</code> and provide a whisper model to turn on
            voice search.
          </p>
        ) : (
          <Row label="STT sidecar">
            <Dot ok={stt.healthy} label={stt.healthy ? 'up' : 'down'} />
          </Row>
        )}
      </section>
    </>
  )
}
