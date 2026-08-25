'use client'

import { getEval, type EvalReport } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, usePoll } from '@/components/admin/ui'

const pct = (v?: number) => (v == null ? '—' : `${(v * 100).toFixed(1)}%`)
const score = (v?: number) => (v == null ? '—' : v.toFixed(4))

/** Per-language nDCG as one readable line — languages vary per report, so no fixed columns. */
function langs(r: EvalReport) {
  const entries = Object.entries(r.by_language ?? {})
  if (!entries.length) return '—'
  return entries
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([lang, v]) => `${lang} ${v.toFixed(2)}`)
    .join(' · ')
}

/**
 * The quality trail (PROB-003 item 4): every report the eval harness, A/B runner, calibrator, and
 * SERP yardstick wrote, plus the regression-gate verdict and the miner sheets awaiting review.
 * Read-only on purpose — re-baselining and applying calibrations change what "regression" means,
 * so they stay deliberate CLI acts (`make eval`, `xustive ab`, `xustive calibrate`).
 */
export default function EvaluationPage() {
  const { data, error } = usePoll(getEval, 60_000)
  const reports = data?.reports ?? []
  const evals = reports.filter((r) => r.kind === 'eval' || r.kind === 'baseline')
  const abs = reports.filter((r) => r.kind === 'ab')
  const others = reports.filter((r) => r.kind === 'serp' || r.kind === 'calibration')
  const gate = data?.gate

  return (
    <>
      <PageHead title="Evaluation">
        Ranking quality over time, from the reports in <code>eval/reports/</code>. New numbers come
        from running the harness (<code>make eval</code>), not from this page.
      </PageHead>
      <StatusLine>
        {error
          ? `Could not load evaluation reports: ${error}`
          : !data
            ? 'Loading…'
            : reports.length
              ? `${reports.length} report(s) on disk.`
              : 'No reports yet — run `make eval` to produce the first one.'}
      </StatusLine>

      {gate ? (
        <p
          className="mb-6 rounded border px-3 py-2 text-sm"
          style={{ borderColor: gate.pass ? 'var(--line)' : 'var(--warn)', color: gate.pass ? 'var(--fg-muted)' : 'var(--warn)' }}
        >
          Regression gate: <strong>{gate.pass ? 'pass' : 'FAIL'}</strong> — nDCG@10{' '}
          {gate.baseline_ndcg.toFixed(4)} (baseline) → {gate.latest_ndcg.toFixed(4)} ({gate.latest_file}),{' '}
          {gate.delta >= 0 ? '+' : ''}
          {gate.delta.toFixed(4)} against a {gate.tolerance_pct}% relative tolerance.
          {!gate.pass ? ' If the drop is intended, re-baseline deliberately from the CLI.' : ''}
        </p>
      ) : data ? (
        <p className="mb-6 text-sm" style={{ color: 'var(--fg-faint)' }}>
          No gate verdict — it needs both a <code>baseline.json</code> and at least one dated eval report.
        </p>
      ) : null}

      {data?.unreadable?.length ? (
        <p className="mb-6 text-sm" style={{ color: 'var(--warn)' }}>
          Unreadable report file(s) — present but not valid JSON: {data.unreadable.join(', ')}
        </p>
      ) : null}

      {evals.length > 0 ? (
        <div className="mb-8">
          <h2 className="mb-2 text-lg font-semibold">Eval runs</h2>
          <Table
            head={
              <>
                <Th>report</Th>
                <Th num>queries</Th>
                <Th num>nDCG@10</Th>
                <Th num>MRR@10</Th>
                <Th num>recall@50</Th>
                <Th num>zero-result</Th>
                <Th>nDCG by language</Th>
              </>
            }
          >
            {evals.map((r) => (
              <tr key={r.file}>
                <Td>
                  {r.file}
                  {r.kind === 'baseline' ? ' (gate reference)' : ''}
                </Td>
                <Td num>{r.queries ?? '—'}</Td>
                <Td num>{score(r.ndcg_at_10)}</Td>
                <Td num>{score(r.mrr_at_10)}</Td>
                <Td num>{score(r.recall_at_50)}</Td>
                <Td num>{pct(r.zero_result_rate)}</Td>
                <Td>{langs(r)}</Td>
              </tr>
            ))}
          </Table>
        </div>
      ) : null}

      {abs.map((r) => (
        <div key={r.file} className="mb-8">
          <h2 className="mb-2 text-lg font-semibold">A/B — {r.file}</h2>
          <Table
            head={
              <>
                <Th>variant</Th>
                <Th num>nDCG@10</Th>
                <Th num>MRR@10</Th>
                <Th>why</Th>
              </>
            }
          >
            {(r.variants ?? []).map((v) => (
              <tr key={v.name}>
                <Td>{v.name}</Td>
                <Td num>{score(v.ndcg_at_10)}</Td>
                <Td num>{score(v.mrr_at_10)}</Td>
                <Td>{v.why ?? '—'}</Td>
              </tr>
            ))}
          </Table>
        </div>
      ))}

      {others.length > 0 ? (
        <div className="mb-8">
          <h2 className="mb-2 text-lg font-semibold">Calibration &amp; SERP references</h2>
          <Table
            head={
              <>
                <Th>report</Th>
                <Th>kind</Th>
                <Th num>queries</Th>
              </>
            }
          >
            {others.map((r) => (
              <tr key={r.file}>
                <Td>{r.file}</Td>
                <Td>{r.kind === 'serp' ? `SERP yardstick${r.engine ? ` (${r.engine})` : ''}` : 'weight calibration'}</Td>
                <Td num>{r.queries ?? '—'}</Td>
              </tr>
            ))}
          </Table>
          <p className="mt-2 text-sm" style={{ color: 'var(--fg-faint)' }}>
            Full detail stays in the file — calibration recommendations are read, weighed, and
            applied by hand on purpose.
          </p>
        </div>
      ) : null}

      <div className="mb-8">
        <h2 className="mb-2 text-lg font-semibold">Synonym candidates awaiting review</h2>
        {data?.candidates?.length ? (
          <Table
            head={
              <>
                <Th>sheet</Th>
                <Th num>rows</Th>
              </>
            }
          >
            {data.candidates.map((c) => (
              <tr key={c.file}>
                <Td>
                  <code>data/expansion/{c.file}</code>
                </Td>
                <Td num>{c.rows}</Td>
              </tr>
            ))}
          </Table>
        ) : (
          <p className="text-sm" style={{ color: 'var(--fg-faint)' }}>
            None on disk. <code>xustive mine-synonyms</code> writes a dated review sheet; accepted
            rows are moved into <code>synonyms.tsv</code> by hand.
          </p>
        )}
      </div>
    </>
  )
}
