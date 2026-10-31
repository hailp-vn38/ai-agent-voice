<script setup lang="ts">
import { Database } from '@lucide/vue'
import { useRouter } from 'vue-router'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import DatabaseBackupCard from '@/components/system/DatabaseBackupCard.vue'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()
</script>

<template>
  <section class="space-y-4" data-testid="system-backup-page">
    <DetailHeader
      :title="t('system.hub.backupTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-warning/15 text-warning-foreground dark:text-warning"
        >
          <Database class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.backupSub') }}</span>
        </div>
      </template>
    </DetailHeader>

    <DatabaseBackupCard />
  </section>
</template>
