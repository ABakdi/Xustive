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
  date: 'التاريخ',
  copy: 'نسخ',
  copied: 'تم النسخ',
  asOf: 'محسوب في',
  calculator: 'حاسبة',
  'unit-converter': 'تحويل الوحدات',
  'prayer-times': 'أوقات الصلاة',
  wilaya: 'الولاية',
  weather: 'الطقس',
  utility: 'أداة',
  transliterate: 'كتابة بالحروف العربية',
  alternatives: 'قراءة أخرى',
  fuel: 'أسعار الوقود',
  administered: 'سعر مقنّن، ليس تسعيرة لحظية',
  hideTool: 'إخفاء هذه الأداة',
  translate: 'ترجمة',
  translating: 'جارٍ الترجمة…',
  translateFrom: 'من',
  translateTo: 'إلى',
  translateAuto: 'كشف تلقائي',
  translateLocal: 'تتم الترجمة على هذا الخادم. النص لا يغادره.',
  translateApprox: 'ترجمة آلية تقريبية',
  translateTruncated: 'توقفت الترجمة عند الحد الأقصى.',
  translateFailed: 'تعذّرت الترجمة.',
  stop: 'إيقاف',
  settings: 'الإعدادات',
  toolsHeading: 'أدوات الإجابة الفورية',
  toolsNote: 'الأدوات المطفأة لا تظهر فوق النتائج. الإعداد محفوظ في هذا المتصفح فقط.',
  on: 'مفعّلة',
  off: 'مطفأة',
  enable: 'تفعيل',
  disable: 'إطفاء',
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
  date: 'Date',
  copy: 'Copier',
  copied: 'Copié',
  asOf: 'mesuré à',
  calculator: 'Calculatrice',
  'unit-converter': 'Convertisseur',
  'prayer-times': 'Heures de prière',
  wilaya: 'Wilaya',
  weather: 'Météo',
  utility: 'Outil',
  transliterate: 'Translittération',
  alternatives: 'Autre lecture',
  fuel: 'Prix des carburants',
  administered: 'Prix administré, pas une cotation en direct',
  hideTool: 'Masquer cet outil',
  translate: 'Traduction',
  translating: 'Traduction en cours…',
  translateFrom: 'De',
  translateTo: 'Vers',
  translateAuto: 'Détection automatique',
  translateLocal: 'Traduit sur ce serveur. Le texte n’en sort pas.',
  translateApprox: 'Traduction automatique approximative',
  translateTruncated: 'Traduction interrompue à la limite.',
  translateFailed: 'La traduction a échoué.',
  stop: 'Arrêter',
  settings: 'Paramètres',
  toolsHeading: 'Outils de réponse instantanée',
  toolsNote: 'Un outil désactivé n’apparaît plus au-dessus des résultats. Le réglage reste dans ce navigateur.',
  on: 'Activé',
  off: 'Désactivé',
  enable: 'Activer',
  disable: 'Désactiver',
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
  date: 'Date',
  copy: 'Copy',
  copied: 'Copied',
  asOf: 'measured at',
  calculator: 'Calculator',
  'unit-converter': 'Unit converter',
  'prayer-times': 'Prayer times',
  wilaya: 'Wilaya',
  weather: 'Weather',
  utility: 'Utility',
  transliterate: 'Transliteration',
  alternatives: 'Another reading',
  fuel: 'Fuel prices',
  administered: 'Administered price, not a live quote',
  hideTool: 'Hide this tool',
  translate: 'Translation',
  translating: 'Translating…',
  translateFrom: 'From',
  translateTo: 'To',
  translateAuto: 'Detect automatically',
  translateLocal: 'Translated on this server. The text does not leave it.',
  translateApprox: 'Approximate machine translation',
  translateTruncated: 'Translation stopped at the limit.',
  translateFailed: 'Translation failed.',
  stop: 'Stop',
  settings: 'Settings',
  toolsHeading: 'Instant answer tools',
  toolsNote: 'A disabled tool no longer appears above results. The setting stays in this browser.',
  on: 'On',
  off: 'Off',
  enable: 'Enable',
  disable: 'Disable',
}

/**
 * Darija reuses the Arabic catalogue until a real one exists.
 *
 * Falling back to Arabic rather than English is the whole point: someone who set the interface to
 * Darija reads Arabic, and sending them to English would be a strictly worse guess. Writing Darija
 * UI copy well is harder than translating it and needs a native speaker (blocker B7).
 */
/**
 * Algerian Darija.
 *
 * A distinct catalogue rather than an alias for `ar`. Darija was falling back to Arabic wholesale,
 * which is the *right* fallback — MSA is readable to every Darija speaker, and English is not — but
 * a fallback is not a translation. Choosing Darija and getting formal newsreader Arabic tells the
 * user the option was decorative.
 *
 * # What is and is not translated here
 *
 * Only the strings a person would actually say differently. Darija has no settled written
 * standard, so inventing spellings for institutional vocabulary — `الإعدادات`, `الولاية`,
 * `إيجابي` — would produce something no Algerian writes and every Algerian reads more slowly than
 * the MSA they already know from every form and news bulletin. Those rows deliberately keep the
 * Arabic wording.
 *
 * What changes is the conversational register: prompts, empty states, errors, and anything phrased
 * as the site talking to you.
 *
 * MACHINE-GENERATED, UNREVIEWED — blocker B7. Spelling is the part most likely to be wrong: Darija
 * is written by ear and regional habits differ. A reviewer should treat every value here as a
 * proposal.
 */
const ary: Messages = {
  ...ar,
  tagline: 'محرك البحث تاع الجزائر',
  searchPlaceholder: 'قلب على…',
  searchLabel: 'قلب',
  privacyLine: 'ما نسجلوش واش تقلب عليه',
  resultsCount: 'نتيجة',
  resultsApprox: 'تقريبا',
  noResults: 'ما لقينا والو',
  noResultsHint: 'جرب كلمات أخرى ولا قلل فيهم',
  filters: 'نقّي',
  clearFilters: 'امسح التنقية',
  previous: 'اللي قبل',
  next: 'اللي بعد',
  summaryNote: 'هاد الملخص مولّد من النتائج اللي تحت. شوف المصادر.',
  dateUnknown: 'ما نعرفوش التاريخ',
  theme: 'الشكل',
  themeSystem: 'كيما الجهاز',
  errorTitle: 'كاين مشكل',
  copy: 'انسخ',
  copied: 'تنسخ',
  asOf: 'محسوب في',
  hideTool: 'خبّي هاد الأداة',
  translating: 'راه يترجم…',
  translateAuto: 'يكشفها وحدو',
  translateLocal: 'الترجمة تتدار هنا. النص ما يخرجش من السيرفور.',
  translateApprox: 'ترجمة آلية، تقريبية',
  translateTruncated: 'حبست الترجمة كي وصلت للحد.',
  translateFailed: 'ما نجحتش الترجمة.',
  stop: 'حبس',
  toolsNote: 'الأدوات المطفية ما تبانش فوق النتائج. هاد الإعداد يتسجل غير في هاد المتصفح.',
  enable: 'شعّل',
  disable: 'طفّي',
}

const catalogues: Record<Locale, Messages> = { ar, ary, fr, en }

export function messages(locale: Locale): Messages {
  return catalogues[locale]
}

