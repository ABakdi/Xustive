import { notFound } from 'next/navigation'

/**
 * The responsive audit harness ([[UI - Responsive]] §4) — **development only**.
 *
 * `/responsive?path=/fr/search%3Fq%3Dtest&w=360,390,768` renders the given page in one iframe per
 * width. The iframes are same-origin, so `window.audit()` can reach into each one and report what
 * overflows, by how much, and how many controls are under 40 px — measurements, rather than an
 * opinion about a screenshot. Media queries inside an iframe respond to the iframe's own width,
 * which is why this works where resizing a desktop browser window does not.
 */
const PAGE = `<!doctype html><meta charset="utf-8"><title>responsive audit</title>
<style>
 body{margin:0;font:12px system-ui;background:#111;color:#eee}
 .row{display:flex;gap:12px;align-items:flex-start;padding:8px;overflow-x:auto}
 figure{margin:0} figcaption{padding:4px 0}
 iframe{border:1px solid #555;background:#fff;height:820px}
 pre{white-space:pre-wrap;padding:8px;margin:0;border-top:1px solid #333}
</style>
<div class="row" id="row"></div>
<pre id="out">window.audit() — or press A</pre>
<script>
const p = new URLSearchParams(location.search)
const path = p.get('path') || '/fr/search?q=couscous'
const widths = (p.get('w') || '360,390,768,1024').split(',').map(Number)
const row = document.getElementById('row')
for (const w of widths) {
  const fig = document.createElement('figure')
  fig.innerHTML = '<figcaption>' + w + 'px</figcaption>'
  const f = document.createElement('iframe')
  f.width = w; f.src = path; f.dataset.w = w
  fig.appendChild(f); row.appendChild(fig)
}
window.audit = () => [...document.querySelectorAll('iframe')].map(f => {
  try {
    const d = f.contentDocument, de = d.documentElement, vw = de.clientWidth
    // An element outside the viewport is only a bug when nothing above it scrolls horizontally:
    // a table inside its wrapper and a tab strip inside .scroll-x are the *fix*, not the fault.
    const scrolls = e => {
      for (let n = e.parentElement; n && n !== d.body; n = n.parentElement) {
        const ov = f.contentWindow.getComputedStyle(n).overflowX
        if (ov === 'auto' || ov === 'scroll') return true
      }
      return false
    }
    // Measured against the body box, not the viewport: in RTL the vertical scrollbar sits on the
    // left, so every full-width element legitimately ends at the window edge rather than at
    // clientWidth, and comparing to the viewport reported the whole page as broken in Arabic.
    const box = d.body.getBoundingClientRect()
    const over = [...d.querySelectorAll('body *')].filter(e => {
      const r = e.getBoundingClientRect()
      return r.width > 0 && r.height > 0 && (r.right > box.right + 1 || r.left < box.left - 1) && !scrolls(e)
    })
    const seen = new Set(), items = []
    for (const e of over) {
      const key = e.tagName + '.' + (e.className || '').toString().slice(0, 40)
      if (seen.has(key)) continue
      seen.add(key)
      const r = e.getBoundingClientRect()
      items.push({ tag: e.tagName.toLowerCase(), cls: (e.className || '').toString().slice(0, 50), w: Math.round(r.width), right: Math.round(r.right) })
      if (items.length >= 8) break
    }
    // Informational only: the 40px floor is a coarse-pointer rule, and a desktop iframe
    // reports a fine pointer, so this counts what a phone would grow rather than what is wrong.
    const small = [...d.querySelectorAll('button,select,[role=button],nav a,.chip')]
      .filter(e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height < 40 }).length
    return { width: +f.dataset.w, vw, overflowsBy: de.scrollWidth - vw, offenders: items, under40: small }
  } catch (e) { return { width: +f.dataset.w, error: String(e).slice(0, 80) } }
})
addEventListener('keydown', e => { if (e.key === 'a') document.getElementById('out').textContent = JSON.stringify(window.audit(), null, 1) })
</script>`

export function GET() {
  if (process.env.NODE_ENV === 'production') notFound()
  return new Response(PAGE, { headers: { 'content-type': 'text/html; charset=utf-8' } })
}
