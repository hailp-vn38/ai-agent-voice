<script setup lang="ts">
import { ArrowRight, CircleAlert, LoaderCircle } from '@lucide/vue'
import { computed, ref, useId } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import AuthPageShell from '@/components/auth/AuthPageShell.vue'
import PasswordField from '@/components/auth/PasswordField.vue'
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
const invalidPassword = ref(false)
const errorId = useId()

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
  invalidPassword.value = false
  try {
    const response = await authApi.login(password.value)
    password.value = ''
    auth.setLoginStatus({ passwordLoginEnabled: true, mustChangePassword: response.must_change_password })
    auth.setAdminWebToken(response.token)
    await router.replace(response.must_change_password
      ? { name: 'change-password', query: { redirect: redirect.value } }
      : { path: redirect.value })
  } catch (cause) {
    error.value = messageFor(cause)
    invalidPassword.value = isApiError(cause) && cause.code === 'invalid_password'
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <AuthPageShell :title="t('auth.loginTitle')" :description="t('auth.loginDescription')">
    <div v-if="error" :id="errorId" role="alert" class="auth-error">
      <CircleAlert :size="16" aria-hidden="true" /><span>{{ error }}</span>
    </div>
    <form class="space-y-5" :aria-busy="pending" @submit.prevent="submit">
      <PasswordField
        @submit="submit"
        v-model="password" :label="t('auth.passwordLabel')"
        :placeholder="t('auth.passwordPlaceholder')" :disabled="pending"
        :invalid="invalidPassword" :described-by="error ? errorId : undefined"
      />
      <Button type="submit" class="auth-submit" :disabled="!password || pending">
        <LoaderCircle v-if="pending" :size="16" class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        {{ pending ? t('auth.signingIn') : t('auth.signIn') }}
        <ArrowRight v-if="!pending" :size="16" aria-hidden="true" />
      </Button>
    </form>
  </AuthPageShell>
</template>
