<script setup lang="ts">
import { ArrowLeft, ChevronRight } from '@lucide/vue'
import { useRouter } from 'vue-router'

import { useI18n } from '@/composables/useI18n'

defineProps<{ number: string; eyebrow: string; title: string; description: string }>()
const router = useRouter()
const { t } = useI18n()
</script>

<template>
  <header class="sys-page-header" data-detail-header>
    <nav class="sys-breadcrumb" :aria-label="t('system.detail.back')">
      <button type="button" data-detail-back @click="router.push({ name: 'system' })">
        <ArrowLeft class="size-3.5" aria-hidden="true" />{{ t('system.detail.back') }}
      </button>
      <ChevronRight class="size-3" aria-hidden="true" />
      <span>{{ eyebrow }}</span>
    </nav>
    <div class="sys-title-row">
      <div>
        <p class="sys-kicker"><span>{{ number }}</span> / {{ eyebrow }}</p>
        <h1>{{ title }}</h1>
        <p class="sys-page-description">{{ description }}</p>
      </div>
      <div v-if="$slots.actions" class="sys-header-actions"><slot name="actions" /></div>
    </div>
  </header>
</template>

<style src="./system-detail.css"></style>
