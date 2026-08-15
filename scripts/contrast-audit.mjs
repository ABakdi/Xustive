#!/usr/bin/env node
// WCAG AA contrast audit of the colour tokens, both themes (M1B-T02.6).
//
// The design system is oklch tokens in globals.css. oklch is chosen for perceptual uniformity, not
// for legibility — a token can look right and still fail contrast, and nothing in the CSS reveals
// it. This reads the tokens straight from the stylesheet, converts each to linear-light sRGB,
// computes WCAG relative luminance, and checks every foreground/background pair that carries text
// or a control against the AA thresholds: 4.5:1 for body text, 3:1 for large text and UI edges.
//
// Reading the stylesheet rather than a hand-copied table is the point: a token edited in globals.css
// is audited on the next run, with no second list to keep in step.
//
//   node scripts/contrast-audit.mjs

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '../web/app/globals.css'), 'utf8')

// oklch(L% C h) → the three numbers. Alpha and `oklch(L C h / a)` are not used by these tokens.
function parseOklch(str) {
  const m = str.match(/oklch\(\s*([\d.]+)%?\s+([\d.]+)\s+([\d.]+)/)
  if (!m) return null
  return { L: parseFloat(m[1]) / (str.includes('%') ? 100 : 1), C: parseFloat(m[2]), h: parseFloat(m[3]) }
}

// Oklch → linear-light sRGB (Björn Ottosson's matrices). The output is linear, which is exactly
// what WCAG luminance wants — no gamma round-trip needed.
function oklchToLinearRgb({ L, C, h }) {
  const hr = (h * Math.PI) / 180
  const a = C * Math.cos(hr)
  const b = C * Math.sin(hr)
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b
  const s_ = L - 0.0894841775 * a - 1.291485548 * b
  const l = l_ ** 3
  const m = m_ ** 3
  const s = s_ ** 3
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ]
}

// WCAG relative luminance from linear-light RGB. Values are clamped: out-of-gamut tokens can push a
// channel slightly past [0,1], and a negative would corrupt the ratio.
function luminance(oklch) {
  const [r, g, b] = oklchToLinearRgb(oklch).map((v) => Math.min(1, Math.max(0, v)))
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

function contrast(a, b) {
  const la = luminance(a)
  const lb = luminance(b)
  const [hi, lo] = la > lb ? [la, lb] : [lb, la]
  return (hi + 0.05) / (lo + 0.05)
}

// Pull the token block for a selector out of the stylesheet, so light and dark are read from the
// same source they ship from.
function tokens(selector) {
  const start = css.indexOf(selector)
  if (start < 0) throw new Error(`selector not found: ${selector}`)
  const open = css.indexOf('{', start)
  const close = css.indexOf('}', open)
  const block = css.slice(open + 1, close)
  const out = {}
  for (const m of block.matchAll(/(--[\w-]+):\s*(oklch\([^)]*\))/g)) {
    const parsed = parseOklch(m[2])
    if (parsed) out[m[1]] = parsed
  }
  return out
}

// (foreground, background, minimum ratio, what it is). Large/UI edges get 3.0; body text 4.5.
const PAIRS = [
  ['--fg', '--bg', 4.5, 'body text on the page'],
  ['--fg', '--surface', 4.5, 'body text on a card'],
  ['--fg', '--bg-sunk', 4.5, 'body text on a sunk panel'],
  ['--fg-muted', '--bg', 4.5, 'muted text on the page'],
  ['--fg-muted', '--surface', 4.5, 'muted text on a card'],
  ['--accent', '--bg', 4.5, 'a link on the page'],
  ['--accent', '--surface', 4.5, 'a link on a card'],
  ['--accent-fg', '--accent', 4.5, 'button text on the accent'],
  // fg-faint is placeholder/hint text and non-essential edges — the AA floor for those is 3:1.
  ['--fg-faint', '--bg', 3.0, 'faint hint text'],
  ['--line-strong', '--bg', 3.0, 'a strong divider'],
]

const THEMES = [
  ['light', ':root {'],
  ['dark', ":root[data-theme='dark'] {"],
]

let failures = 0
for (const [name, selector] of THEMES) {
  // Dark overrides only the tokens it changes; the rest inherit from :root. Merge so a pair that
  // references an unchanged token is still resolved.
  const base = tokens(':root {')
  const theme = name === 'light' ? base : { ...base, ...tokens(selector) }
  console.log(`\n${name}`)
  for (const [fg, bg, min, what] of PAIRS) {
    if (!theme[fg] || !theme[bg]) {
      console.log(`  ? ${what}: token missing (${fg} on ${bg})`)
      continue
    }
    const ratio = contrast(theme[fg], theme[bg])
    const ok = ratio >= min
    if (!ok) failures++
    console.log(`  ${ok ? '✓' : '✗'} ${what}: ${ratio.toFixed(2)}:1 (need ${min}:1)  ${fg} on ${bg}`)
  }
}

if (failures > 0) {
  console.error(`\n✗ contrast audit: ${failures} pair(s) below AA`)
  process.exit(1)
}
console.log('\n✓ contrast audit: every text and control pair meets AA in both themes')
