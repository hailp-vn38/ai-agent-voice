<script setup lang="ts">
import { Bell } from '@lucide/vue'
import { useRouter } from 'vue-router'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import NotificationSettingsCard from '@/components/notifications/NotificationSettingsCard.vue'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()
</script>

<template>
  <section class="space-y-5" data-testid="notification-settings-page">
    <DetailHeader
      :title="t('notifications.settings.title')"
      :back-label="t('system.title')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet"
        >
          <Bell class="size-5" />
        </div>
      </template>
      <template #details>
        <span class="line-clamp-2">{{ t('notifications.settings.description') }}</span>
      </template>
    </DetailHeader>

    <NotificationSettingsCard />
  </section>
</template>
