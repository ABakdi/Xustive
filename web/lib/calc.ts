/**
 * The scientific calculator's evaluator: real and complex arithmetic over `+ - * / ^ mod`,
 * postfix `! % °`, parentheses and implicit multiplication, the constants `pi π e i`, and the
 * functions a scientific keypad offers (trigonometric and hyperbolic with their inverses, `ln`,
 * `log`/`log10`, `log2`, `exp`, `sqrt`/`√`, `cbrt`, `abs`, `floor`, `ceil`, `round`, `re`, `im`,
 * `arg`, `conj`). Trigonometry reads its argument in the chosen angle mode (degrees by default,
 * like a pocket calculator); a `°` postfix always means degrees whatever the mode.
 *
 * Complex numbers are values, not a mode: `sqrt(-4)` is `2i`, `(2+3i)(1-i)` is `5 + i`,
 * `e^(i*pi)` is `-1`. Everything is done on `{re, im}` pairs; a result with a vanishing
 * imaginary part is shown as a real number. `null` for anything unreadable — the display
 * shows the expression unchanged rather than a guess.
 */
export interface Complex {
  re: number
  im: number
}
export type AngleMode = 'deg' | 'rad'

const C = (re: number, im = 0): Complex => ({ re, im })
const EPS = 1e-12

export function normalise(expr: string): string {
  return expr
    .replace(/[٠-٩]/g, (d) => String(d.charCodeAt(0) - 0x0660))
    .replace(/[۰-۹]/g, (d) => String(d.charCodeAt(0) - 0x06f0))
    .replace(/[×✕]/g, '*')
    .replace(/÷/g, '/')
    .replace(/−/g, '-')
    .replace(/٪/g, '%')
    .replace(/٫/g, '.')
    .replace(/،/g, '')
    .replace(/π/g, ' pi ')
    .replace(/√/g, ' sqrt ')
    .replace(/²/g, '^2')
    .replace(/³/g, '^3')
}

