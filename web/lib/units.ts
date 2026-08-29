/**
 * The unit table behind the interactive converter — a mirror of `xustive-tools::units::UNITS`
 * (same canonical names, same factors), so what the API parsed from the query and what the
 * reader then changes in the dropdowns agree to the digit. Factors are "per base unit" of the
 * dimension: metre, kilogram, °C, square metre, litre, km/h, byte.
 */
export type Dimension = 'length' | 'mass' | 'temperature' | 'area' | 'volume' | 'speed' | 'data'

export interface Unit {
  name: string
  dimension: Dimension
  perBase: number
  label: { en: string; fr: string; ar: string }
}

export const UNITS: Unit[] = [
  { name: 'metre', dimension: 'length', perBase: 1, label: { en: 'metre', fr: 'mètre', ar: 'متر' } },
  { name: 'kilometre', dimension: 'length', perBase: 1000, label: { en: 'kilometre', fr: 'kilomètre', ar: 'كيلومتر' } },
  { name: 'centimetre', dimension: 'length', perBase: 0.01, label: { en: 'centimetre', fr: 'centimètre', ar: 'سنتيمتر' } },
  { name: 'millimetre', dimension: 'length', perBase: 0.001, label: { en: 'millimetre', fr: 'millimètre', ar: 'مليمتر' } },
  { name: 'mile', dimension: 'length', perBase: 1609.344, label: { en: 'mile', fr: 'mile', ar: 'ميل' } },
  { name: 'foot', dimension: 'length', perBase: 0.3048, label: { en: 'foot', fr: 'pied', ar: 'قدم' } },
  { name: 'inch', dimension: 'length', perBase: 0.0254, label: { en: 'inch', fr: 'pouce', ar: 'بوصة' } },
  { name: 'kilogram', dimension: 'mass', perBase: 1, label: { en: 'kilogram', fr: 'kilogramme', ar: 'كيلوغرام' } },
  { name: 'gram', dimension: 'mass', perBase: 0.001, label: { en: 'gram', fr: 'gramme', ar: 'غرام' } },
  { name: 'tonne', dimension: 'mass', perBase: 1000, label: { en: 'tonne', fr: 'tonne', ar: 'طن' } },
  { name: 'pound', dimension: 'mass', perBase: 0.45359237, label: { en: 'pound', fr: 'livre', ar: 'رطل' } },
  { name: 'qintar', dimension: 'mass', perBase: 100, label: { en: 'qintar', fr: 'quintal', ar: 'قنطار' } },
  { name: 'celsius', dimension: 'temperature', perBase: 1, label: { en: '°C', fr: '°C', ar: 'درجة مئوية' } },
  { name: 'fahrenheit', dimension: 'temperature', perBase: 1, label: { en: '°F', fr: '°F', ar: 'فهرنهايت' } },
  { name: 'kelvin', dimension: 'temperature', perBase: 1, label: { en: 'kelvin', fr: 'kelvin', ar: 'كلفن' } },
  { name: 'square metre', dimension: 'area', perBase: 1, label: { en: 'square metre', fr: 'mètre carré', ar: 'متر مربع' } },
  { name: 'hectare', dimension: 'area', perBase: 10000, label: { en: 'hectare', fr: 'hectare', ar: 'هكتار' } },
  { name: "sa'a", dimension: 'area', perBase: 400, label: { en: "sa'a", fr: 'saa', ar: 'ساعة' } },
  { name: 'square kilometre', dimension: 'area', perBase: 1_000_000, label: { en: 'square kilometre', fr: 'kilomètre carré', ar: 'كيلومتر مربع' } },
  { name: 'litre', dimension: 'volume', perBase: 1, label: { en: 'litre', fr: 'litre', ar: 'لتر' } },
  { name: 'millilitre', dimension: 'volume', perBase: 0.001, label: { en: 'millilitre', fr: 'millilitre', ar: 'مليلتر' } },
  { name: 'cubic metre', dimension: 'volume', perBase: 1000, label: { en: 'cubic metre', fr: 'mètre cube', ar: 'متر مكعب' } },
  { name: 'kilometre per hour', dimension: 'speed', perBase: 1, label: { en: 'km/h', fr: 'km/h', ar: 'كلم/سا' } },
  { name: 'mile per hour', dimension: 'speed', perBase: 1.609344, label: { en: 'mph', fr: 'mph', ar: 'ميل/سا' } },
  { name: 'metre per second', dimension: 'speed', perBase: 3.6, label: { en: 'm/s', fr: 'm/s', ar: 'م/ث' } },
  { name: 'byte', dimension: 'data', perBase: 1, label: { en: 'byte', fr: 'octet', ar: 'بايت' } },
  { name: 'kilobyte', dimension: 'data', perBase: 1024, label: { en: 'kilobyte', fr: 'kilo-octet', ar: 'كيلوبايت' } },
  { name: 'megabyte', dimension: 'data', perBase: 1048576, label: { en: 'megabyte', fr: 'mégaoctet', ar: 'ميغابايت' } },
  { name: 'gigabyte', dimension: 'data', perBase: 1073741824, label: { en: 'gigabyte', fr: 'gigaoctet', ar: 'غيغابايت' } },
]

export const DIMENSION_LABEL: Record<Dimension, { en: string; fr: string; ar: string }> = {
  length: { en: 'Length', fr: 'Longueur', ar: 'الطول' },
  mass: { en: 'Mass', fr: 'Masse', ar: 'الكتلة' },
  temperature: { en: 'Temperature', fr: 'Température', ar: 'الحرارة' },
  area: { en: 'Area', fr: 'Surface', ar: 'المساحة' },
  volume: { en: 'Volume', fr: 'Volume', ar: 'الحجم' },
  speed: { en: 'Speed', fr: 'Vitesse', ar: 'السرعة' },
  data: { en: 'Data', fr: 'Données', ar: 'البيانات' },
}

export function unitLabel(u: Unit, locale: string): string {
  return locale === 'fr' ? u.label.fr : locale === 'ar' || locale === 'ary' ? u.label.ar : u.label.en
}

export function findUnit(name: string): Unit | undefined {
  return UNITS.find((u) => u.name === name)
}

/** Convert `value` from one unit to another of the same dimension; `null` across dimensions. */
export function convert(value: number, from: Unit, to: Unit): number | null {
  if (from.dimension !== to.dimension) return null
  if (from.dimension === 'temperature') {
    const c = from.name === 'celsius' ? value : from.name === 'fahrenheit' ? (value - 32) / 1.8 : value - 273.15
    return to.name === 'celsius' ? c : to.name === 'fahrenheit' ? c * 1.8 + 32 : c + 273.15
  }
  return (value * from.perBase) / to.perBase
}

/** Ten significant digits, trailing zeros trimmed — the same rendering the API uses. */
export function trim(n: number): string {
  if (!Number.isFinite(n)) return '—'
  const s = Math.abs(n) >= 1e15 || (Math.abs(n) < 1e-9 && n !== 0) ? n.toExponential(6) : n.toPrecision(10)
  return s.includes('e') ? s : s.replace(/\.?0+$/, '')
}
