<script setup lang="ts">
import { RefreshCcw } from '@lucide/vue'
import { ref } from 'vue'

import AssistantDeviceSettings from '@/components/system/AssistantDeviceSettings.vue'
import SystemPageHeader from '@/components/system/SystemPageHeader.vue'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
const settingsRef = ref<InstanceType<typeof AssistantDeviceSettings> | null>(null)
</script>

<template>
  <section class="system-detail assistant-page" data-testid="system-assistant-page">
    <SystemPageHeader number="02" :eyebrow="t('system.detail.assistantEyebrow')"
      :title="t('system.detail.assistantTitle')" :description="t('system.detail.assistantDescription')">
      <template #actions>
        <button type="button" class="sys-button" data-testid="assistant-refresh-btn" :disabled="settingsRef?.busy" @click="settingsRef?.reload()">
          <RefreshCcw class="size-3.5" aria-hidden="true" />{{ t('system.refresh') }}
        </button>
      </template>
    </SystemPageHeader>
    <AssistantDeviceSettings ref="settingsRef" />
  </section>
</template>
