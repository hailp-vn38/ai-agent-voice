<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { onMounted, ref } from 'vue'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import AdminShell from '@/layouts/AdminShell.vue'
import { useAdminStore } from '@/stores/admin'
import { useAuthStore } from '@/stores/auth'

const admin = useAdminStore()
const auth = useAuthStore()
const { error, loading } = storeToRefs(admin)
const { t } = useI18n()

const tokenDraft = ref('')

onMounted(() => {
  if (auth.adminToken) void admin.loadAll()
})

async function connect() {
  auth.setAdminToken(tokenDraft.value)
  tokenDraft.value = ''
  await admin.loadAll()
}
</script>

<template>
  <AdminShell>
    <form
      v-if="!auth.adminToken"
      class="mb-4 flex flex-wrap items-end gap-3 rounded-lg border bg-card px-4 py-3"
      @submit.prevent="connect"
    >
      <label class="flex-1 space-y-1">
        <span class="text-sm font-medium">{{ t('auth.tokenLabel') }}</span>
        <input
          v-model="tokenDraft"
          type="password"
          autocomplete="off"
          class="admin-input w-full"
          :placeholder="t('auth.tokenPlaceholder')"
        />
      </label>
      <Button type="submit" :disabled="!tokenDraft.trim()">{{ t('auth.connect') }}</Button>
    </form>

    <div
      v-else-if="error"
      role="alert"
      class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
    >
      <span>{{ error }}</span>
      <span class="flex gap-2">
        <Button size="sm" variant="outline" :disabled="loading" @click="admin.refreshAll()">
          {{ t('common.retry') }}
        </Button>
        <Button size="sm" variant="ghost" @click="admin.clearError()">
          {{ t('common.close') }}
        </Button>
      </span>
    </div>
    <RouterView v-if="auth.adminToken" />
  </AdminShell>
</template>