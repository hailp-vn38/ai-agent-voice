<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { safeTarget } from '@/router/auth-guard'
import { useAuthStore } from '@/stores/auth'

const { t } = useI18n()
const auth = useAuthStore()
const route = useRoute()
const router = useRouter()

const password = ref('')
const pending = ref(false)
const error = ref<string | null>(null)

const redirect = computed(() => safeTarget(route.query.redirect))

const KNOWN_ERRORS: Record<string, MessageKey> = {
  invalid_password: 'auth.errorInvalidPassword',
  too_many_attempts: 'auth.errorTooManyAttempts',
  password_login_disabled: 'auth.errorPasswordLoginDisabled',
}

function messageFor(cause: unknown): string {
  if (isApiError(cause) && KNOWN_ERRORS[cause.code]) return t(KNOWN_ERRORS[cause.code])
  return formatApiError(cause)
}

async function submit() {
  if (!password.value || pending.value) return
  pending.value = true
  error.value = null
  try {
    const response = await authApi.login(password.value)
    password.value = ''
    auth.setLoginStatus({ passwordLoginEnabled: true, mustChangePassword: response.must_change_password })
    auth.setAdminToken(response.token)
    await router.replace(response.must_change_password
      ? { name: 'change-password', query: { redirect: redirect.value } }
      : { path: redirect.value })
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
        <h1 class="text-lg font-semibold">{{ t('auth.loginTitle') }}</h1>
        <p class="text-sm text-muted-foreground">{{ t('auth.loginDescription') }}</p>
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
          <span class="text-sm font-medium">{{ t('auth.passwordLabel') }}</span>
          <input
            v-model="password"
            type="password"
            autocomplete="current-password"
            class="admin-input w-full"
            :placeholder="t('auth.passwordPlaceholder')"
          />
        </label>
        <Button type="submit" class="w-full" :disabled="!password || pending">
          {{ pending ? t('auth.signingIn') : t('auth.signIn') }}
        </Button>
      </form>
    </section>
  </main>
</template>
