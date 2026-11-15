<script setup>
// Sea band overview (direction contract: docs/design/swells-redesign-
// contract.md). One full-width stacked composition: every swell with
// active runs is a sea-colored stratum, its height proportional to its
// share of active runs. Inline SVG only — no chart library.
//
// Honesty boundary: the controller status payload carries no RPS history
// and no live RPS at all, so the band renders the latest known
// composition (layer weight = active run count) and the legend says so.
// Time scrubbing waits for the metrics series API (WM-0412) — nothing is
// fabricated into a fake time axis.
import { computed, ref } from 'vue'
import { stateTone } from '../api.js'

const props = defineProps({
  // Derived swell list, or null while the status is loading.
  swells: { type: Array, default: null },
})

defineEmits(['open'])

const SEA = ['var(--sea-deep)', 'var(--sea-mid)', 'var(--sea-surface)']
const hovered = ref(-1)

const activeRunCount = (swell) =>
  Object.entries(swell.states)
    .filter(([state]) => stateTone(state) === 'ok')
    .reduce((total, [, count]) => total + count, 0)

// A deterministic wavy boundary across the band width.
function waveBoundary(yBase, amp, phase, samples = 40) {
  const points = []
  for (let i = 0; i <= samples; i++) {
    const x = (i / samples) * 1000
    const y = yBase + amp * Math.sin((i / samples) * Math.PI * 4 + phase)
    points.push([x, Math.min(Math.max(y, 2), 148)])
  }
  return points
}

function areaPath(top, bottom) {
  const down = top
    .map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`)
    .join('')
  const back = bottom
    .slice()
    .reverse()
    .map(([x, y]) => `L${x.toFixed(1)},${y.toFixed(1)}`)
    .join('')
  return `${down}${back}Z`
}

const layers = computed(() => {
  if (!props.swells) return []
  const active = props.swells
    .map((swell) => ({ ...swell, active_runs: activeRunCount(swell) }))
    .filter((swell) => swell.active_runs > 0)
  const total = active.reduce((sum, swell) => sum + swell.active_runs, 0)
  let stacked = 0
  return active.map((swell, i) => {
    const share = swell.active_runs / total
    // Small rendering floor so a one-run swell stays visible; the legend
    // always carries the exact counts.
    const frac = Math.max(share, 10 / 150)
    const bottomFrac = stacked
    stacked = Math.min(stacked + frac, 0.96)
    const topFrac = stacked
    const amp = Math.min(6, 3 + share * 6)
    return {
      swell,
      color: SEA[i % 3],
      d: areaPath(
        waveBoundary(150 - topFrac * 150, amp, i * 1.7),
        waveBoundary(150 - bottomFrac * 150, amp * 0.6, i * 1.7 + 2.4),
      ),
    }
  })
})

// Calm water used by the loading ripples and the empty state.
function calmPath(offset = 0) {
  const base = 150 - 24 - offset * 11
  const points = []
  for (let i = 0; i <= 40; i++) {
    const x = (i / 40) * 1000
    const y = base + 4 * Math.sin((i / 40) * Math.PI * 6 + offset * 1.3)
    points.push(`${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`)
  }
  return `${points.join('')}L1000,150L0,150Z`
}
</script>

<template>
  <section class="sea-band" data-test="sea-band" aria-label="Active swell overview">
    <div class="sea-band-head">
      <h2>Sea state</h2>
      <span class="muted sea-band-note" data-test="band-note">
        now — latest composition; history scrubbing lands with the metrics series (WM-0412)
      </span>
    </div>

    <div v-if="swells === null" class="sea-band-canvas" data-test="band-loading" aria-busy="true">
      <svg viewBox="0 0 1000 150" preserveAspectRatio="none" aria-hidden="true">
        <path
          v-for="n in 3"
          :key="n"
          class="ripple"
          :style="{ animationDelay: `${n * 0.35}s` }"
          :d="calmPath(n)"
        />
      </svg>
    </div>

    <div
      v-else-if="layers.length === 0"
      class="sea-band-canvas sea-band-empty"
      data-test="band-empty"
    >
      <svg viewBox="0 0 1000 150" preserveAspectRatio="none" aria-hidden="true">
        <path class="calm" :d="calmPath(0)" />
      </svg>
      <p>
        No active swells — <RouterLink to="/spectrum">launch from Spectrum</RouterLink>
      </p>
    </div>

    <template v-else>
      <div class="sea-band-canvas">
        <svg
          viewBox="0 0 1000 150"
          preserveAspectRatio="none"
          role="img"
          aria-label="Stacked composition of active swells"
        >
          <path
            v-for="(layer, i) in layers"
            :key="layer.swell.swell_id"
            class="sea-layer"
            :class="{ 'is-hovered': hovered === i }"
            :d="layer.d"
            :fill="layer.color"
            @mouseenter="hovered = i"
            @mouseleave="hovered = -1"
            @click="$emit('open', layer.swell.swell_id)"
          />
          <line class="now-cursor" x1="994" y1="6" x2="994" y2="144" />
        </svg>
        <span class="now-tag">now</span>
      </div>
      <ul class="sea-legend" data-test="band-legend">
        <li
          v-for="(layer, i) in layers"
          :key="layer.swell.swell_id"
          data-test="band-legend-item"
          :class="{ 'is-hovered': hovered === i }"
          @mouseenter="hovered = i"
          @mouseleave="hovered = -1"
        >
          <RouterLink :to="`/swell/${encodeURIComponent(layer.swell.swell_id)}`">
            <span class="chip" :style="{ background: layer.color }" />
            {{ layer.swell.swell_id }}
          </RouterLink>
          <span class="num">{{ layer.swell.active_runs }} active</span>
          <span class="unwired" title="no live RPS series yet (WM-0412)">— RPS</span>
        </li>
      </ul>
    </template>
  </section>
</template>

<style scoped>
.sea-band-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.sea-band-head h2 {
  margin: 18px 0 8px;
}

.sea-band-note {
  font-size: 12.5px;
}

.sea-band-canvas {
  position: relative;
  overflow: hidden;
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
}

.sea-band-canvas svg {
  display: block;
  width: 100%;
  height: 150px;
}

.sea-layer {
  cursor: pointer;
}

.sea-layer.is-hovered {
  filter: brightness(1.4);
}

.calm {
  fill: rgba(27, 127, 158, 0.25);
}

.ripple {
  fill: rgba(27, 127, 158, 0.3);
  animation: ripple 2.2s ease-in-out infinite;
}

@keyframes ripple {
  0%,
  100% {
    transform: translateY(0);
    opacity: 0.55;
  }
  50% {
    transform: translateY(-4px);
    opacity: 0.95;
  }
}

.sea-band-empty p {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  color: var(--ink-dim);
}

.now-cursor {
  stroke: var(--accent);
  stroke-width: 2;
}

.now-tag {
  position: absolute;
  top: 8px;
  right: 10px;
  color: var(--accent);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.06em;
}

.sea-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 18px;
  margin: 0;
  padding: 10px 2px 0;
  list-style: none;
  font-size: 13px;
  color: var(--ink-dim);
}

.sea-legend a {
  color: var(--ink);
}

.sea-legend li.is-hovered {
  color: var(--ink);
}

.chip {
  display: inline-block;
  width: 10px;
  height: 10px;
  border-radius: 3px;
  margin-right: 6px;
}
</style>
