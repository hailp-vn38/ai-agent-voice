<script setup lang="ts">
import { useId } from 'vue'

const id = useId().replace(/:/g, '')
const satellites = [
  { x: 220, y: 54, route: 'M370 136 C318 136 303 54 220 54' },
  { x: 489, y: 42, route: 'M370 136 C433 136 424 42 489 42' },
  { x: 568, y: 130, route: 'M370 136 C440 100 500 100 568 130' },
  { x: 480, y: 235, route: 'M370 136 C424 136 414 235 480 235' },
  { x: 232, y: 218, route: 'M370 136 C308 136 303 218 232 218' },
  { x: 155, y: 132, route: 'M370 136 C290 166 225 98 155 132' },
]
const particles = Array.from({ length: 22 }, (_, i) => ({
  x: 84 + ((i * 83) % 530), y: 18 + ((i * 47) % 248), r: i % 4 === 0 ? 1.8 : 0.8,
}))
</script>

<template>
  <svg viewBox="0 0 640 280" fill="none" focusable="false" aria-hidden="true">
    <defs>
      <radialGradient :id="`${id}-aura`">
        <stop stop-color="#c9fff4" stop-opacity="0.44" />
        <stop offset="0.3" stop-color="#8be8ff" stop-opacity="0.16" />
        <stop offset="1" stop-color="#bda5ff" stop-opacity="0" />
      </radialGradient>
      <linearGradient :id="`${id}-orbit`" x1="160" y1="40" x2="570" y2="240" gradientUnits="userSpaceOnUse">
        <stop stop-color="#e6c9ff" stop-opacity="0.1" />
        <stop offset="0.5" stop-color="#d3fff6" stop-opacity="0.6" />
        <stop offset="1" stop-color="#a4ceff" stop-opacity="0.14" />
      </linearGradient>
      <linearGradient :id="`${id}-core`" x1="344" y1="102" x2="399" y2="168" gradientUnits="userSpaceOnUse">
        <stop stop-color="#f4e6ff" /><stop offset="1" stop-color="#a3fff0" />
      </linearGradient>
    </defs>

    <ellipse cx="370" cy="136" rx="215" ry="150" :fill="`url(#${id}-aura)`" class="agent-aura" />
    <g class="agent-dust">
      <circle v-for="(p, i) in particles" :key="i" :cx="p.x" :cy="p.y" :r="p.r" :style="{ '--phase': `${i * -0.63}s` }" />
    </g>

    <!-- Tilted orbital planes give the network depth without a canvas or timers. -->
    <g :stroke="`url(#${id}-orbit)`" class="agent-orbital-planes">
      <ellipse cx="370" cy="136" rx="209" ry="82" transform="rotate(-17 370 136)" />
      <ellipse cx="370" cy="136" rx="165" ry="113" transform="rotate(22 370 136)" />
      <ellipse cx="370" cy="136" rx="107" ry="66" transform="rotate(-17 370 136)" stroke-dasharray="1 7" />
    </g>
    <g class="agent-orbit-comets">
      <ellipse cx="370" cy="136" rx="209" ry="82" transform="rotate(-17 370 136)" pathLength="100" />
      <ellipse cx="370" cy="136" rx="165" ry="113" transform="rotate(22 370 136)" pathLength="100" />
    </g>

    <g v-for="(node, i) in satellites" :key="i" :style="{ '--phase': `${i * -1.3}s` }">
      <path :d="node.route" class="agent-link" />
      <path :d="node.route" pathLength="100" class="agent-signal" />
      <path :d="node.route" pathLength="100" class="agent-signal agent-signal-return" />
      <g class="agent-satellite" :style="{ 'transform-origin': `${node.x}px ${node.y}px` }">
        <circle :cx="node.x" :cy="node.y" r="24" class="agent-node-halo" />
        <rect :x="node.x - 13" :y="node.y - 13" width="26" height="26" rx="9" class="agent-node" />
        <path :d="`M${node.x - 5} ${node.y + 4} q5 4 10 0 M${node.x} ${node.y - 9} v-4`" class="agent-node-mark" />
        <circle :cx="node.x - 4" :cy="node.y - 2" r="1.4" class="agent-eye" />
        <circle :cx="node.x + 4" :cy="node.y - 2" r="1.4" class="agent-eye" />
        <circle :cx="node.x + 12" :cy="node.y - 11" r="2.5" class="agent-presence" />
      </g>
    </g>

    <g class="agent-hub">
      <circle cx="370" cy="136" r="39" class="agent-hub-ring" />
      <circle cx="370" cy="136" r="48" class="agent-hub-pulse" />
      <circle cx="370" cy="136" r="31" class="agent-hub-glass" />
      <g :stroke="`url(#${id}-core)`" class="agent-core">
        <path d="M370 113 Q373 131 393 136 Q373 141 370 159 Q367 141 347 136 Q367 131 370 113Z" />
        <path d="m354 120 32 32m0-32-32 32" class="agent-core-cross" />
      </g>
      <circle cx="370" cy="136" r="4" fill="#e6fff8" />
      <circle cx="370" cy="136" r="10" :fill="`url(#${id}-aura)`" />
    </g>
  </svg>
