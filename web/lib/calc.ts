/**
 * A small expression evaluator for the interactive calculator: `+ - * / ^ %`, parentheses,
 * unary minus, `sqrt(…)`, and the symbols people type (× ÷ − , Arabic digits). `%` after a
 * number is "per cent" (`15% * 80` → 12), the reading the API's tool uses; it never means
 * modulo here. Returns `null` for anything it cannot read — the display shows the expression
 * unchanged rather than a guess.
 */
export function normalise(expr: string): string {
  return expr
    .replace(/[٠-٩]/g, (d) => String(d.charCodeAt(0) - 0x0660))
    .replace(/[۰-۹]/g, (d) => String(d.charCodeAt(0) - 0x06f0))
    .replace(/[×✕x]/gi, '*')
    .replace(/÷/g, '/')
    .replace(/−/g, '-')
    .replace(/٪/g, '%')
    .replace(/٫/g, '.')
    .replace(/،/g, '')
    .replace(/√/g, 'sqrt')
}

type Tok = { t: 'n'; v: number } | { t: 'op'; v: string } | { t: '('; v: '(' } | { t: ')'; v: ')' } | { t: 'fn'; v: string }

function lex(s: string): Tok[] | null {
  const out: Tok[] = []
  let i = 0
  while (i < s.length) {
    const c = s[i] ?? ''
    if (c === ' ' || c === ',') {
      i++
      continue
    }
    if (/[0-9.]/.test(c)) {
      let j = i
      while (j < s.length && /[0-9.]/.test(s[j] ?? '')) j++
      const v = Number(s.slice(i, j))
      if (Number.isNaN(v)) return null
      out.push({ t: 'n', v })
      i = j
      // A trailing `%` scales the number it follows.
      if (s[i] === '%') {
        out[out.length - 1] = { t: 'n', v: v / 100 }
        i++
      }
      continue
    }
    if ('+-*/^'.includes(c)) {
      out.push({ t: 'op', v: c })
      i++
      continue
    }
    if (c === '(' || c === ')') {
      out.push({ t: c, v: c } as Tok)
      i++
      continue
    }
    if (/[a-z]/i.test(c)) {
      let j = i
      while (j < s.length && /[a-z]/i.test(s[j] ?? '')) j++
      const name = s.slice(i, j).toLowerCase()
      if (name === 'sqrt') out.push({ t: 'fn', v: name })
      else if (name === 'pi') out.push({ t: 'n', v: Math.PI })
      else if (name === 'e') out.push({ t: 'n', v: Math.E })
      else return null
      i = j
      continue
    }
    return null
  }
  return out
}

export function evaluate(expr: string): number | null {
  const toks = lex(normalise(expr))
  if (!toks || toks.length === 0) return null
  let pos = 0
  const peek = () => toks[pos]
  const next = () => toks[pos++]

  function primary(): number | null {
    const t = next()
    if (!t) return null
    if (t.t === 'n') return t.v
    if (t.t === 'op' && t.v === '-') {
      const v = unary()
      return v === null ? null : -v
    }
    if (t.t === 'op' && t.v === '+') return unary()
    if (t.t === 'fn') {
      const v = unary()
      return v === null || v < 0 ? null : Math.sqrt(v)
    }
    if (t.t === '(') {
      const v = expression()
      const close = next()
      return close && close.t === ')' ? v : null
    }
    return null
  }
  function unary(): number | null {
    return primary()
  }
  function power(): number | null {
    const base = unary()
    if (base === null) return null
    const t = peek()
    if (t && t.t === 'op' && t.v === '^') {
      next()
      const exp = power() // right-associative
      return exp === null ? null : Math.pow(base, exp)
    }
    return base
  }
  function term(): number | null {
    let v = power()
    if (v === null) return null
    for (;;) {
      const t = peek()
      if (t && t.t === 'op' && (t.v === '*' || t.v === '/')) {
        next()
        const r = power()
        if (r === null) return null
        v = t.v === '*' ? v * r : v / r
      } else if (t && (t.t === 'n' || t.t === '(' || t.t === 'fn')) {
        // Implicit multiplication: `2(3+4)`, `3 sqrt(4)`.
        const r = power()
        if (r === null) return null
        v = v * r
      } else return v
    }
  }
  function expression(): number | null {
    let v = term()
    if (v === null) return null
    for (;;) {
      const t = peek()
      if (t && t.t === 'op' && (t.v === '+' || t.v === '-')) {
        next()
        const r = term()
        if (r === null) return null
        v = t.v === '+' ? v + r : v - r
      } else return v
    }
  }
  const v = expression()
  if (v === null || pos !== toks.length || !Number.isFinite(v)) return null
  return v
}

/** Render like the API: up to ten significant digits, no trailing zeros. */
export function render(n: number): string {
  if (Number.isInteger(n) && Math.abs(n) < 1e15) return String(n)
  const s = Math.abs(n) >= 1e15 || (Math.abs(n) < 1e-9 && n !== 0) ? n.toExponential(6) : n.toPrecision(10)
  return s.includes('e') ? s : s.replace(/\.?0+$/, '')
}
