<script setup lang="ts">
import { ArrowUpRight, AudioLines, Cpu, KeyRound, Network } from '@lucide/vue'
import { useId } from 'vue'
import LanguageSwitcher from '@/components/LanguageSwitcher.vue'
import ThemeToggle from '@/components/ThemeToggle.vue'
import VelaAssistantMark from '@/components/icons/VelaAssistantMark.vue'
import { useI18n } from '@/composables/useI18n'

defineProps<{ title: string; description: string }>()
const { t } = useI18n()
const id = useId().replace(/:/g, '')
const bars = Array.from({ length: 36 }, (_, i) => 7 + (Math.sin(i * .67) * .5 + .5) * 31)
</script>

<template>
  <main class="auth-page">
    <header class="auth-topbar">
      <div class="auth-wordmark"><span><VelaAssistantMark /></span><strong>Vela<span>Studio</span></strong></div>
      <div class="auth-preferences"><LanguageSwitcher /><ThemeToggle /></div>
    </header>
    <div class="auth-layout">
      <aside class="auth-story" aria-labelledby="auth-story-title">
        <div class="auth-story-grid" aria-hidden="true" />
        <p class="auth-story-kicker"><i aria-hidden="true" />{{ t('auth.studioEyebrow') }}</p>
        <h2 id="auth-story-title">{{ t('auth.studioHeadline') }}</h2>
        <p class="auth-story-caption">{{ t('auth.studioCaption') }}</p>
        <div class="auth-art" aria-hidden="true">
          <svg viewBox="0 0 420 260" fill="none" focusable="false">
            <defs><radialGradient :id="`${id}-glow`"><stop stop-color="#b5f5ff" stop-opacity=".3" /><stop offset="1" stop-color="#9c8bff" stop-opacity="0" /></radialGradient></defs>
            <ellipse cx="210" cy="128" rx="160" ry="120" :fill="`url(#${id}-glow)`" class="auth-art-glow" />
            <g stroke="#c8dcff" stroke-opacity=".18"><ellipse cx="210" cy="128" rx="151" ry="65" transform="rotate(-18 210 128)" /><ellipse cx="210" cy="128" rx="110" ry="94" transform="rotate(24 210 128)" /><path d="M65 99 Q140 155 210 128 T355 99 M95 211 Q150 170 210 128 T325 49" /></g>
            <ellipse cx="210" cy="128" rx="151" ry="65" transform="rotate(-18 210 128)" pathLength="100" class="auth-orbit" />
            <rect x="167" y="85" width="86" height="86" rx="27" fill="#d9e6ff" fill-opacity=".08" stroke="#deedff" stroke-opacity=".35" />
            <circle cx="210" cy="128" r="58" stroke="#c6f9ff" stroke-opacity=".18" stroke-dasharray="2 7" />
            <g fill="#d4e9ff"><circle cx="65" cy="99" r="3" /><circle cx="355" cy="99" r="3" /><circle cx="95" cy="211" r="3" /><circle cx="325" cy="49" r="3" /></g>
            <rect v-for="(bar, i) in bars" :key="i" :x="103 + i * 6" :y="224 - bar / 2" width="2" :height="bar" rx="1" class="auth-wave" :style="{ '--phase': `${i * -.09}s` }" />
          </svg>
          <VelaAssistantMark class="auth-art-mark" />
        </div>
        <div class="auth-story-footer"><span><AudioLines :size="13" />{{ t('auth.featureVoice') }}</span><span><Cpu :size="13" />Agent</span><span><Network :size="13" />{{ t('nav.knowledge') }}</span><ArrowUpRight :size="15" class="auth-story-arrow" /></div>
      </aside>
      <section class="auth-form-panel" aria-labelledby="auth-form-title">
        <span class="auth-access-icon" aria-hidden="true"><KeyRound :size="20" /></span>
        <p class="auth-form-kicker">{{ t('auth.adminAccess') }}</p>
        <h1 id="auth-form-title">{{ title }}</h1>
        <p class="auth-form-description">{{ description }}</p>
        <div class="auth-form-content"><slot /></div>
        <p class="auth-form-note">{{ t('auth.adminOnly') }}</p>
      </section>
    </div>
    <footer class="auth-footer"><span>Vela Studio</span><span>{{ t('auth.footer') }}</span></footer>
  </main>
</template>

<style scoped src="./auth-page-shell.css"></style>
