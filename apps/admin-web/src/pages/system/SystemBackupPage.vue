<script setup lang="ts">
import DatabaseBackupCard from '@/components/system/DatabaseBackupCard.vue'
import SystemPageHeader from '@/components/system/SystemPageHeader.vue'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
</script>

<template>
  <section class="system-detail backup-page" data-testid="system-backup-page">
    <SystemPageHeader number="01" :eyebrow="t('system.detail.backupEyebrow')"
      :title="t('system.detail.backupTitle')" :description="t('system.detail.backupDescription')" />
    <DatabaseBackupCard />
  </section>
</template>
