<script setup lang="ts">
import type { Component } from 'vue'
import HeroScene from './HeroScene.vue'

defineProps<{
  tone: 'agents' | 'devices' | 'speakers' | 'templates' | 'providers' | 'mcp' | 'artifacts' | 'gateway'
  eyebrow: string
  title: string
  intro: string
  /** `key` makes a stat a button that emits `select`; `attr` adds a data attribute to its value. */
  stats: { label: string; value: string | number; icon: Component; hint?: string; key?: string; attr?: string }[]
}>()
const emit = defineEmits<{ select: [key: string] }>()

/** Decorative waveform for the speakers hero. */
const waveform = [22, 38, 60, 44, 78, 96, 64, 40, 70, 88, 52, 30, 58, 82, 100, 72, 46, 34, 62, 90, 68, 42, 26, 50, 76, 56, 36, 24]

</script>

<template>
  <header class="directory-hero" :data-tone="tone">
    <div class="hero-motif" aria-hidden="true">
      <template v-if="tone === 'speakers'">
        <i v-for="(height, index) in waveform" :key="index" :style="{ height: `${height}%`, animationDelay: `${index * -90}ms` }" />
      </template>
    </div>
    <div class="hero-row">
      <div class="hero-copy">
        <p class="hero-eyebrow">{{ eyebrow }}</p>
        <h1>{{ title }}</h1>
        <p class="hero-intro">{{ intro }}</p>
      </div>
      <div class="hero-actions"><slot name="actions" /></div>
    </div>
    <HeroScene :theme="tone" />
    <dl class="hero-stats" :data-count="stats.length">
      <div
        v-for="stat in stats"
        :key="stat.label"
        :data-kpi-card="stat.key"
        :role="stat.key ? 'button' : undefined"
        :tabindex="stat.key ? 0 : undefined"
        @click="stat.key && emit('select', stat.key)"
        @keydown.enter.prevent="stat.key && emit('select', stat.key)"
        @keydown.space.prevent="stat.key && emit('select', stat.key)"
      >
        <dt><component :is="stat.icon" :size="14" aria-hidden="true" />{{ stat.label }}</dt>
        <dd v-bind="stat.attr ? { [stat.attr]: '' } : {}">{{ stat.value }}</dd>
        <small v-if="stat.hint">{{ stat.hint }}</small>
      </div>
    </dl>
  </header>
</template>

