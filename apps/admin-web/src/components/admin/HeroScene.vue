<script setup lang="ts">
import AgentHeroScene from './AgentHeroScene.vue'

/** Ambient illustrations, independent of API state: these are decoration, not telemetry. */
export type HeroTheme = 'agents' | 'providers' | 'jobs' | 'devices' | 'speakers' | 'templates' | 'mcp' | 'artifacts' | 'gateway' | 'system'
defineProps<{ theme: HeroTheme; surface?: boolean }>()

const providerRoutes = [
  'M60 32 H150 Q180 32 180 62 V104 Q180 130 210 130 H300',
  'M30 100 H122 Q142 100 142 116 Q142 130 165 130 H300',
  'M44 192 H160 Q190 192 190 162 Q190 130 220 130 H300',
  'M110 250 H216 Q246 250 246 220 V160 Q246 130 276 130 H300',
  'M300 130 H360 Q390 130 390 100 V64 Q390 40 420 40 H570',
  'M300 130 H590',
  'M300 130 H376 Q408 130 408 160 V190 Q408 220 438 220 H570',
]
const terminals = [{ x: 60, y: 32 }, { x: 30, y: 100 }, { x: 44, y: 192 }, { x: 110, y: 250 }, { x: 570, y: 40 }, { x: 590, y: 130 }, { x: 570, y: 220 }]
const jobLanes = [58, 130, 202]
const jobSteps = [115, 265, 415, 565]
const wave = Array.from({ length: 52 }, (_, i) => 10 + (Math.sin(i * 0.61) * 0.5 + 0.5) * 64)
const constellation = Array.from({ length: 12 }, (_, i) => ({ x: 70 + i * 45, y: 130 + Math.sin(i * 1.7) * 38 }))
</script>

