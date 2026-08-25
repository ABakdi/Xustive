'use client'

import { useCallback, useEffect, useState } from 'react'

import { getConfig } from '@/lib/admin'
import { PageHead } from '@/components/admin/ui'

/**
 * The effective configuration, read-only (PROB-003).
 *
 * One page that answers "what is this system actually running with" — defaults, config file, and
 * environment overrides already merged by the backend. Secrets arrive redacted (the marker
 * distinguishes "set" from "empty"). Deliberately not editable: config changes stay file edits
 * that pass Config::validate() on start, so the safety guards (k-floors, politeness, salts)
 * cannot be side-stepped from a browser.
 */
export default function ConfigPage() {
  const [cfg, setCfg] = useState<Record<string, unknown> | null>(null)
  const [msg, setMsg] = useState('')
  const [filter, setFilter] = useState('')

  const load = useCallback(() => {
    getConfig()
      .then((d) => setCfg(d.config))
      .catch((e) => setMsg((e as Error).message))
  }, [])
  useEffect(() => load(), [load])

  // The config is a two-level tree: top-level scalars (environment) and sections of scalars/lists.
  const sections: [string, Record<string, unknown>][] = []
  const scalars: [string, unknown][] = []
  if (cfg) {
    for (const [k, v] of Object.entries(cfg)) {
      if (v !== null && typeof v === 'object' && !Array.isArray(v)) {
        sections.push([k, v as Record<string, unknown>])
      } else {
        scalars.push([k, v])
      }
    }
  }
  const q = filter.trim().toLowerCase()
  const matches = (key: string, value: unknown) =>
    !q || key.toLowerCase().includes(q) || String(value).toLowerCase().includes(q)

  const renderValue = (v: unknown) => {
    if (Array.isArray(v)) return v.length ? v.join(', ') : '(empty list)'
    if (typeof v === 'boolean') return v ? 'true' : 'false'
    if (v === '' || v == null) return <span style={{ color: 'var(--fg-faint)' }}>(empty)</span>
    return String(v)
  }

  return (
    <>
      <PageHead title="Configuration">
        The effective configuration — defaults, <code>config/*.toml</code>, and environment
        overrides merged, exactly as the running process sees it. Secrets are redacted (shown as
        set or empty, never the value). Read-only: change values in the config file; they are
        validated on the next start.
      </PageHead>

      <input
        type="search"
        placeholder="filter keys and values…"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        className="mb-6 min-h-11 w-full max-w-sm rounded border px-3 py-2 text-sm"
        style={{ borderColor: 'var(--line)', background: 'var(--bg)', color: 'var(--fg)' }}
      />

      {!cfg && !msg ? (
        <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>Loading…</p>
      ) : null}
      {msg ? <p className="mb-4 text-sm" style={{ color: 'var(--warn)' }}>{msg}</p> : null}

      {scalars.filter(([k, v]) => matches(k, v)).length > 0 ? (
        <table className="mb-6 w-full max-w-2xl border-collapse text-sm">
          <tbody>
            {scalars
              .filter(([k, v]) => matches(k, v))
              .map(([k, v]) => (
                <tr key={k}>
                  <td className="border-b py-1 pr-4 font-mono text-xs" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                  <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>{renderValue(v)}</td>
                </tr>
              ))}
          </tbody>
        </table>
      ) : null}

      {sections.map(([name, section]) => {
        const rows = Object.entries(section).filter(([k, v]) => matches(`${name}.${k}`, v))
        if (rows.length === 0) return null
        return (
          <section key={name} className="mb-6">
            <h2 className="mb-1 text-base font-semibold">[{name}]</h2>
            <table className="w-full max-w-2xl border-collapse text-sm">
              <tbody>
                {rows.map(([k, v]) => (
                  <tr key={k}>
                    <td className="border-b py-1 pr-4 font-mono text-xs" style={{ borderColor: 'var(--line)', color: 'var(--fg-muted)' }}>{k}</td>
                    <td className="border-b py-1 font-mono text-xs" style={{ borderColor: 'var(--line)' }}>
                      {typeof v === 'object' && v !== null && !Array.isArray(v)
                        ? JSON.stringify(v)
                        : renderValue(v)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        )
      })}
    </>
  )
}
