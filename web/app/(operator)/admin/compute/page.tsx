'use client'

import { useCallback, useEffect, useState } from 'react'

import { getCompute, setDevice, setLogLevel, setPoliteness } from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

interface Resolved {
  /** The device actually in use — the API serialises this as `active` ("cpu" | "gpu"). */
  active?: string
  reason?: string
  gpu_layers?: number
  /** True when GPU was asked for but not used (no CUDA build, or not enough VRAM). */
  fell_back?: boolean
  gpu?: { name?: string; total_mib?: number; free_mib?: number } | null
}

export default function ComputePage() {
  const [data, setData] = useState<Record<string, unknown> | null>(null)
  const [msg, setMsg] = useState('')
  const [pref, setPref] = useState('auto')
  const [layers, setLayers] = useState('')
  const [logFilter, setLogFilter] = useState('')

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

  type ModelRow = {
    spec: { id: string; role: string; size_mib: number; licence: string; commercial_use: boolean }
    present: boolean
    actual_mib: number
  }
  const models = (data?.models ?? []) as ModelRow[]
  const logging = (data?.logging ?? {}) as {
    filter?: string
    baseline?: string
    override_expires_in?: number | null
  }
  const ranking = (data?.ranking ?? {}) as Record<string, number>
  // The additive signals, in score order — the same ones the re-ranker sums (spam is subtracted).
  const rankSignals = ['relevance', 'freshness', 'trust', 'authority', 'quality', 'interaction']

  return (
    <>
      <PageHead title="Compute">
        Which device the models run on, and the crawler&rsquo;s politeness switch. Device changes
        take effect on the next model load, not mid-request.
      </PageHead>

      <p className="mb-6 text-[0.95rem]">
        Currently running on <strong>{resolved.active ?? 'unknown'}</strong>
        {resolved.reason ? ` — ${resolved.reason}` : ''}. GPU support{' '}
        {gpuCompiled ? 'compiled in' : 'not compiled in'}; hardware{' '}
        {gpuDetected ? 'detected' : 'not detected'}.
      </p>

      {/* The common misconfiguration on this hardware: a GPU is present but this binary cannot use
          it because it was built without CUDA. Spell out the fix rather than leaving it in the
          resolution reason, where it reads as an error rather than an instruction. */}
      {resolved.fell_back && resolved.gpu ? (
        <p
          className="mb-6 max-w-2xl rounded border px-3 py-2 text-sm"
          style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
        >
          A <strong>{resolved.gpu.name}</strong> is present
          {resolved.gpu.free_mib != null ? ` (${resolved.gpu.free_mib} MB free)` : ''}, but this
          build runs on the CPU because it was compiled without CUDA. Restart the API with{' '}
          <code>make run-api</code> — it auto-detects the CUDA toolkit and builds with GPU support.
          (Launching with a plain <code>cargo run</code> or the fast/no-summariser build skips CUDA,
          which is what happened here.)
        </p>
      ) : null}

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

      {/* Models present on disk, with the licence that governs each. Surfaces the finding that the
          default Qwen2.5-3B is non-commercial — an operator planning a launch needs to see it. */}
      <h2 className="mb-2 text-lg font-semibold">Models</h2>
      {models.length > 0 ? (
        <table className="mb-8 w-full max-w-2xl border-collapse text-sm">
          <thead>
            <tr style={{ color: 'var(--fg-muted)' }}>
              <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>id</th>
              <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>role</th>
              <th className="border-b py-1 text-end" style={{ borderColor: 'var(--line)' }}>size</th>
              <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>licence</th>
              <th className="border-b py-1 text-start" style={{ borderColor: 'var(--line)' }}>present</th>
            </tr>
          </thead>
          <tbody>
            {models.map((m) => (
              <tr key={m.spec.id}>
                <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{m.spec.id}</td>
                <td className="border-b py-1" style={{ borderColor: 'var(--line)' }}>{m.spec.role}</td>
                <td className="border-b py-1 text-end tabular-nums" style={{ borderColor: 'var(--line)' }}>{m.spec.size_mib} MB</td>
                <td className="border-b py-1 text-xs" style={{ borderColor: 'var(--line)' }}>
                  {m.spec.licence}
                  {!m.spec.commercial_use ? (
                    <strong style={{ color: 'var(--warn)' }}> — non-commercial</strong>
                  ) : null}
                </td>
                <td className="border-b py-1" style={{ borderColor: 'var(--line)' }}>
                  {m.present ? 'yes' : <span style={{ color: 'var(--fg-faint)' }}>no</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p className="mb-8 text-sm" style={{ color: 'var(--fg-faint)' }}>No models registered.</p>
      )}

      {/* Live ranking weights — how results are scored right now. Read-only: tuning is a config
          file (config/ranking.toml) applied at restart, not a runtime knob, so this is the mirror. */}
      <h2 className="mb-2 text-lg font-semibold">Ranking weights</h2>
      <p className="mb-2 text-sm" style={{ color: 'var(--fg-muted)' }}>
        How results are scored. Relevance dominates by construction; the rest are bounded
        tie-breakers. Change these in <code>config/ranking.toml</code> (applied on restart).
      </p>
      <ul className="mb-8 flex max-w-xl flex-wrap gap-x-6 gap-y-1 list-none p-0 text-sm">
        {rankSignals.map((k) => (
          <li key={k} className="flex gap-2">
            <span style={{ color: 'var(--fg-muted)' }}>{k}</span>
            <span className="tabular-nums font-medium">{ranking[k]?.toFixed(2) ?? '—'}</span>
          </li>
        ))}
        <li className="flex gap-2">
          <span style={{ color: 'var(--fg-muted)' }}>spam (−)</span>
          <span className="tabular-nums font-medium">{ranking['spam_penalty']?.toFixed(2) ?? '—'}</span>
        </li>
      </ul>

      {/* Runtime log verbosity. A temporary raise auto-reverts, so turning on debug to chase an
          issue cannot be left on by accident. */}
      <h2 className="mb-2 text-lg font-semibold">Logging</h2>
      <p className="mb-3 text-sm" style={{ color: 'var(--fg-muted)' }}>
        Current filter <code>{logging.filter ?? '…'}</code>
        {logging.override_expires_in != null
          ? ` — temporary, reverts in ${logging.override_expires_in}s`
          : ' — baseline'}
        .
      </p>
      <form
        className="mb-8 flex flex-wrap items-end gap-2"
        suppressHydrationWarning
        onSubmit={async (e) => {
          e.preventDefault()
          setMsg('applying…')
          try {
            const r = await setLogLevel(logFilter.trim() || null)
            setMsg(`log filter now ${r.filter}${r.expires_in != null ? ` (reverts in ${r.expires_in}s)` : ''}`)
            setLogFilter('')
            load()
          } catch (err) {
            setMsg((err as Error).message)
          }
        }}
      >
        <label className="flex flex-col gap-1 text-sm">
          Temporary filter <span style={{ color: 'var(--fg-faint)' }}>(e.g. debug, or info,xustive=debug)</span>
          <input
            value={logFilter}
            onChange={(e) => setLogFilter(e.target.value)}
            placeholder="debug"
            autoComplete="off"
            className="min-h-10 min-w-[240px] rounded border px-3 py-1.5"
            style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
            suppressHydrationWarning
          />
        </label>
        <button type="submit" disabled={!logFilter.trim()} className="min-h-10 self-end rounded border px-4 text-sm" style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}>
          Raise
        </button>
        <button
          type="button"
          onClick={async () => {
            try {
              await setLogLevel(null)
              setMsg('log filter reverted to baseline')
              load()
            } catch (err) {
              setMsg((err as Error).message)
            }
          }}
          className="min-h-10 self-end rounded border px-4 text-sm"
          style={{ borderColor: 'var(--line)', color: 'var(--fg)' }}
        >
          Revert
        </button>
      </form>

      <p className="text-sm" style={{ color: 'var(--fg-muted)' }}>
        {msg}
      </p>
    </>
  )
}