// ---- complex arithmetic ---------------------------------------------------------------------
const add = (a: Complex, b: Complex) => C(a.re + b.re, a.im + b.im)
const sub = (a: Complex, b: Complex) => C(a.re - b.re, a.im - b.im)
const mul = (a: Complex, b: Complex) => C(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
function div(a: Complex, b: Complex): Complex | null {
  const d = b.re * b.re + b.im * b.im
  if (d === 0) return null
  return C((a.re * b.re + a.im * b.im) / d, (a.im * b.re - a.re * b.im) / d)
}
const abs = (a: Complex) => Math.hypot(a.re, a.im)
const arg = (a: Complex) => Math.atan2(a.im, a.re)
const cexp = (a: Complex) => C(Math.exp(a.re) * Math.cos(a.im), Math.exp(a.re) * Math.sin(a.im))
function cln(a: Complex): Complex | null {
  if (a.re === 0 && a.im === 0) return null
  return C(Math.log(abs(a)), arg(a))
}
function cpow(a: Complex, b: Complex): Complex | null {
  if (a.re === 0 && a.im === 0) return b.re > 0 ? C(0) : null
  if (b.im === 0 && a.im === 0 && (a.re > 0 || Number.isInteger(b.re))) return C(Math.pow(a.re, b.re))
  const l = cln(a)
  return l ? cexp(mul(l, b)) : null
}
function csqrt(a: Complex): Complex {
  if (a.im === 0) return a.re >= 0 ? C(Math.sqrt(a.re)) : C(0, Math.sqrt(-a.re))
  const r = abs(a)
  return C(Math.sqrt((r + a.re) / 2), Math.sign(a.im) * Math.sqrt((r - a.re) / 2))
}
const csin = (a: Complex) => C(Math.sin(a.re) * Math.cosh(a.im), Math.cos(a.re) * Math.sinh(a.im))
const ccos = (a: Complex) => C(Math.cos(a.re) * Math.cosh(a.im), -Math.sin(a.re) * Math.sinh(a.im))
const csinh = (a: Complex) => C(Math.sinh(a.re) * Math.cos(a.im), Math.cosh(a.re) * Math.sin(a.im))
const ccosh = (a: Complex) => C(Math.cosh(a.re) * Math.cos(a.im), Math.sinh(a.re) * Math.sin(a.im))
const I = C(0, 1)
function casin(z: Complex): Complex | null {
  // -i ln(iz + sqrt(1 - z²))
  const l = cln(add(mul(I, z), csqrt(sub(C(1), mul(z, z)))))
  return l ? mul(C(0, -1), l) : null
}
function cacos(z: Complex): Complex | null {
  const s = casin(z)
  return s ? sub(C(Math.PI / 2), s) : null
}
function catan(z: Complex): Complex | null {
  // (i/2) ln((i+z)/(i-z))
  const q = div(add(I, z), sub(I, z))
  const l = q ? cln(q) : null
  return l ? mul(C(0, 0.5), l) : null
}
function factorial(n: number): number | null {
  if (!Number.isInteger(n) || n < 0 || n > 170) return null
  let f = 1
  for (let k = 2; k <= n; k++) f *= k
  return f
}
const isReal = (z: Complex) => Math.abs(z.im) < EPS

// ---- lexer ----------------------------------------------------------------------------------
type Tok =
  | { t: 'n'; v: number }
  | { t: 'id'; v: string }
  | { t: 'op'; v: string }
  | { t: 'post'; v: string }
  | { t: '(' }
  | { t: ')' }
  | { t: ',' }

const FUNCTIONS = new Set([
  'sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'sinh', 'cosh', 'tanh', 'ln', 'log', 'log2',
  'log10', 'exp', 'sqrt', 'cbrt', 'abs', 'floor', 'ceil', 'round', 're', 'im', 'arg', 'conj',
])

function lex(s: string): Tok[] | null {
  const out: Tok[] = []
  let i = 0
  while (i < s.length) {
    const c = s[i] ?? ''
    if (c === ' ') {
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
      continue
    }
    if ('+-*/^'.includes(c)) {
      out.push({ t: 'op', v: c })
      i++
      continue
    }
    if (c === '!' || c === '%' || c === '°') {
      out.push({ t: 'post', v: c })
      i++
      continue
    }
    if (c === '(' || c === ')' || c === ',') {
      out.push({ t: c } as Tok)
      i++
      continue
    }
    if (/[a-z]/i.test(c)) {
      let j = i
      while (j < s.length && /[a-z0-9]/i.test(s[j] ?? '')) j++
      let name = s.slice(i, j).toLowerCase()
      // "2pi", "3i", "45deg": a number glued to a word was lexed above, so this is the word;
      // but "2x3" must stay an x-multiplication, and "log10" a function name.
      if (name === 'x' && out.length > 0 && out[out.length - 1]?.t === 'n') {
        out.push({ t: 'op', v: '*' })
        i = j
        continue
      }
      if (name === 'mod') out.push({ t: 'op', v: 'mod' })
      else if (name === 'deg') out.push({ t: 'post', v: '°' })
      else if (name === 'rad') out.push({ t: 'post', v: 'rad' })
      else if (FUNCTIONS.has(name) || name === 'pi' || name === 'e' || name === 'i') out.push({ t: 'id', v: name })
      else {
        // Glued single-letter constants: "2ei" is rare; "3i" and "2e" are lexed as digits then id.
        name = name
        return null
      }
      i = j
      continue
    }
    return null
  }
  return out
}

// ---- parser (recursive descent) --------------------------------------------------------------
export function evaluate(expr: string, mode: AngleMode = 'deg'): Complex | null {
  const toks = lex(normalise(expr))
  if (!toks || toks.length === 0) return null
  let pos = 0
  const peek = () => toks[pos]
  const next = () => toks[pos++]
  const toRad = (z: Complex) => (mode === 'deg' ? mul(z, C(Math.PI / 180)) : z)
  const fromRad = (z: Complex) => (mode === 'deg' ? mul(z, C(180 / Math.PI)) : z)

  function call(name: string, a: Complex): Complex | null {
    switch (name) {
      case 'sin': return csin(toRad(a))
      case 'cos': return ccos(toRad(a))
      case 'tan': return div(csin(toRad(a)), ccos(toRad(a)))
      case 'asin': { const r = casin(a); return r ? fromRad(r) : null }
      case 'acos': { const r = cacos(a); return r ? fromRad(r) : null }
      case 'atan': { const r = catan(a); return r ? fromRad(r) : null }
      case 'sinh': return csinh(a)
      case 'cosh': return ccosh(a)
      case 'tanh': return div(csinh(a), ccosh(a))
      case 'ln': return cln(a)
      case 'log': case 'log10': { const l = cln(a); return l ? div(l, C(Math.LN10)) : null }
      case 'log2': { const l = cln(a); return l ? div(l, C(Math.LN2)) : null }
      case 'exp': return cexp(a)
      case 'sqrt': return csqrt(a)
      case 'cbrt': return isReal(a) ? C(Math.cbrt(a.re)) : cpow(a, C(1 / 3))
      case 'abs': return C(abs(a))
      case 'floor': return isReal(a) ? C(Math.floor(a.re)) : null
      case 'ceil': return isReal(a) ? C(Math.ceil(a.re)) : null
      case 'round': return isReal(a) ? C(Math.round(a.re)) : null
      case 're': return C(a.re)
      case 'im': return C(a.im)
      case 'arg': return fromRad(C(arg(a)))
      case 'conj': return C(a.re, -a.im)
      default: return null
    }
  }

  function primary(): Complex | null {
    const t = next()
    if (!t) return null
    if (t.t === 'n') return C(t.v)
    if (t.t === 'op' && t.v === '-') { const v = unary(); return v ? mul(v, C(-1)) : null }
    if (t.t === 'op' && t.v === '+') return unary()
    if (t.t === 'id') {
      if (t.v === 'pi') return C(Math.PI)
      if (t.v === 'e') return C(Math.E)
      if (t.v === 'i') return I
      // A function: parenthesised argument, or the next power-level term ("ln 5", "sin 30°").
      const p = peek()
      let a: Complex | null
      if (p && p.t === '(') {
        next()
        a = expression()
        const close = next()
        if (!close || close.t !== ')') return null
      } else a = power()
      return a ? call(t.v, a) : null
    }
    if (t.t === '(') {
      const v = expression()
      const close = next()
      return close && close.t === ')' ? v : null
    }
    return null
  }
  function postfix(): Complex | null {
    let v = primary()
    if (!v) return null
    for (;;) {
      const t = peek()
      if (t && t.t === 'post') {
        next()
        if (t.v === '!') { if (!isReal(v)) return null; const f = factorial(v.re); if (f === null) return null; v = C(f) }
        else if (t.v === '%') v = mul(v, C(0.01))
        else if (t.v === '°') v = mode === 'deg' ? v : mul(v, C(Math.PI / 180))
        else if (t.v === 'rad') v = mode === 'rad' ? v : mul(v, C(180 / Math.PI))
      } else return v
    }
  }
  function unary(): Complex | null {
    return postfix()
  }
  function power(): Complex | null {
    const base = unary()
    if (!base) return null
    const t = peek()
    if (t && t.t === 'op' && t.v === '^') {
      next()
      const exp = power()
      return exp ? cpow(base, exp) : null
    }
    return base
  }
  function term(): Complex | null {
    let v = power()
    if (!v) return null
    for (;;) {
      const t = peek()
      if (t && t.t === 'op' && (t.v === '*' || t.v === '/' || t.v === 'mod')) {
        next()
        const r = power()
        if (!r) return null
        if (t.v === '*') v = mul(v, r)
        else if (t.v === '/') { const q = div(v, r); if (!q) return null; v = q }
        else { if (!isReal(v) || !isReal(r) || r.re === 0) return null; v = C(((v.re % r.re) + r.re) % r.re) }
      } else if (t && (t.t === 'n' || t.t === '(' || t.t === 'id')) {
        // Implicit multiplication: `2(3+4)`, `2pi`, `3i`, `2 sqrt 2`.
        const r = power()
        if (!r) return null
        v = mul(v, r)
      } else return v
    }
  }
  function expression(): Complex | null {
    let v = term()
    if (!v) return null
    for (;;) {
      const t = peek()
      if (t && t.t === 'op' && (t.v === '+' || t.v === '-')) {
        next()
        const r = term()
        if (!r) return null
        v = t.v === '+' ? add(v, r) : sub(v, r)
      } else return v
    }
  }
  const v = expression()
  if (!v || pos !== toks.length || !Number.isFinite(v.re) || !Number.isFinite(v.im)) return null
  return v
}

/** Ten significant digits, no trailing zeros; complex as `a + bi`. */
function real(n: number): string {
  if (Math.abs(n) < EPS) return '0'
  if (Number.isInteger(n) && Math.abs(n) < 1e15) return String(n)
  const r = Number(n.toPrecision(10))
  if (Number.isInteger(r) && Math.abs(r) < 1e15) return String(r)
  const s = Math.abs(r) >= 1e15 || Math.abs(r) < 1e-9 ? r.toExponential(6) : String(r)
  return s
}
export function render(z: Complex): string {
  const re = Math.abs(z.re) < EPS ? 0 : z.re
  const im = Math.abs(z.im) < EPS ? 0 : z.im
  if (im === 0) return real(re)
  const imPart = Math.abs(im) === 1 ? 'i' : `${real(Math.abs(im))}i`
  if (re === 0) return `${im < 0 ? '-' : ''}${imPart}`
  return `${real(re)} ${im < 0 ? '-' : '+'} ${imPart}`
}