</template>

<style scoped>
.agent-aura { transform-origin: 370px 136px; animation: agent-aura 10s ease-in-out infinite alternate; }
.agent-orbital-planes { stroke-width: 0.75; }
.agent-orbit-comets { stroke: #d3fff4; stroke-width: 1.6; stroke-linecap: round; }
.agent-orbit-comets ellipse { stroke-dasharray: 2 98; animation: agent-orbit 24s linear infinite; }
.agent-orbit-comets ellipse:nth-child(2) { stroke: #e8ceff; animation-duration: 32s; animation-direction: reverse; animation-delay: -12s; }
.agent-link { stroke: rgb(224 243 255 / 0.19); stroke-width: 0.8; }
.agent-signal { stroke: #b8ffed; stroke-width: 1.5; stroke-linecap: round; stroke-dasharray: 4 96; stroke-dashoffset: 100; animation: agent-exchange 7.8s linear var(--phase) infinite; }
.agent-signal-return { stroke: #efcaff; stroke-width: 1; stroke-dasharray: 2 98; animation-direction: reverse; animation-delay: calc(var(--phase) - 3.9s); opacity: 0.65; }
.agent-node-halo { fill: rgb(181 247 255 / 0.04); stroke: rgb(202 243 255 / 0.12); stroke-width: 0.6; }
.agent-node { fill: rgb(200 222 255 / 0.13); stroke: rgb(226 250 255 / 0.64); stroke-width: 0.9; }
.agent-node-mark { stroke: #daf7ff; stroke-width: 1; stroke-linecap: round; }
.agent-eye { fill: #e0fff8; }
.agent-presence { fill: #acffe3; animation: agent-presence 7.8s ease-in-out var(--phase) infinite; }
.agent-satellite { animation: agent-satellite 9s ease-in-out var(--phase) infinite; }
.agent-hub-ring { stroke: rgb(230 250 255 / 0.4); stroke-width: 0.8; stroke-dasharray: 12 5 2 5; transform-origin: 370px 136px; animation: agent-hub-turn 40s linear infinite; }
.agent-hub-pulse { stroke: #bcfff0; stroke-width: 0.7; transform-origin: 370px 136px; animation: agent-hub-pulse 7.8s ease-out infinite; }
.agent-hub-glass { fill: rgb(210 234 255 / 0.09); stroke: rgb(232 254 255 / 0.35); stroke-width: 0.75; }
.agent-core { stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
.agent-core-cross { opacity: 0.4; stroke-width: 0.7; }
.agent-dust circle { fill: #e4faff; animation: agent-dust 7s ease-in-out var(--phase) infinite; }
@keyframes agent-exchange { to { stroke-dashoffset: 0; } }
@keyframes agent-orbit { from { stroke-dashoffset: 0; } to { stroke-dashoffset: -100; } }
@keyframes agent-hub-turn { to { transform: rotate(360deg); } }
@keyframes agent-aura { to { transform: scale(1.15, 0.9); opacity: 0.6; } }
@keyframes agent-presence { 0%, 100% { opacity: 0.35; } 50% { opacity: 1; } }
@keyframes agent-satellite { 0%, 100% { transform: scale(0.96); } 50% { transform: scale(1.04); } }
@keyframes agent-hub-pulse { 0% { transform: scale(0.8); opacity: 0; } 18% { opacity: 0.4; } 85%, 100% { transform: scale(1.65); opacity: 0; } }
@keyframes agent-dust { 0%, 100% { opacity: 0.12; } 50% { opacity: 0.6; } }
@media (prefers-reduced-motion: reduce) {
  * { animation: none !important; }
  .agent-hub-pulse { opacity: 0.12; }
  .agent-signal { stroke-dashoffset: 35; }
}
</style>
