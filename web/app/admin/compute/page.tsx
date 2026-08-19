'use client'

import { useCallback, useEffect, useState } from 'react'

import { getCompute, setDevice, setPoliteness } from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

interface Resolved {
  device?: string
  reason?: string
  gpu_layers?: number
}

export default function ComputePage() {
  const [data, setData] = useState<Record<string, unknown> | null>(null)
  const [msg, setMsg] = useState('')
  const [pref, setPref] = useState('auto')
  const [layers, setLayers] = useState('')

  const load = useCallback(() => {
    getCompute()
      .then(setData)
      .catch((e) => setMsg((e as Error).message))
  }, [])
  useEffect(() => load(), [load])

  const resolved = (data?.device ?? {}) as Resolved
  const gpuCompiled = data?.gpu_support_compiled as boolean | undefined
  const gpuDetected = data?.gpu_detected as unknown
  const ignorePoliteness = Boolean(data?.ignore_politeness)

  return (
    <>
      <PageHead title="Compute">
        Which device the models run on, and the crawler&rsquo;s politeness switch. Device changes
        take effect on the next model load, not mid-request.
      </PageHead>

      <p className="mb-6 text-[0.95rem]">
        Currently running on <strong>{resolved.device ?? 'unknown'}</strong>
        {resolved.reason ? ` — ${resolved.reason}` : ''}. GPU support{' '}
        {gpuCompiled ? 'compiled in' : 'not compiled in'}; hardware{' '}
        {gpuDetected ? 'detected' : 'not detected'}.
      </p>

      <form
        className="mb-8 flex max-w-md flex-col gap-4"
        onSubmit={async (e) => {
          e.preventDefault()
          setMsg('applying…')
          try {
            const r = (await setDevice(pref, layers === '' ? null : Number(layers))) as { note?: string }
            setMsg(r.note ?? 'applied')
            load()
          } catch (err) {
            setMsg((err as Error).message)
          }
        }}
      >
        <label className="flex flex-col gap-1 text-sm">
          Device preference
          <select
            value={pref}
            onChange={(e) => setPref(e.target.value)}
            className="min-h-11 rounded border px-3 py-2"
            style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
          >
            <option value="auto">auto — decide from available memory</option>
            <option value="gpu">gpu</option>
            <option value="cpu">cpu</option>
          </select>
        </label>
        <label className="flex flex-col gap-1 text-sm">
          GPU layers <span style={{ color: 'var(--fg-faint)' }}>(blank = auto)</span>
          <input
            type="number"
            min={0}
            max={999}
            value={layers}
            onChange={(e) => setLayers(e.target.value)}
            className="min-h-11 rounded border px-3 py-2"
            style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
          />
        </label>
        <button type="submit" className="min-h-11 self-start rounded border px-4" style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}>
          Apply
        </button>
      </form>

      <h2 className="mb-2 text-lg font-semibold">Politeness bypass</h2>
      <p className="mb-3 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <strong>Testing only.</strong> With this on, the crawler ignores robots.txt, per-host delays,
        and adaptive slowdown. Production refuses to start with it enabled.
      </p>
      <label className="mb-8 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={ignorePoliteness}
          onChange={async (e) => {
            try {
              await setPoliteness(e.target.checked)
              load()
            } catch (err) {
              setMsg((err as Error).message)
            }
          }}
        />
        Ignore politeness (bypass is <strong>{ignorePoliteness ? 'ON' : 'off'}</strong>)
      </label>

      <p className="text-sm" style={{ color: 'var(--fg-muted)' }}>
        {msg}
      </p>
    </>
  )
}
