import type { Locale } from './config'

/**
 * Message catalogues.
 *
 * Typed against the Arabic catalogue, so a key missing from any other locale is a **compile
 * error**. A French page with English strings scattered through it looks broken in a way nobody
 * reports — they just leave.
 */
const ar = {
  tagline: 'محرك البحث الجزائري',
  searchPlaceholder: 'ابحث…',
  searchLabel: 'بحث',
  privacyLine: 'ما نسجلوش عمليات البحث تاعك',
  resultsCount: 'نتيجة',
  resultsApprox: 'حوالي',
  took: 'مللي ثانية',
  noResults: 'ما لقينا والو',
  noResultsHint: 'جرب كلمات أخرى ولا أقل',
  filters: 'تصفية',
  language: 'اللغة',
  source: 'المصدر',
  tone: 'الانطباع',
  clearFilters: 'مسح التصفية',
  previous: 'السابق',
  next: 'التالي',
  page: 'صفحة',
  summaryNote: 'مولّد من النتائج تحت. راجع المصادر.',
  dateUnknown: 'التاريخ غير معروف',
  theme: 'المظهر',
  themeLight: 'فاتح',
  themeDark: 'داكن',
  themeSystem: 'حسب النظام',
  positive: 'إيجابي',
  neutral: 'محايد',
  negative: 'سلبي',
  web: 'موقع',
  facebook: 'فيسبوك',
  instagram: 'إنستغرام',
  tiktok: 'تيك توك',
  lang_ar: 'العربية',
  lang_ary: 'الدارجة',
  lang_fr: 'الفرنسية',
  lang_en: 'الإنجليزية',
  lang_mixed: 'مختلط',
  selfHosted: 'مستضاف في الجزائر',
  errorTitle: 'وقعت مشكلة',
} as const

/**
 * The key set, not the values.
 *
 * `typeof ar` alone would make every string a literal type, so `fr` could only contain the
 * Arabic text — which is exactly what the first version did. Mapping to `string` keeps the
 * missing-key check while letting translations be translations.
 */
export type Messages = { [K in keyof typeof ar]: string }

const fr: Messages = {
  tagline: 'Le moteur de recherche algérien',
  searchPlaceholder: 'Rechercher…',
  searchLabel: 'Rechercher',
  privacyLine: 'Nous n’enregistrons pas vos recherches',
  resultsCount: 'résultats',
  resultsApprox: 'environ',
  took: 'ms',
  noResults: 'Aucun résultat',
  noResultsHint: 'Essayez d’autres mots, ou moins de mots',
  filters: 'Filtrer',
  language: 'Langue',
  source: 'Source',
  tone: 'Ton',
  clearFilters: 'Effacer les filtres',
  previous: 'Précédent',
  next: 'Suivant',
  page: 'Page',
  summaryNote: 'Généré à partir des résultats ci-dessous. Vérifiez les sources.',
  dateUnknown: 'Date inconnue',
  theme: 'Thème',
  themeLight: 'Clair',
  themeDark: 'Sombre',
  themeSystem: 'Système',
  positive: 'Positif',
  neutral: 'Neutre',
  negative: 'Négatif',
  web: 'Web',
  facebook: 'Facebook',
  instagram: 'Instagram',
  tiktok: 'TikTok',
  lang_ar: 'Arabe',
  lang_ary: 'Darija',
  lang_fr: 'Français',
  lang_en: 'Anglais',
  lang_mixed: 'Mixte',
  selfHosted: 'hébergé en Algérie',
  errorTitle: 'Une erreur est survenue',
}

const en: Messages = {
  tagline: 'The Algerian search engine',
  searchPlaceholder: 'Search…',
  searchLabel: 'Search',
  privacyLine: 'We don’t log your searches',
  resultsCount: 'results',
  resultsApprox: 'about',
  took: 'ms',
  noResults: 'No results',
  noResultsHint: 'Try different words, or fewer of them',
  filters: 'Filter',
  language: 'Language',
  source: 'Source',
  tone: 'Tone',
  clearFilters: 'Clear filters',
  previous: 'Previous',
  next: 'Next',
  page: 'Page',
  summaryNote: 'Generated from the results below. Check the sources.',
  dateUnknown: 'Date unknown',
  theme: 'Theme',
  themeLight: 'Light',
  themeDark: 'Dark',
  themeSystem: 'System',
  positive: 'Positive',
  neutral: 'Neutral',
  negative: 'Negative',
  web: 'Web',
  facebook: 'Facebook',
  instagram: 'Instagram',
  tiktok: 'TikTok',
  lang_ar: 'Arabic',
  lang_ary: 'Darija',
  lang_fr: 'French',
  lang_en: 'English',
  lang_mixed: 'Mixed',
  selfHosted: 'self-hosted in Algeria',
  errorTitle: 'Something went wrong',
}

/**
 * Darija reuses the Arabic catalogue until a real one exists.
 *
 * Falling back to Arabic rather than English is the whole point: someone who set the interface to
 * Darija reads Arabic, and sending them to English would be a strictly worse guess. Writing Darija
 * UI copy well is harder than translating it and needs a native speaker (blocker B7).
 */
const catalogues: Record<Locale, Messages> = { ar, ary: ar, fr, en }

export function messages(locale: Locale): Messages {
  return catalogues[locale]
}
