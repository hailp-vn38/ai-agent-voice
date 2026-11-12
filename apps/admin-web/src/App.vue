<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import ConnectionLostDialog from '@/components/feedback/ConnectionLostDialog.vue'
import ToastViewport from '@/components/feedback/ToastViewport.vue'
import { useErrorToast } from '@/composables/useNotify'
import AdminShell from '@/layouts/AdminShell.vue'
import { useAdminStore } from '@/stores/admin'
import { useAuthStore } from '@/stores/auth'

const admin = useAdminStore()
const auth = useAuthStore()
const route = useRoute()
const router = useRouter()
const { error } = storeToRefs(admin)

const isPublicRoute = computed(() => route.meta.public === true)

useErrorToast(() => error.value, {
  retry: () => (auth.hasAdminAccess ? admin.refreshAll() : undefined),
  onDismiss: () => admin.clearError(),
})

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
    <RouterView />
  </AdminShell>
  <ToastViewport />
  <ConnectionLostDialog />
</template>
