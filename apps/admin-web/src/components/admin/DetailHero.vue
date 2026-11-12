<script setup lang="ts">
import { ArrowLeft } from '@lucide/vue'
import HeroScene from './HeroScene.vue'

defineProps<{
  tone: 'agents' | 'devices' | 'speakers' | 'templates' | 'providers' | 'mcp' | 'artifacts' | 'gateway'
  title: string
  eyebrow?: string
  description?: string
  backLabel: string
}>()

const emit = defineEmits<{ back: [] }>()

</script>

<template>
  <header class="detail-hero" :data-tone="tone" data-detail-header>
    <div class="detail-hero-motif" aria-hidden="true" />
    <button type="button" class="detail-hero-back" data-detail-back @click="emit('back')">
      <ArrowLeft :size="15" aria-hidden="true" />{{ backLabel }}
    </button>
    <div class="detail-hero-row">
      <div class="detail-hero-identity">
        <span v-if="$slots.icon" class="detail-hero-avatar" aria-hidden="true"><slot name="icon" /></span>
        <div class="min-w-0">
          <p v-if="eyebrow" class="detail-hero-eyebrow">{{ eyebrow }}</p>
          <h1 class="break-words line-clamp-2" :title="title">{{ title }}</h1>
          <p v-if="description" class="detail-hero-description">{{ description }}</p>
          <div v-if="$slots.meta" class="detail-hero-meta"><slot name="meta" /></div>
        </div>
      </div>
      <div v-if="$slots.actions" class="detail-hero-actions" data-detail-actions><slot name="actions" /></div>
    </div>
    <HeroScene :theme="tone" />
    <div v-if="$slots.facts" class="detail-hero-facts"><slot name="facts" /></div>
  </header>
</template>

<style scoped>
.detail-hero {
  --hero-a: #4a2fc4; --hero-b: #6944e9; --hero-c: #0798b9;
  position: relative; isolation: isolate; min-width: 0; overflow: hidden; padding: 14px 22px 18px; border-radius: 22px; color: #fff;
  background:
    radial-gradient(90% 120% at 100% 0%, rgb(255 255 255 / 0.2), transparent 55%),
    linear-gradient(120deg, var(--hero-a) 0%, var(--hero-b) 42%, var(--hero-c) 100%);
  box-shadow: 0 10px 30px -12px color-mix(in oklab, var(--hero-b) 70%, transparent);
}
.detail-hero[data-tone="devices"] { --hero-a: #0a5873; --hero-b: #0d8fae; --hero-c: #5a45e0; }
.detail-hero[data-tone="speakers"] { --hero-a: #3a2ab4; --hero-b: #7a3fe0; --hero-c: #c2499f; }
.detail-hero[data-tone="templates"] { --hero-a: #2b3bb8; --hero-b: #4a66ea; --hero-c: #10a3a8; }
.detail-hero[data-tone="providers"] { --hero-a: #5a2bbd; --hero-b: #8046e6; --hero-c: #3f7fe8; }
.detail-hero[data-tone="mcp"] { --hero-a: #0c5f66; --hero-b: #0f97a0; --hero-c: #4d57e0; }
.detail-hero[data-tone="artifacts"] { --hero-a: #33309f; --hero-b: #5a5fe6; --hero-c: #9a5fe0; }
.detail-hero[data-tone="gateway"] { --hero-a: #1f2a8f; --hero-b: #3d52e0; --hero-c: #0aa2c0; }
[data-tone="gateway"] .detail-hero-motif {
  background-image: repeating-linear-gradient(0deg, rgb(255 255 255 / 0.13) 0 1px, transparent 1px 14px);
  mask-image: linear-gradient(270deg, black 0%, transparent 48%);
}

.detail-hero-motif { position: absolute; inset: 0; border-radius: inherit; opacity: 0.4; pointer-events: none; }
[data-tone="agents"] .detail-hero-motif {
  background-image: radial-gradient(rgb(230 250 255 / 0.3) 0.8px, transparent 1px);
  background-size: 28px 28px;
  mask-image: linear-gradient(90deg, transparent 25%, black);
}
[data-tone="devices"] .detail-hero-motif {
  background-image: radial-gradient(rgb(255 255 255 / 0.28) 1.4px, transparent 1.6px); background-size: 22px 22px;
  mask-image: radial-gradient(ellipse at 92% 8%, black 0%, transparent 65%);
}
[data-tone="templates"] .detail-hero-motif {
  background-image: linear-gradient(rgb(255 255 255 / 0.12) 1px, transparent 1px), linear-gradient(90deg, rgb(255 255 255 / 0.12) 1px, transparent 1px);
  background-size: 32px 32px; mask-image: linear-gradient(225deg, black 0%, transparent 58%);
}
[data-tone="speakers"] .detail-hero-motif {
  background-image: repeating-linear-gradient(90deg, rgb(255 255 255 / 0.14) 0 4px, transparent 4px 11px);
  mask-image: radial-gradient(ellipse 34% 62% at 86% 30%, black 0%, transparent 100%);
}

.detail-hero-back { position: relative; display: inline-flex; align-items: center; gap: 6px; height: 30px; margin-left: -10px; padding: 0 10px; border-radius: 999px; color: rgb(255 255 255 / 0.8); font-size: 13px; font-weight: 500; cursor: pointer; transition: background-color 160ms, color 160ms; }
.detail-hero-back:hover { background: rgb(255 255 255 / 0.14); color: #fff; }
.detail-hero-back:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
.detail-hero-row { position: relative; display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-top: 14px; }
.detail-hero-identity { display: flex; align-items: flex-start; gap: 16px; min-width: 0; }
.detail-hero-avatar { display: grid; place-items: center; flex-shrink: 0; width: 60px; height: 60px; border: 1px solid rgb(255 255 255 / 0.28); border-radius: 18px; background: rgb(255 255 255 / 0.16); font-size: 26px; font-weight: 600; backdrop-filter: blur(8px); }
.detail-hero-eyebrow { font-size: 12px; font-weight: 600; letter-spacing: 0.18em; text-transform: uppercase; color: rgb(255 255 255 / 0.72); }
.detail-hero h1 { margin-top: 4px; font-size: clamp(24px, 2.6vw, 32px); line-height: 1.2; font-weight: 600; letter-spacing: -0.02em; }
.detail-hero-description { max-width: 640px; margin-top: 6px; color: rgb(255 255 255 / 0.82); font-size: 14.5px; line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; }
.detail-hero-meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin-top: 12px; }
.detail-hero-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 8px; flex-shrink: 0; }
.detail-hero-facts { position: relative; display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 12px; margin-top: 12px; }

@media (max-width: 900px) {
  .detail-hero-row { flex-direction: column; gap: 16px; }
  .detail-hero-actions { justify-content: flex-start; }
}
@media (max-width: 560px) {
  .detail-hero { padding: 16px 20px 20px; border-radius: 20px; }
  .detail-hero-avatar { width: 48px; height: 48px; border-radius: 15px; font-size: 21px; }
  .detail-hero-facts { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
}

.detail-hero-back, .detail-hero-row, .detail-hero-facts { z-index: 2; }
</style>
