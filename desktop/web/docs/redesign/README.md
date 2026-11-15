# Swells redesign — evidence shots (Waveform Status Band)

Direction contract: `docs/design/swells-redesign-contract.md` (user-locked
2026-11-15). These captures come from a real run: the controller binary
(`wm-controller-server`) serving seeded demo runs through the REST API,
the web app under `pnpm dev`, captured with headless Chromium.

| File | What it shows |
|---|---|
| `desktop-1440.png` | 1440×900 first viewport: 52px top nav, sea band with layered active swells + now cursor, legend, swell table with wave marks / state pills / honest unwired columns |
| `mobile-390.png` | 390px width: nav scrolls horizontally, legend stacks vertically, table pans, pills wrap |
| `empty-band.png` | All runs stopped: the band calms to a flat ripple with the "No active swells — launch from Spectrum" guide (no fake activity) |
| `error-controller-down.png` | Controller unreachable: named error card with Retry button, nothing fabricated |
| `error-controller-hint.png` | Same case after WM-DEV-001: the card now leads with the actionable hint (`pnpm dev:full` / the cargo command) and trails the raw named error as small detail — no naked fetch string |

Demo data note: the seeded runs (`checkout-swell`, `kb-baseline`,
`login-storm`, `soak-nightly`) exist only on the capture machine's
controller instance; the UI derives everything from the live
`/api/v1/status` payload and invents no history (RPS / p99 / errors are
shown as "—" until the metrics face lands, WM-0412).

**待用户装机亲验 — visual acceptance is open.** These PNGs prove the
states render; the visual verdict (colors, density, the band's feel at
real size) belongs to the user on a real desktop session. Visual items
do not close from screenshots alone.
