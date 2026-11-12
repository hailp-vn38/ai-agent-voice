<script setup lang="ts">
import { ShieldCheck } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import { signOut } from '@/api/session'
import BaseModal from '@/components/admin/BaseModal.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { useAuthStore } from '@/stores/auth'

const router = useRouter()
const { t } = useI18n()
const auth = useAuthStore()

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

const passwordLoginEnabled = computed(() => auth.loginStatus?.passwordLoginEnabled !== false)

// Password Login toggle: the request is sent only after the Admin confirms and types the current password.
const toggleOpen = ref(false)
const toggleTarget = ref(false)
const togglePassword = ref('')
const togglePending = ref(false)
const toggleError = ref<string | null>(null)

function openToggle() {
  toggleTarget.value = !passwordLoginEnabled.value
  togglePassword.value = ''
  toggleError.value = null
  toggleOpen.value = true
}

async function confirmToggle() {
  if (!togglePassword.value || togglePending.value) return
  togglePending.value = true
  toggleError.value = null
  try {
    const response = await authApi.setPasswordLogin(toggleTarget.value, togglePassword.value)
    auth.setLoginStatus({
      passwordLoginEnabled: response.password_login_enabled,
      mustChangePassword: response.must_change_password,
    })
    togglePassword.value = ''
    toggleOpen.value = false
  } catch (cause) {
    toggleError.value = messageFor(cause)
  } finally {
    togglePending.value = false
  }
}

// Change password: the server rotates the token, so this browser keeps a fresh one.
const currentPassword = ref('')
const newPassword = ref('')
const changePending = ref(false)
const changeError = ref<string | null>(null)

async function submitChangePassword() {
  if (!currentPassword.value || !newPassword.value || changePending.value) return
  changePending.value = true
  changeError.value = null
  try {
    const response = await authApi.changePassword(currentPassword.value, newPassword.value)
    currentPassword.value = ''
    newPassword.value = ''
    auth.setAdminWebToken(response.token)
  } catch (cause) {
    changeError.value = messageFor(cause)
  } finally {
    changePending.value = false
  }
}

// Sign out everywhere: the server revokes other tokens and returns one for this browser.
const everywherePending = ref(false)
const everywhereError = ref<string | null>(null)

async function signOutEverywhere() {
  if (everywherePending.value) return
  everywherePending.value = true
  everywhereError.value = null
  try {
    const response = await authApi.logoutEverywhere()
    auth.setAdminWebToken(response.token)
  } catch (cause) {
    everywhereError.value = messageFor(cause)
  } finally {
    everywherePending.value = false
  }
}
</script>

<template>
  <section class="space-y-4" data-testid="system-security-page">
    <DetailHeader
      :title="t('system.hub.securityTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet">
          <ShieldCheck class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.securitySub') }}</span>
        </div>
      </template>
    </DetailHeader>

    <div class="studio-panel space-y-3 p-4">
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <h3 class="text-sm font-semibold">{{ t('security.passwordLoginTitle') }}</h3>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
            {{ t('security.passwordLoginDescription') }}
          </p>
        </div>
        <button
          type="button"
          role="switch"
          data-testid="password-login-switch"
          :aria-checked="passwordLoginEnabled ? 'true' : 'false'"
          :aria-label="t('security.passwordLoginTitle')"
          class="relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors"
          :class="passwordLoginEnabled ? 'bg-studio-violet' : 'bg-muted'"
          @click="openToggle"
        >
          <span
            class="inline-block size-5 rounded-full bg-white shadow transition-transform"
            :class="passwordLoginEnabled ? 'translate-x-5' : 'translate-x-0.5'"
          />
        </button>
      </div>
      <p class="text-xs text-muted-foreground">
        {{ passwordLoginEnabled ? t('security.passwordLoginOn') : t('security.passwordLoginOff') }}
      </p>
    </div>

    <div class="studio-panel space-y-3 p-4">
      <h3 class="text-sm font-semibold">{{ t('security.changePasswordTitle') }}</h3>
      <p class="text-xs text-muted-foreground">{{ t('security.changePasswordDescription') }}</p>
      <div
        v-if="changeError"
        role="alert"
        class="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
      >
        {{ changeError }}
      </div>
      <form class="space-y-3" data-testid="change-password-form" @submit.prevent="submitChangePassword">
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('auth.currentPasswordLabel') }}</span>
          <input v-model="currentPassword" type="password" autocomplete="current-password" class="admin-input w-full" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('auth.newPasswordLabel') }}</span>
          <input v-model="newPassword" type="password" autocomplete="new-password" class="admin-input w-full" />
          <span class="block text-xs text-muted-foreground">{{ t('auth.newPasswordHint') }}</span>
        </label>
        <Button type="submit" :disabled="!currentPassword || !newPassword || changePending">
          {{ changePending ? t('auth.savingPassword') : t('auth.savePassword') }}
        </Button>
      </form>
    </div>

    <div class="studio-panel space-y-3 p-4">
      <h3 class="text-sm font-semibold">{{ t('security.signOutEverywhereTitle') }}</h3>
      <p class="text-xs text-muted-foreground">{{ t('security.signOutEverywhereDescription') }}</p>
      <div
        v-if="everywhereError"
        role="alert"
        class="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
      >
        {{ everywhereError }}
      </div>
      <div class="flex flex-wrap gap-2">
        <Button
          variant="outline"
          data-testid="logout-everywhere"
          :disabled="everywherePending"
          @click="signOutEverywhere"
        >
          {{ everywherePending ? t('security.signingOutEverywhere') : t('security.signOutEverywhereAction') }}
        </Button>
        <Button variant="outline" data-testid="logout-this-browser" @click="signOut()">
          {{ t('auth.logout') }}
        </Button>
      </div>
      <p class="text-xs text-muted-foreground">{{ t('security.signOutThisBrowserDescription') }}</p>
    </div>

    <BaseModal
      v-model="toggleOpen"
      :title="toggleTarget ? t('security.turnOnTitle') : t('security.turnOffTitle')"
      :description="toggleTarget ? t('security.turnOnDescription') : t('security.turnOffWarning')"
      width-class="max-w-md"
      :close-disabled="togglePending"
    >
      <div class="space-y-4">
        <div
          v-if="toggleError"
          role="alert"
          class="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700"
        >
          {{ toggleError }}
        </div>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('security.confirmPasswordLabel') }}</span>
          <input v-model="togglePassword" type="password" autocomplete="current-password" class="admin-input w-full" />
        </label>
        <div class="flex justify-end gap-2">
          <Button variant="outline" :disabled="togglePending" @click="toggleOpen = false">
            {{ t('security.cancel') }}
          </Button>
          <Button
            data-testid="password-login-confirm"
            :class="toggleTarget ? '' : 'bg-red-600 text-white hover:bg-red-700'"
            :disabled="!togglePassword || togglePending"
            @click="confirmToggle"
          >
            {{ togglePending ? t('security.applying') : toggleTarget ? t('security.turnOnConfirm') : t('security.turnOffConfirm') }}
          </Button>
        </div>
      </div>
    </BaseModal>
  </section>
</template>
