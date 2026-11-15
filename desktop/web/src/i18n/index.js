// Locale setup (WM-I18N-001). vue-i18n in composition mode with two
// dictionaries; detection order is the saved choice, then the browser
// language, then English. The chosen locale persists in localStorage and
// mirrors onto <html lang> so assistive tech follows the switch.
import { createI18n } from 'vue-i18n'
import en from './locales/en.js'
import zhCN from './locales/zh-CN.js'

export const LOCALE_STORAGE_KEY = 'wm-locale'

const messages = { en, 'zh-CN': zhCN }

// Detection: localStorage → navigator.language → en fallback.
export function detectLocale() {
  try {
    const saved =
      typeof localStorage !== 'undefined' && localStorage.getItem(LOCALE_STORAGE_KEY)
    if (saved && messages[saved]) return saved
  } catch {
    // storage unavailable — fall through to the browser hint
  }
  const browser = typeof navigator !== 'undefined' ? navigator.language : ''
  if (browser && browser.toLowerCase().startsWith('zh')) return 'zh-CN'
  return 'en'
}

function syncDocumentLang(locale) {
  if (typeof document !== 'undefined') document.documentElement.lang = locale
}

// Factory so tests can pin a locale explicitly (mounts pass one instance
// per suite); main.js uses the singleton below.
export function makeI18n(locale = detectLocale()) {
  const instance = createI18n({
    legacy: false,
    locale,
    fallbackLocale: 'en',
    messages,
  })
  syncDocumentLang(locale)
  return instance
}

export const i18n = makeI18n()

// Persist a manual choice and propagate it app-wide. Unknown locales are
// ignored rather than silently applied.
export function setLocale(locale) {
  if (!messages[locale]) return
  i18n.global.locale.value = locale
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale)
  } catch {
    // storage unavailable — the choice applies for this session only
  }
  syncDocumentLang(locale)
}
