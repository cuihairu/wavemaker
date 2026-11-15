// Pin the locale to English for every test: the suites assert today's
// English wording verbatim (WM-I18N-001 keeps the en dictionary
// byte-identical to the pre-i18n UI). zh-CN gets its own dictionary
// parity suite instead of per-string snapshots.
localStorage.setItem('wm-locale', 'en')
