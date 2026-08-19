import type { Metadata } from 'next'

// These mirror the crawler's constants in `xustive-ingest::robots`. They change rarely; a test on
// the Rust side pins the token a site would actually type, so a drift here is caught there.
const USER_AGENT = 'XustiveBot/1.0 (+https://xustive.dz/bot; Algerian search engine)'
const UA_TOKEN = 'xustivebot'
const DEFAULT_DELAY_S = 1.5
const MAX_DELAY_S = 60

export const metadata: Metadata = {
  title: 'XustiveBot',
  description: 'Who XustiveBot is and how to control it.',
}

function Rule({ children }: { children: string }) {
  return (
    <pre
      className="my-3 overflow-x-auto rounded border p-3 text-sm"
      style={{ borderColor: 'var(--line)', background: 'var(--bg-sunk)' }}
    >
      <code style={{ fontFamily: 'var(--font-mono)' }}>{children}</code>
    </pre>
  )
}

export default function BotPage() {
  return (
    <main className="mx-auto max-w-[720px] px-6 py-10" style={{ color: 'var(--fg)' }}>
      <h1 className="text-2xl font-semibold">XustiveBot</h1>
      <p className="mt-3 text-[1.05rem]" style={{ color: 'var(--fg-muted)' }}>
        Xustive is a search engine for Algeria. XustiveBot is the crawler that builds its index. If
        you found it in your logs, this page is how you control it.
      </p>

      <table className="mt-6 w-full border-collapse text-sm">
        <tbody>
          <tr>
            <th className="border-b py-2 pr-4 text-left align-top" style={{ borderColor: 'var(--line)' }}>User-agent</th>
            <td className="border-b py-2" style={{ borderColor: 'var(--line)' }}><code>{USER_AGENT}</code></td>
          </tr>
          <tr>
            <th className="border-b py-2 pr-4 text-left align-top" style={{ borderColor: 'var(--line)' }}>robots.txt token</th>
            <td className="border-b py-2" style={{ borderColor: 'var(--line)' }}><code>{UA_TOKEN}</code></td>
          </tr>
          <tr>
            <th className="border-b py-2 pr-4 text-left align-top" style={{ borderColor: 'var(--line)' }}>Default delay</th>
            <td className="border-b py-2" style={{ borderColor: 'var(--line)' }}>{DEFAULT_DELAY_S} s per host</td>
          </tr>
          <tr>
            <th className="py-2 pr-4 text-left align-top">Concurrent requests per host</th>
            <td className="py-2">1</td>
          </tr>
        </tbody>
      </table>

      <h2 className="mt-8 text-lg font-semibold">Slow it down</h2>
      <p className="mt-1 text-sm">
        Add this to <code>robots.txt</code>. We honour it up to {MAX_DELAY_S} seconds; beyond that we
        reduce how often we visit instead.
      </p>
      <Rule>{`User-agent: ${UA_TOKEN}\nCrawl-delay: 10`}</Rule>

      <h2 className="mt-8 text-lg font-semibold">Block part of the site</h2>
      <Rule>{`User-agent: ${UA_TOKEN}\nDisallow: /search\nDisallow: /cart`}</Rule>

      <h2 className="mt-8 text-lg font-semibold">Block it entirely</h2>
      <Rule>{`User-agent: ${UA_TOKEN}\nDisallow: /`}</Rule>
      <p className="mt-1 text-sm" style={{ color: 'var(--fg-muted)' }}>
        We re-read <code>robots.txt</code> at least once a day. An unreachable <code>robots.txt</code>{' '}
        — a timeout, a 403, a 5xx — is treated as a full block, not as permission.
      </p>

      <h2 className="mt-8 text-lg font-semibold">Keep pages out of the index without blocking the crawl</h2>
      <p className="mt-1 text-sm">
        Crawling and indexing are separate permissions. The header form works for files with no HTML
        head, such as PDFs.
      </p>
      <Rule>{`<meta name="robots" content="noindex">`}</Rule>
      <Rule>{`X-Robots-Tag: noindex`}</Rule>

      <h2 className="mt-8 text-lg font-semibold">What we do not do</h2>
      <ul className="mt-1 list-disc pl-6 text-sm" style={{ color: 'var(--fg-muted)' }}>
        <li>We do not submit forms, log in, or reach anything behind authentication.</li>
        <li>We do not run more than one request at a time against a host.</li>
        <li>We do not disguise the user-agent or rotate it to avoid blocks.</li>
      </ul>
    </main>
  )
}
