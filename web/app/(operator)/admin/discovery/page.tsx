'use client'

import { getChannels } from '@/lib/admin'
import { PageHead, StatusLine, Table, Td, Th, pct, usePoll } from '@/components/admin/ui'

export default function DiscoveryPage() {
  const { data, error } = usePoll(getChannels, 10_000)
  const rows = data ?? []
  return (
    <>
      <PageHead title="Discovery yield">
        The funnel per discovery channel: how many URLs each introduced, how many were fetched, and
        how many survived to an indexed document. <strong>Yield</strong> is indexed ÷ discovered;{' '}
        <strong>unique</strong> is the share of a channel&rsquo;s documents that were not duplicates
        of something a cheaper channel already found — the number that decides whether an expensive
        channel earns its place.
      </PageHead>
      <StatusLine>
        {error
          ? `Could not load discovery yield: ${error}`
          : rows.length
            ? `${rows.length} channel(s). Refreshing every 10s.`
            : data
              ? 'No discovery activity recorded yet.'
              : 'Loading…'}
      </StatusLine>
      <Table
        head={
          <>
            <Th>channel</Th>
            <Th num>discovered</Th>
            <Th num>fetched</Th>
            <Th num>indexed</Th>
            <Th num>duplicate</Th>
            <Th num>yield</Th>
            <Th num>unique</Th>
          </>
        }
      >
        {rows.map((c) => (
          <tr key={c.channel}>
            <Td>{c.channel}</Td>
            <Td num>{c.discovered}</Td>
            <Td num>{c.fetched}</Td>
            <Td num>{c.indexed}</Td>
            <Td num>{c.duplicate}</Td>
            <Td num>{pct(c.yield_rate)}</Td>
            <Td num>{pct(c.unique_rate)}</Td>
          </tr>
        ))}
      </Table>
    </>
  )
}
