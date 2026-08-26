import { createHash } from 'node:crypto'

/**
 * The thumbnail URL Wikimedia Commons serves for a file, built rather than discovered.
 *
 * `Special:FilePath/<name>?width=` answers with three redirects before the bytes arrive, and a row
 * of twelve cards fired twelve of those at once through a four-second leash: the photos that
 * arrived were the lucky ones. Commons lays thumbnails out by the MD5 of the underscored file
 * name — `/a/a0/Name.jpg/240px-Name.jpg` — which has been stable for as long as Commons has
 * existed, so the final address is one hash away and the proxy makes one request.
 *
 * Vector and TIFF originals are served as raster thumbnails with a suffix; everything else keeps
 * its own name. The width must be one Commons pre-renders — 120, 250, 330, 500 — and nothing
 * else: 240 was refused with a 400 on the day this was written, while 250 was served from cache.
 */
export function commonsThumbUrl(fileName: string, width: 120 | 250 | 330 | 500 = 250): string {
  const name = fileName.trim().replace(/^File:/i, '').replace(/ /g, '_')
  const hash = createHash('md5').update(name).digest('hex')
  const lower = name.toLowerCase()
  const rendered = lower.endsWith('.svg')
    ? `${name}.png`
    : lower.endsWith('.tif') || lower.endsWith('.tiff') || lower.endsWith('.pdf') || lower.endsWith('.djvu')
      ? `${name}.jpg`
      : name
  return (
    `https://upload.wikimedia.org/wikipedia/commons/thumb/${hash[0]}/${hash.slice(0, 2)}/` +
    `${encodeURIComponent(name)}/${width}px-${encodeURIComponent(rendered)}`
  )
}
