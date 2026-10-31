<script setup lang="ts">
import { Bot, RefreshCcw } from '@lucide/vue'
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import AssistantDeviceSettings from '@/components/system/AssistantDeviceSettings.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()
const settingsRef = ref<InstanceType<typeof AssistantDeviceSettings> | null>(null)

function refresh() {
  void settingsRef.value?.reload()
}
</script>

<template>
  <section class="space-y-4" data-testid="system-assistant-page">
    <DetailHeader
      :title="t('system.hub.assistantTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
        >
          <Bot class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.assistantSub') }}</span>
        </div>
      </template>
      <template #actions>
        <Button variant="outline" size="sm" data-testid="assistant-refresh-btn" @click="refresh">
          <RefreshCcw class="size-4" aria-hidden="true" />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <AssistantDeviceSettings ref="settingsRef" />
  </section>
</template>
