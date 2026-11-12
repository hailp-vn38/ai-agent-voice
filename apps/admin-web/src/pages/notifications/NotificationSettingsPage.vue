<script setup lang="ts">
import NotificationSettingsCard from '@/components/notifications/NotificationSettingsCard.vue'
import SystemPageHeader from '@/components/system/SystemPageHeader.vue'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
</script>

<template>
  <section class="system-detail notifications-page" data-testid="notification-settings-page">
    <SystemPageHeader number="05" :eyebrow="t('system.detail.notificationsEyebrow')"
      :title="t('system.detail.notificationsTitle')" :description="t('system.detail.notificationsDescription')" />
    <NotificationSettingsCard />
  </section>
</template>
