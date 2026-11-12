<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import AdminShell from '@/layouts/AdminShell.vue'
import { useAdminStore } from '@/stores/admin'
import { useAuthStore } from '@/stores/auth'

const admin = useAdminStore()
const auth = useAuthStore()
const route = useRoute()
const router = useRouter()
const { error, loading } = storeToRefs(admin)
const { t } = useI18n()

const isPublicRoute = computed(() => route.meta.public === true)

// Data loads only once the router has decided where the Admin goes, and never while a password change is pending.
function canLoadData(): boolean {
  if (auth.mustChangePassword) return false
  return auth.hasAdminAccess
}

onMounted(async () => {
  await router.isReady()
  if (!isPublicRoute.value && canLoadData()) void admin.loadAll()
})

// Signing in navigates from the public login page into the app; that is when data starts loading.
watch(isPublicRoute, (isPublic, wasPublic) => {
  if (wasPublic && !isPublic && canLoadData()) void admin.loadAll()
})
</script>

<template>
  <RouterView v-if="isPublicRoute" />
  <AdminShell v-else>
    <div
      v-if="error"
      role="alert"
      class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
    >
      <span>{{ error }}</span>
      <span class="flex gap-2">
        <Button v-if="auth.hasAdminAccess" size="sm" variant="outline" :disabled="loading" @click="admin.refreshAll()">
          {{ t('common.retry') }}
        </Button>
        <Button size="sm" variant="ghost" @click="admin.clearError()">
          {{ t('common.close') }}
        </Button>
      </span>
    </div>
    <RouterView />
  </AdminShell>
</template>