<style scoped>
.directory-hero {
  --hero-a: #4a2fc4; --hero-b: #6944e9; --hero-c: #0798b9;
  position: relative; isolation: isolate; overflow: hidden; padding: 18px 22px; border-radius: 22px; color: #fff;
  background:
    radial-gradient(90% 120% at 100% 0%, rgb(255 255 255 / 0.2), transparent 55%),
    linear-gradient(120deg, var(--hero-a) 0%, var(--hero-b) 42%, var(--hero-c) 100%);
  box-shadow: 0 10px 30px -12px color-mix(in oklab, var(--hero-b) 70%, transparent);
}
.directory-hero[data-tone="devices"] { --hero-a: #0a5873; --hero-b: #0d8fae; --hero-c: #5a45e0; }
.directory-hero[data-tone="speakers"] { --hero-a: #3a2ab4; --hero-b: #7a3fe0; --hero-c: #c2499f; }
.directory-hero[data-tone="templates"] { --hero-a: #2b3bb8; --hero-b: #4a66ea; --hero-c: #10a3a8; }
.directory-hero[data-tone="providers"] { --hero-a: #5a2bbd; --hero-b: #8046e6; --hero-c: #3f7fe8; }
.directory-hero[data-tone="mcp"] { --hero-a: #0c5f66; --hero-b: #0f97a0; --hero-c: #4d57e0; }
.directory-hero[data-tone="artifacts"] { --hero-a: #33309f; --hero-b: #5a5fe6; --hero-c: #9a5fe0; }
.directory-hero[data-tone="gateway"] { --hero-a: #1f2a8f; --hero-b: #3d52e0; --hero-c: #0aa2c0; }

.hero-motif { position: absolute; inset: 0; opacity: 0.4; pointer-events: none; }
/* Agents: orbits around an orchestrator. */
[data-tone="agents"] .hero-motif {
  background-image: radial-gradient(rgb(230 250 255 / 0.3) 0.8px, transparent 1px);
  background-size: 28px 28px;
  mask-image: linear-gradient(90deg, transparent 25%, black);
}
/* Devices: a field of connection points. */
[data-tone="devices"] .hero-motif {
  background-image: radial-gradient(rgb(255 255 255 / 0.28) 1.4px, transparent 1.6px);
  background-size: 22px 22px;
  mask-image: radial-gradient(ellipse at 92% 8%, black 0%, transparent 65%);
}
/* Templates: a blueprint grid. */
[data-tone="templates"] .hero-motif {
  background-image:
    linear-gradient(rgb(255 255 255 / 0.12) 1px, transparent 1px),
    linear-gradient(90deg, rgb(255 255 255 / 0.12) 1px, transparent 1px);
  background-size: 32px 32px;
  mask-image: linear-gradient(225deg, black 0%, transparent 58%);
}
/* Providers: a woven mesh of interchangeable parts. */
[data-tone="providers"] .hero-motif {
  background-image:
    repeating-linear-gradient(45deg, rgb(255 255 255 / 0.1) 0 1px, transparent 1px 22px),
    repeating-linear-gradient(-45deg, rgb(255 255 255 / 0.1) 0 1px, transparent 1px 22px);
  mask-image: radial-gradient(ellipse at 92% 6%, black 0%, transparent 62%);
}
/* MCP: sockets waiting for a plug. */
[data-tone="mcp"] .hero-motif {
  background-image: radial-gradient(circle, transparent 5px, rgb(255 255 255 / 0.26) 5px 6.5px, transparent 7px);
  background-size: 30px 30px;
  mask-image: radial-gradient(ellipse at 92% 8%, black 0%, transparent 64%);
}
/* Artifacts: a shelf of files. */
[data-tone="artifacts"] .hero-motif {
  background-image:
    linear-gradient(rgb(255 255 255 / 0.13) 2px, transparent 2px),
    linear-gradient(90deg, rgb(255 255 255 / 0.13) 2px, transparent 2px);
  background-size: 54px 68px;
  mask-image: linear-gradient(215deg, black 0%, transparent 55%);
}
/* Gateway: parallel lanes of traffic. */
[data-tone="gateway"] .hero-motif {
  background-image: repeating-linear-gradient(0deg, rgb(255 255 255 / 0.13) 0 1px, transparent 1px 14px);
  mask-image: linear-gradient(270deg, black 0%, transparent 48%);
}
/* Speakers: a live waveform. */
[data-tone="speakers"] .hero-motif {
  inset: 0 24px auto auto; display: flex; align-items: center; gap: 5px; width: min(46%, 420px); height: 130px;
  mask-image: linear-gradient(90deg, transparent, black 45%);
}
[data-tone="speakers"] .hero-motif i {
  flex: 1; min-width: 3px; border-radius: 99px; background: rgb(255 255 255 / 0.15);
  transform-origin: center; animation: hero-wave 2.4s ease-in-out infinite;
}
@keyframes hero-wave { 0%, 100% { transform: scaleY(0.55); } 50% { transform: scaleY(1); } }

.hero-row { position: relative; z-index: 2; display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; }
.hero-copy { min-width: 0; max-width: 640px; }
.hero-eyebrow { font-size: 12px; font-weight: 600; letter-spacing: 0.2em; text-transform: uppercase; color: rgb(255 255 255 / 0.72); }
.directory-hero h1 { margin-top: 6px; font-size: clamp(26px, 2.4vw, 32px); line-height: 1.15; font-weight: 600; letter-spacing: -0.025em; }
.hero-intro { margin-top: 6px; font-size: 14px; line-height: 1.5; color: rgb(255 255 255 / 0.82); }
.hero-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; flex-shrink: 0; }

.hero-stats { position: relative; z-index: 2; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; margin-top: 16px; }
.hero-stats[data-count="4"] { grid-template-columns: repeat(4, minmax(0, 1fr)); }
.hero-stats > div { padding: 10px 14px; border: 1px solid rgb(255 255 255 / 0.08); border-radius: 14px; background: rgb(255 255 255 / 0.1); backdrop-filter: blur(8px); }
.hero-stats dt { display: flex; align-items: center; gap: 6px; font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: rgb(255 255 255 / 0.72); }
.hero-stats dd { margin-top: 3px; font-size: 24px; line-height: 1.2; font-weight: 600; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.hero-stats small { display: block; margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; color: rgb(255 255 255 / 0.7); }
.hero-stats > [role="button"] { cursor: pointer; transition: background-color 160ms, transform 160ms; }
.hero-stats > [role="button"]:hover { background: rgb(255 255 255 / 0.2); transform: translateY(-2px); }
.hero-stats > [role="button"]:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }

@media (max-width: 900px) {
  .hero-row { flex-direction: column; gap: 16px; }
  .hero-actions { justify-content: flex-start; }
  .hero-stats[data-count="4"] { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@media (max-width: 560px) {
  .directory-hero { padding: 16px; border-radius: 18px; }
  .hero-stats { gap: 8px; }
  .hero-stats > div { padding: 10px 12px; }
  .hero-stats:not([data-count="4"]) { grid-template-columns: minmax(0, 1fr); }
  .hero-stats:not([data-count="4"]) > div { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .hero-stats:not([data-count="4"]) dd { margin-top: 0; }
  .hero-stats dt { font-size: 10px; letter-spacing: 0.04em; }
  .hero-stats dd { font-size: 22px; }
  [data-tone="speakers"] .hero-motif { opacity: 0.5; }
}
@media (prefers-reduced-motion: reduce) { [data-tone="speakers"] .hero-motif i { animation: none; } }

[data-tone="speakers"] .hero-motif { display: none; }
</style>
