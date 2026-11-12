<script setup lang="ts">
import { ArrowRight, CircleAlert, LoaderCircle } from '@lucide/vue'
import { computed, ref, useId } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import { signOut } from '@/api/session'
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

const currentPassword = ref('')
const newPassword = ref('')
const pending = ref(false)
const error = ref<string | null>(null)
const errorCode = ref<string | null>(null)
const errorId = useId()

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
  errorCode.value = null
  try {
    const response = await authApi.changePassword(currentPassword.value, newPassword.value)
    currentPassword.value = ''
    newPassword.value = ''
    auth.setLoginStatus({ passwordLoginEnabled: true, mustChangePassword: false })
    auth.setAdminWebToken(response.token)
    await router.replace({ path: redirect.value })
  } catch (cause) {
    error.value = messageFor(cause)
    errorCode.value = isApiError(cause) ? cause.code : null
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <AuthPageShell :title="t('auth.changePasswordTitle')" :description="t('auth.changePasswordDescription')">
    <div v-if="error" :id="errorId" role="alert" class="auth-error">
      <CircleAlert :size="16" aria-hidden="true" /><span>{{ error }}</span>
    </div>
    <form class="space-y-5" :aria-busy="pending" @submit.prevent="submit">
      <PasswordField
        @submit="submit"
        v-model="currentPassword" name="current-password" :label="t('auth.currentPasswordLabel')"
        :disabled="pending" :invalid="errorCode === 'invalid_current_password'"
        :described-by="error ? errorId : undefined"
      />
      <PasswordField
        @submit="submit"
        v-model="newPassword" name="new-password" autocomplete="new-password" :label="t('auth.newPasswordLabel')"
        :hint="t('auth.newPasswordHint')" :disabled="pending"
        :invalid="!!errorCode && errorCode.startsWith('password_')"
        :described-by="error ? errorId : undefined"
      />
      <Button type="submit" class="auth-submit" :disabled="!currentPassword || !newPassword || pending">
        <LoaderCircle v-if="pending" :size="16" class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        {{ pending ? t('auth.savingPassword') : t('auth.savePassword') }}
        <ArrowRight v-if="!pending" :size="16" aria-hidden="true" />
      </Button>
    </form>
    <Button variant="ghost" size="sm" class="mt-3 w-full text-xs text-muted-foreground" :disabled="pending" @click="signOut()">{{ t('auth.logout') }}</Button>
  </AuthPageShell>
</template>
