<script setup lang="ts">
import { Search } from '@lucide/vue'
import { computed, ref } from 'vue'
import { RouterLink } from 'vue-router'

import PageHeader from '@/components/admin/PageHeader.vue'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'

const store = useAdminStore()
const { t } = useI18n()
const search = ref('')
const devices = computed(() => {
  const needle = search.value.trim().toLowerCase()
  return store.devices.filter((device) =>
    !needle || [device.name, device.deviceId, device.agentId].some((part) => part.toLowerCase().includes(needle)),
  )
})
</script>

<template>
  <section class="space-y-6">
    <PageHeader :eyebrow="t('devices.eyebrow')" :title="t('devices.title')" :description="t('devices.description')" />
    <div class="studio-panel p-4">
      <label class="relative block max-w-md">
        <Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
        <span class="sr-only">{{ t('devices.search') }}</span>
        <input v-model="search" class="admin-input pl-9" type="search" :placeholder="t('devices.search')" />
      </label>
    </div>
    <div class="studio-panel overflow-x-auto">
      <table class="w-full min-w-[660px] text-left text-sm">
        <thead class="border-b bg-surface text-xs text-muted-foreground">
          <tr>
            <th class="px-5 py-4">{{ t('devices.device') }}</th>
            <th class="px-5 py-4">{{ t('nav.agents') }}</th>
            <th class="px-5 py-4">{{ t('nav.templates') }}</th>
            <th class="px-5 py-4">{{ t('devices.admission') }}</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-border/70">
          <tr v-for="device in devices" :key="device.id" class="hover:bg-surface">
            <td class="px-5 py-4">
              <p class="font-medium">{{ device.name }}</p>
              <p class="mt-1 font-mono text-xs text-muted-foreground">{{ device.deviceId }}</p>
            </td>
            <td class="px-5 py-4">
              <RouterLink class="font-medium hover:underline" :to="`/agents/${encodeURIComponent(device.agentId)}`">
                {{ store.getAgent(device.agentId)?.name ?? device.agentId }}
              </RouterLink>
            </td>
            <td class="px-5 py-4 text-muted-foreground">{{ store.getEffectiveDeviceTemplateById(device.id)?.name ?? t('common.none') }}</td>
            <td class="px-5 py-4">
              <span :class="device.status === 'online' ? 'text-success' : 'text-muted-foreground'">
                {{ device.status === 'online' ? t('devices.enabled') : t('devices.disabled') }}
              </span>
            </td>
          </tr>
          <tr v-if="!devices.length">
            <td colspan="4" class="px-5 py-12 text-center text-muted-foreground">{{ t('devices.empty') }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p class="text-xs text-muted-foreground">{{ t('devices.note') }}</p>
  </section>
</template>