<template>
  <div class="hero-scene" :class="[`hero-scene--${theme}`, { 'hero-scene--surface': surface }]" aria-hidden="true">
    <div class="scene-aura" />
    <AgentHeroScene v-if="theme === 'agents'" class="scene-drawing" />
    <svg v-else class="scene-drawing" viewBox="0 0 640 280" fill="none" focusable="false">
      <template v-if="theme === 'providers'">
        <!-- Each signal follows its own trace through the switching fabric. -->
        <g v-for="(route, i) in providerRoutes" :key="route" :style="{ '--phase': `${i * -0.85}s` }">
          <path :d="route" class="provider-trace" />
          <path :d="route" pathLength="100" class="provider-signal" />
        </g>
        <g v-for="(point, i) in terminals" :key="i" :style="{ '--phase': `${i * -0.85}s` }">
          <circle :cx="point.x" :cy="point.y" r="11" class="provider-socket" />
          <circle :cx="point.x" :cy="point.y" r="3" class="provider-terminal" />
        </g>
        <g class="provider-core">
          <circle cx="300" cy="130" r="52" class="provider-halo" />
          <rect x="274" y="104" width="52" height="52" rx="15" class="provider-chip" />
          <path d="m300 115 14 8v15l-14 8-14-8v-15zm-14 8 14 8 14-8m-14 8v15" class="provider-glyph" />
        </g>
        <g class="scene-specks">
          <circle cx="238" cy="42" r="2" /><circle cx="466" cy="90" r="2" />
          <circle cx="340" cy="234" r="2" /><circle cx="524" cy="176" r="1.5" />
        </g>
      </template>
      <template v-else-if="theme === 'jobs'">
        <!-- A staggered sequence of task cards travelling through execution lanes. -->
        <g v-for="(y, lane) in jobLanes" :key="y" :style="{ '--phase': `${lane * -2.4}s` }">
          <path :d="`M20 ${y} H620`" class="job-track" />
          <path :d="`M20 ${y} H620`" pathLength="100" class="job-flow" />
          <g v-for="(x, step) in jobSteps" :key="x" :style="{ '--step-phase': `${lane * -2.4 - step * 1.6}s` }">
            <rect :x="x - 18" :y="y - 18" width="36" height="36" rx="11" class="job-station" />
            <circle :cx="x" :cy="y" r="23" class="job-station-pulse" />
            <path v-if="step === 3" :d="`m${x - 6} ${y} 4 4 8-8`" class="job-check" />
            <g v-else class="job-station-mark">
              <path :d="`M${x - 5} ${y - 4} h10 M${x - 5} ${y + 1} h7 M${x - 5} ${y + 6} h4`" />
            </g>
          </g>
          <g class="job-packet">
            <rect x="-12" :y="y - 10" width="25" height="20" rx="6" />
            <path :d="`M-5 ${y - 3} h11 M-5 ${y + 3} h6`" />
          </g>
        </g>
        <path d="M115 20 V242 M265 20 V242 M415 20 V242 M565 20 V242" class="job-columns" />
      </template>
      <template v-else-if="theme === 'devices'">
        <path d="M40 130 H600 M320 30 V230" class="ambient-guide" />
        <circle v-for="r in [30, 60, 90, 120]" :key="r" cx="320" cy="130" :r="r" class="ambient-line" />
        <g class="radar-sweep"><path d="M320 130 440 130 A120 120 0 0 0 405 45Z" class="ambient-fill" /><path d="M320 130 H440" class="ambient-bright" /></g>
        <g v-for="(p, i) in constellation.filter((_, i) => i % 2 === 0)" :key="i" :style="{ '--phase': `${i * -0.8}s` }">
          <circle :cx="p.x" :cy="p.y" r="4" class="ambient-dot" /><circle :cx="p.x" :cy="p.y" r="13" class="connection-ring" />
        </g>
      </template>
      <template v-else-if="theme === 'speakers'">
        <path d="M30 130 H610" class="ambient-guide" />
        <rect v-for="(height, i) in wave" :key="i" :x="38 + i * 11" :y="130 - height / 2" width="4" :height="height" rx="2" class="voice-bar" :style="{ '--phase': `${i * -0.12}s` }" />
        <ellipse cx="320" cy="130" rx="280" ry="51" class="ambient-guide" />
      </template>
      <template v-else-if="theme === 'templates'">
        <path d="M40 90 H600 M40 170 H600 M100 65 V195 M210 65 V195 M320 65 V195 M430 65 V195 M540 65 V195" class="ambient-guide" />
        <path d="M100 130 H540" class="template-link" />
        <g v-for="(x, i) in [100, 210, 320, 430, 540]" :key="x" class="blueprint-module" :style="{ '--phase': `${i * -0.7}s` }">
          <rect :x="x - 27" y="103" width="54" height="54" rx="8" class="ambient-panel" />
          <path :d="`M${x - 12} 119 h24 M${x - 12} 130 h16 M${x - 12} 141 h20`" class="ambient-bright" />
          <circle :cx="x + 27" cy="130" r="3" class="ambient-dot" />
        </g>
      </template>
      <template v-else-if="theme === 'mcp'">
        <g v-for="(y, i) in [92, 130, 168]" :key="y" :style="{ '--phase': `${i * -1.1}s` }">
          <path :d="`M45 ${y} H265 M375 ${y} H595`" class="ambient-line" />
          <path :d="`M45 ${y} H265 M375 ${y} H595`" pathLength="100" class="provider-signal" />
          <rect x="268" :y="y - 10" width="34" height="20" rx="5" class="ambient-panel" />
          <g class="socket-plug"><rect x="320" :y="y - 9" width="28" height="18" rx="4" class="ambient-panel" /><path :d="`M314 ${y - 4} h6 M314 ${y + 4} h6`" class="ambient-bright" /></g>
          <circle cx="370" :cy="y" r="3" class="ambient-dot" />
        </g>
      </template>
      <template v-else-if="theme === 'artifacts'">
        <path d="M35 176 H605" class="ambient-line" />
        <g v-for="(x, i) in [100, 210, 320, 430, 540]" :key="x" class="artifact-sheet" :style="{ '--phase': `${i * -1.2}s` }">
          <path :d="`M${x - 22} 166 V96 h30 l14 14 v56 Z M${x + 8} 96 v14 h14`" class="ambient-panel" />
          <path :d="`M${x - 11} 127 h22 M${x - 11} 138 h16 M${x - 11} 149 h20`" class="ambient-bright" />
          <circle :cx="x + 20" cy="164" r="8" class="ambient-panel" /><path :d="`m${x + 16} 164 3 3 5-6`" class="job-check" />
        </g>
      </template>
      <template v-else-if="theme === 'gateway'">
        <g v-for="(y, i) in [90, 116, 144, 170]" :key="y" :class="{ 'gateway-return': i % 2 }" :style="{ '--phase': `${i * -1.3}s` }">
          <path :d="`M25 ${y} H615`" class="ambient-line" />
          <path :d="`M25 ${y} H615`" pathLength="100" class="provider-signal" />
        </g>
        <path d="m300 83 20-12 20 12 v94 l-20 12-20-12Z" class="ambient-panel" /><path d="m314 119 10 11-10 11" class="ambient-bright" />
      </template>
      <template v-else-if="theme === 'system'">
        <g v-for="(x, i) in [130, 320, 510]" :key="x" :style="{ '--phase': `${i * -1.3}s` }">
          <rect :x="x - 52" y="94" width="104" height="72" rx="12" class="ambient-panel" />
          <path :d="`M${x - 38} 132 h18 l8-14 12 29 10-20 9 5 h18`" pathLength="100" class="system-pulse" />
          <circle :cx="x + 37" cy="107" r="3" class="ambient-dot" />
        </g>
        <path d="M182 130 H268 M372 130 H458" class="template-link" />
      </template>
    </svg>
  </div>
