<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import { signOut } from '@/api/session'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { safeTarget } from '@/router/auth-guard'
import { useAuthStore } from '@/stores/auth'

const { t } = useI18n()
const auth = useAuthStore()
const route = useRoute()
const router = useRouter()

const currentPassword = ref('')
const newPassword = ref('')
const pending = ref(false)
const error = ref<string | null>(null)

const redirect = computed(() => safeTarget(route.query.redirect))

const KNOWN_ERRORS: Record<string, MessageKey> = {
  password_too_short: 'auth.errorPasswordTooShort',
  password_is_default: 'auth.errorPasswordIsDefault',
  password_too_long: 'auth.errorPasswordTooLong',
  password_unchanged: 'auth.errorPasswordUnchanged',
  invalid_current_password: 'auth.errorInvalidCurrentPassword',
}

function messageFor(cause: unknown): string {
  if (isApiError(cause) && KNOWN_ERRORS[cause.code]) return t(KNOWN_ERRORS[cause.code])
  return formatApiError(cause)
}

async function submit() {
  if (!currentPassword.value || !newPassword.value || pending.value) return
  pending.value = true
  error.value = null
  try {
    const response = await authApi.changePassword(currentPassword.value, newPassword.value)
    currentPassword.value = ''
    newPassword.value = ''
    auth.setLoginStatus({ passwordLoginEnabled: true, mustChangePassword: false })
    auth.setAdminToken(response.token)
    await router.replace({ path: redirect.value })
  } catch (cause) {
    error.value = messageFor(cause)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <main class="grid min-h-screen place-items-center bg-background px-4 text-foreground">
    <section class="w-full max-w-sm space-y-6 rounded-lg border bg-card p-6 shadow-sm">
      <header class="space-y-1">
        <h1 class="text-lg font-semibold">{{ t('auth.changePasswordTitle') }}</h1>
        <p class="text-sm text-muted-foreground">{{ t('auth.changePasswordDescription') }}</p>
      </header>

      <div
        v-if="error"
        role="alert"
        class="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
      >
        {{ error }}
      </div>

      <form class="space-y-4" @submit.prevent="submit">
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('auth.currentPasswordLabel') }}</span>
          <input v-model="currentPassword" type="password" autocomplete="current-password" class="admin-input w-full" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('auth.newPasswordLabel') }}</span>
          <input v-model="newPassword" type="password" autocomplete="new-password" class="admin-input w-full" />
          <span class="block text-xs text-muted-foreground">{{ t('auth.newPasswordHint') }}</span>
        </label>
        <Button type="submit" class="w-full" :disabled="!currentPassword || !newPassword || pending">
          {{ pending ? t('auth.savingPassword') : t('auth.savePassword') }}
        </Button>
      </form>

      <Button variant="ghost" size="sm" class="w-full" @click="signOut()">{{ t('auth.logout') }}</Button>
    </section>
  </main>
</template>
