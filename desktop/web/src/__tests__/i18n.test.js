import { describe, it, expect, beforeEach } from 'vitest'
import { makeI18n, detectLocale, setLocale, i18n, LOCALE_STORAGE_KEY } from '../i18n/index.js'
import en from '../i18n/locales/en.js'
import zhCN from '../i18n/locales/zh-CN.js'

// Walk a dictionary into dot-separated key paths ("swells.colWave").
function keyPaths(messages, prefix = '') {
  return Object.entries(messages).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key
    return typeof value === 'string' ? [path] : keyPaths(value, path)
  })
}

const sorted = (list) => [...list].sort()

describe('i18n dictionaries', () => {
  it('has identical key sets in en and zh-CN (missing key fails)', () => {
    expect(sorted(keyPaths(zhCN))).toEqual(sorted(keyPaths(en)))
  })

  it('keeps every dictionary leaf a non-empty string', () => {
    const leaves = (dict) =>
      Object.values(dict).flatMap((v) => (typeof v === 'string' ? [v] : leaves(v)))
    for (const leaf of [...leaves(en), ...leaves(zhCN)]) {
      expect(leaf.length).toBeGreaterThan(0)
    }
  })
})

describe('locale detection and persistence', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('prefers the saved choice over the browser hint', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'zh-CN')
    expect(detectLocale()).toBe('zh-CN')
  })

  it('ignores an unknown saved value', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'klingon')
    expect(detectLocale()).not.toBe('klingon')
  })

  it('maps any Chinese browser language to zh-CN', () => {
    Object.defineProperty(window, 'navigator', {
      value: { language: 'zh-TW' },
      configurable: true,
    })
    expect(detectLocale()).toBe('zh-CN')
  })

  it('falls back to en for non-Chinese browsers', () => {
    Object.defineProperty(window, 'navigator', {
      value: { language: 'fr-FR' },
      configurable: true,
    })
    expect(detectLocale()).toBe('en')
  })

  it('setLocale persists the choice and moves the active locale', () => {
    setLocale('zh-CN')
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe('zh-CN')
    expect(i18n.global.locale.value).toBe('zh-CN')
    setLocale('en')
    expect(i18n.global.locale.value).toBe('en')
  })

  it('ignores unsupported locales', () => {
    setLocale('en')
    setLocale('klingon')
    expect(i18n.global.locale.value).toBe('en')
  })
})

describe('translations render', () => {
  it('produces the en working from before i18n for a pinned instance', () => {
    const instance = makeI18n('en')
    expect(instance.global.t('swells.heading')).toBe('Swells')
    expect(instance.global.t('swells.unwiredNote')).toContain('never fabricated')
  })

  it('interpolates counts in both locales', () => {
    const instance = makeI18n('en')
    expect(instance.global.t('sealand.activeRuns', { n: 2 })).toBe('2 active')
    instance.global.locale.value = 'zh-CN'
    expect(instance.global.t('sealand.activeRuns', { n: 2 })).toBe('2 活跃')
    expect(instance.global.t('common.secondsAgo', { n: 5 })).toBe('5 秒前')
  })
})