</template>

<style>
/* Neutral headers use the same overlay without adding a separate visual strip. */
.hero-surface { position: relative; isolation: isolate; overflow: hidden; border-radius: 14px; }
.hero-surface > :not(.hero-scene) { position: relative; z-index: 2; }
</style>

<style scoped>
.hero-scene {
  position: absolute;
  inset: 0;
  z-index: 1;
  border-radius: inherit;
  overflow: hidden;
  pointer-events: none;
  mask-image: linear-gradient(90deg, transparent 12%, rgb(0 0 0 / 0.2) 35%, #000 65%);
}
.scene-drawing {
  position: absolute; right: -2%; top: 50%;
  width: min(74%, 820px); height: auto; aspect-ratio: 640 / 280;
  transform: translateY(-50%); opacity: 0.56;
}
.hero-scene--agents .scene-drawing { width: min(82%, 940px); height: calc(100% + 32px); aspect-ratio: auto; right: -1%; opacity: 0.78; }
.hero-scene--agents .scene-aura { background: radial-gradient(ellipse at 78% 42%, rgb(176 255 229 / 0.18), transparent 55%); }
.hero-scene--surface .scene-drawing { opacity: 0.2; }
.hero-scene--surface .scene-aura { background: radial-gradient(ellipse at 85% 35%, color-mix(in oklab, var(--primary) 12%, transparent), transparent 65%); }
.hero-scene--surface :is(.ambient-line, .ambient-guide, .provider-trace, .job-track, .job-columns) { stroke: var(--primary); }
.hero-scene--surface :is(.ambient-bright, .ambient-panel, .provider-signal, .job-flow, .job-station, .job-station-mark, .job-check, .system-pulse) { stroke: var(--primary); }
.hero-scene--surface :is(.ambient-dot, .voice-bar, .provider-terminal) { fill: var(--primary); }
.scene-aura {
  position: absolute; inset: -20%;
  background: radial-gradient(ellipse at 85% 35%, rgb(175 240 255 / 0.16), transparent 58%);
  animation: scene-breathe 12s ease-in-out infinite alternate;
}
.provider-trace { stroke: rgb(255 255 255 / 0.18); stroke-width: 1; }
.provider-signal {
  stroke: #c0f4ff; stroke-width: 2; stroke-linecap: round;
  stroke-dasharray: 6 94; stroke-dashoffset: 100;
  animation: provider-route 5.8s linear var(--phase) infinite;
}
.provider-socket { fill: rgb(255 255 255 / 0.05); stroke: rgb(255 255 255 / 0.3); }
.provider-terminal { fill: #d6f8ff; animation: provider-connect 5.8s ease-in-out var(--phase) infinite; }
.provider-halo {
  stroke: rgb(196 244 255 / 0.3); stroke-width: 1; stroke-dasharray: 2 9;
  transform-origin: 300px 130px; animation: provider-orbit 48s linear infinite;
}
.provider-chip { fill: rgb(202 220 255 / 0.16); stroke: rgb(221 250 255 / 0.6); stroke-width: 1.5; }
.provider-glyph { stroke: #e3fbff; stroke-width: 1.5; stroke-linejoin: round; }
.scene-specks { fill: rgb(224 251 255 / 0.6); animation: scene-drift 7s ease-in-out infinite alternate; }
.hero-scene--jobs .scene-aura { background: radial-gradient(ellipse at 65% 45%, rgb(112 255 217 / 0.2), transparent 52%); }
.job-track { stroke: rgb(255 255 255 / 0.14); stroke-width: 1; }
.job-flow {
  stroke: rgb(177 255 232 / 0.5); stroke-width: 1; stroke-dasharray: 1 3;
  animation: job-flow 16s linear var(--phase) infinite;
}
.job-station { fill: rgb(192 218 255 / 0.08); stroke: rgb(232 248 255 / 0.3); }
.job-station-mark { stroke: rgb(232 248 255 / 0.65); stroke-width: 1.5; stroke-linecap: round; }
.job-check { stroke: #a7ffdc; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
.job-station-pulse {
  stroke: #b5ffe7; stroke-width: 1; opacity: 0;
  transform-box: fill-box; transform-origin: center;
  animation: job-complete 8s ease-out var(--step-phase) infinite;
}
.job-packet { animation: job-dispatch 8s linear var(--phase) infinite; }
.job-packet rect { fill: #c9fff0; stroke: rgb(255 255 255 / 0.9); }
.job-packet path { stroke: #257e94; stroke-width: 1.5; stroke-linecap: round; }
.job-columns { stroke: rgb(255 255 255 / 0.06); stroke-dasharray: 2 8; }
@keyframes provider-route { to { stroke-dashoffset: 0; } }
@keyframes provider-connect { 0%, 100% { opacity: 0.35; } 50% { opacity: 1; } }
@keyframes provider-orbit { to { transform: rotate(360deg); } }
@keyframes scene-breathe { to { transform: translate(-4%, 5%) scale(1.12); opacity: 0.6; } }
@keyframes scene-drift { to { transform: translateY(-9px); opacity: 0.35; } }
@keyframes job-flow { to { stroke-dashoffset: -100; } }
@keyframes job-dispatch {
  0% { transform: translateX(20px); opacity: 0; }
  8%, 88% { opacity: 0.85; }
  100% { transform: translateX(620px); opacity: 0; }
}
@keyframes job-complete {
  0%, 28%, 100% { opacity: 0; transform: scale(0.8); }
  8% { opacity: 0.5; }
  26% { opacity: 0; transform: scale(1.5); }
}
@media (max-width: 900px) {
  .hero-scene--agents .scene-drawing { width: 88%; right: -7%; opacity: 0.62; }
  .scene-drawing { width: 76%; right: -8%; opacity: 0.4; }
  .hero-scene--surface .scene-drawing { opacity: 0.14; }
}
@media (max-width: 560px) {
  .hero-scene--agents .scene-drawing { width: 560px; right: -215px; opacity: 0.42; }
  .scene-drawing { width: 500px; max-width: none; right: -210px; opacity: 0.28; }
  .hero-scene--surface .scene-drawing { opacity: 0.1; }
}
.ambient-line { stroke: rgb(226 249 255 / 0.3); stroke-width: 1; }
.ambient-guide { stroke: rgb(226 249 255 / 0.12); stroke-width: 1; stroke-dasharray: 3 6; }
.ambient-bright { stroke: #d4f7ff; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.ambient-fill { fill: rgb(165 255 229 / 0.13); }
.ambient-dot { fill: #c2fff0; animation: provider-connect 4s ease-in-out var(--phase, 0s) infinite; }
.ambient-panel { fill: rgb(189 223 255 / 0.1); stroke: rgb(219 250 255 / 0.6); stroke-width: 1.2; stroke-linejoin: round; }
.connection-ring { stroke: rgb(195 255 237 / 0.5); transform-box: fill-box; transform-origin: center; animation: job-complete 5s ease-out var(--phase) infinite; }
.radar-sweep { transform-origin: 320px 130px; animation: provider-orbit 12s linear infinite; }
.voice-bar { fill: #e2caff; transform-box: fill-box; transform-origin: center; animation: voice-wave 2.6s ease-in-out var(--phase) infinite; }
.template-link { stroke: #c3fff0; stroke-dasharray: 4 8; animation: template-build 3s linear infinite; }
.blueprint-module { animation: module-build 5s ease-in-out var(--phase) infinite; }
.socket-plug { animation: socket-connect 5s ease-in-out var(--phase) infinite; }
.artifact-sheet { animation: sheet-float 6s ease-in-out var(--phase) infinite; }
.gateway-return .provider-signal { animation-direction: reverse; stroke: #e9caff; }
.system-pulse { stroke: #b6ffdf; stroke-width: 1.8; stroke-dasharray: 35 65; animation: provider-route 4s linear var(--phase) infinite; }
@keyframes voice-wave { 0%, 100% { transform: scaleY(0.4); opacity: 0.4; } 50% { transform: scaleY(1); opacity: 0.9; } }
@keyframes template-build { to { stroke-dashoffset: -36; } }
@keyframes module-build { 0%, 100% { opacity: 0.4; } 50% { opacity: 1; } }
@keyframes socket-connect { 0%, 100% { transform: translateX(10px); opacity: 0.5; } 35%, 65% { transform: translateX(-8px); opacity: 1; } }
@keyframes sheet-float { 0%, 100% { transform: translateY(4px); } 50% { transform: translateY(-8px); } }
@media (prefers-reduced-motion: reduce) {
  .hero-scene *, .hero-scene *::before, .hero-scene *::after { animation: none !important; }
  .provider-signal { stroke-dashoffset: 20; opacity: 0.5; }
  .job-packet { opacity: 0; }
}
</style>
