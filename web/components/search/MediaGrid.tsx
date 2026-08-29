import type { ResultCard as Result } from '@/lib/api'
import type { Messages } from '@/lib/i18n/messages'
import { signThumb } from '@/lib/thumb'

/**
 * The Images and Videos tabs (M9-T03).
 *
 * Pure server components: a CSS grid of `<img loading="lazy">` through the signed proxy, and a
 * list of video tiles that link out. No JavaScript ships for either — a charting-library-sized
 * gallery script would fail the no-JS path and the bundle budget for nothing the browser cannot
 * already do.
 *
 * A tile is *a page that has this image*. The title and host are the page's; the image is one of
 * the ones the parser found on it. Clicking goes to the page, because the page is what we indexed
 * and what a reader can judge.
 */

const hostOf = (url: string) => {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return ''
  }
}

type Tile = { key: string; src: string; upstream: string; page: Result }

/** One tile per image, in result order, so relevance still reads left-to-right, top-to-bottom. */
function imageTiles(results: Result[]): Tile[] {
  const tiles: Tile[] = []
  for (const r of results) {
    for (const m of r.media ?? []) {
      if (m.kind !== 'image') continue
      const upstream = m.thumb_url ?? m.url
      const src = signThumb(upstream)
      if (!src) continue
      tiles.push({ key: `${r.id}:${m.url}`, src, upstream, page: r })
    }
  }
  return tiles
}

export function ImageGrid({ results, t, lang }: { results: Result[]; t: Messages; lang?: string }) {
  const tiles = imageTiles(results)
  if (tiles.length === 0) return null
  return (
    <ul
      className="m-0 grid list-none gap-3 p-0"
      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(100%, 150px), 1fr))' }}
      aria-label={t.verticalImages}
    >
      {tiles.map((tile) => (
        <li key={tile.key} className="min-w-0">
          <a
            href={tile.page.url}
            className="block no-underline"
            rel="noopener noreferrer nofollow"
            dir="auto"
          >
            {/* Proxied same-origin and signed; the browser never contacts the crawled host.
                A plain img because the source is the open web, not a configured domain. */}
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={tile.src}
              alt={tile.page.title}
              loading="lazy"
              decoding="async"
              referrerPolicy="no-referrer"
              className="block aspect-[4/3] w-full rounded-[var(--radius-sm)] object-cover"
              style={{ background: 'var(--bg-sunk)' }}
            />
            <span className="mt-1 line-clamp-2 block text-sm" style={{ color: 'var(--fg)' }}>
              <bdi>{tile.page.title}</bdi>
            </span>
            <span className="block text-xs" style={{ color: 'var(--fg-muted)' }}>
              <bdi>{hostOf(tile.page.url)}</bdi>
              {/* A live federated hit, not yet in the local index (M9-T06). Badged for the same
                  reason the list badges it: provenance is the reader's to judge. */}
              {tile.page.from_web && (
                <span className="ms-1" style={{ color: 'var(--accent)' }}>
                  {t.fromTheWeb}
                </span>
              )}
            </span>
          </a>
          {/* Search with this picture (M10-T04.1): a real link that names the picture by its
              signed proxy URL, so the reverse page can fetch it through our own rules — no
              upload, and no crawled host learns who asked. */}
          {lang && (
            <a
              href={`/${lang}/search/image?${new URL(tile.src, 'http://x').searchParams.toString()}`}
              className="mt-1 block text-[11px] no-underline hover:underline"
              style={{ color: 'var(--fg-faint)' }}
            >
              {(t as unknown as Record<string, string>).reverseFindSimilar ?? 'Find similar'}
            </a>
          )}
        </li>
      ))}
    </ul>
  )
}

const PROVIDER_NAMES: Record<string, string> = {
  youtube: 'YouTube',
  dailymotion: 'Dailymotion',
  vimeo: 'Vimeo',
}

/**
 * Video tiles. The poster is proxied like any image; the tile links to the **watch page** and
 * nothing is embedded — an embedded player is a third-party page load the reader did not choose
 * (ADR-0021). The provider is named because leaving our site is the reader's decision.
 */
export function VideoList({ results, t }: { results: Result[]; t: Messages }) {
  const tiles = results.flatMap((r) =>
    (r.media ?? [])
      .filter((m) => m.kind === 'video')
      .map((m) => ({
        key: `${r.id}:${m.url}`,
        watch: m.url,
        poster: m.thumb_url ? signThumb(m.thumb_url) : null,
        provider: m.provider ?? 'self',
        page: r,
      })),
  )
  if (tiles.length === 0) return null
  return (
    <ul
      className="m-0 grid list-none gap-4 p-0"
      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(100%, 150px), 1fr))' }}
      aria-label={t.verticalVideos}
    >
      {tiles.map((tile) => {
        const providerName = PROVIDER_NAMES[tile.provider] ?? hostOf(tile.watch)
        return (
          <li key={tile.key} className="min-w-0">
            <a
              href={tile.watch}
              className="block no-underline"
              rel="noopener noreferrer nofollow"
              target="_blank"
              dir="auto"
            >
              <span
                className="relative block aspect-video w-full overflow-hidden rounded-[var(--radius-sm)]"
                style={{ background: 'var(--bg-sunk)' }}
              >
                {tile.poster && (
                  // eslint-disable-next-line @next/next/no-img-element
                  <img
                    src={tile.poster}
                    alt=""
                    loading="lazy"
                    decoding="async"
                    referrerPolicy="no-referrer"
                    className="block h-full w-full object-cover"
                  />
                )}
                <span
                  aria-hidden
                  className="absolute inset-0 flex items-center justify-center text-4xl"
                  style={{ color: 'white', textShadow: '0 1px 6px rgba(0,0,0,.6)' }}
                >
                  ▶
                </span>
              </span>
              <span className="mt-1 line-clamp-2 block text-sm" style={{ color: 'var(--fg)' }}>
                <bdi>{tile.page.title}</bdi>
              </span>
              <span className="block text-xs" style={{ color: 'var(--fg-muted)' }}>
                {t.watchOn} <bdi>{providerName}</bdi> · <bdi>{hostOf(tile.page.url)}</bdi>
                {tile.page.from_web && (
                  <span className="ms-1" style={{ color: 'var(--accent)' }}>
                    {t.fromTheWeb}
                  </span>
                )}
              </span>
            </a>
          </li>
        )
      })}
    </ul>
  )
}
