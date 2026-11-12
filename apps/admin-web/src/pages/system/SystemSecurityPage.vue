<script setup lang="ts">
import { Check, Eye, EyeOff, Fingerprint, KeyRound, LogOut, Monitor, ShieldCheck } from '@lucide/vue'
import { computed, ref } from 'vue'

import { authApi } from '@/api/auth'
import { formatApiError, isApiError } from '@/api/errors'
import { signOut } from '@/api/session'
import BaseModal from '@/components/admin/BaseModal.vue'
import SystemPageHeader from '@/components/system/SystemPageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { useAuthStore } from '@/stores/auth'

const { t } = useI18n()
const auth = useAuthStore()

const KNOWN_ERRORS: Record<string, MessageKey> = {
  password_too_short: 'auth.errorPasswordTooShort',
  password_is_default: 'auth.errorPasswordIsDefault',
  password_too_long: 'auth.errorPasswordTooLong',
  password_unchanged: 'auth.errorPasswordUnchanged',
  invalid_current_password: 'auth.errorInvalidCurrentPassword',
  too_many_attempts: 'auth.errorTooManyAttempts',
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
    // Password Login is back on: a browser that holds no token has to sign in from here on.
    if (response.password_login_enabled && !auth.adminWebToken) signOut()
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
const passwordChanged = ref(false)
const showPasswords = ref(false)

async function submitChangePassword() {
  if (!currentPassword.value || !newPassword.value || changePending.value) return
  changePending.value = true
  changeError.value = null
  passwordChanged.value = false
  try {
    const response = await authApi.changePassword(currentPassword.value, newPassword.value)
    currentPassword.value = ''
    newPassword.value = ''
    auth.setAdminWebToken(response.token)
    passwordChanged.value = true
  } catch (cause) {
    changeError.value = messageFor(cause)
  } finally {
    changePending.value = false
  }
}

// Sign out everywhere: the server revokes other tokens and returns one for this browser.
const everywherePending = ref(false)
const everywhereError = ref<string | null>(null)
const sessionsRevoked = ref(false)

async function signOutEverywhere() {
  if (everywherePending.value) return
  everywherePending.value = true
  everywhereError.value = null
  sessionsRevoked.value = false
  try {
    const response = await authApi.logoutEverywhere()
    auth.setAdminWebToken(response.token)
    sessionsRevoked.value = true
  } catch (cause) {
    everywhereError.value = messageFor(cause)
  } finally {
    everywherePending.value = false
  }
}
</script>

<template>
  <section class="system-detail security-page" data-testid="system-security-page">
    <SystemPageHeader number="03" :eyebrow="t('system.detail.securityEyebrow')"
      :title="t('system.detail.securityTitle')" :description="t('system.detail.securityDescription')" />

    <section class="security-access" :data-protected="passwordLoginEnabled" aria-labelledby="access-heading">
      <div class="security-emblem" aria-hidden="true"><Fingerprint class="size-12" /><span /></div>
      <div class="security-access-copy">
        <p class="sys-kicker">01 / {{ t('system.detail.access') }}</p>
        <h2 id="access-heading">{{ t(passwordLoginEnabled ? 'system.detail.protected' : 'system.detail.unprotected') }}</h2>
        <p>{{ t('system.detail.accessDescription') }}</p>
        <span class="security-access-state">{{ t(passwordLoginEnabled ? 'security.passwordLoginOn' : 'security.passwordLoginOff') }}</span>
      </div>
      <button type="button" role="switch" data-testid="password-login-switch"
        :aria-checked="passwordLoginEnabled ? 'true' : 'false'" :aria-label="t('security.passwordLoginTitle')"
        class="security-switch" @click="openToggle"><span /></button>
    </section>

    <div class="security-grid">
      <section class="sys-panel security-password" aria-labelledby="password-heading">
        <header class="sys-panel-header"><span class="sys-section-number">02</span>
          <div><h2 id="password-heading">{{ t('system.detail.password') }}</h2><p>{{ t('system.detail.passwordDescription') }}</p></div>
          <KeyRound class="sys-heading-icon size-5" aria-hidden="true" />
        </header>
        <form class="security-password-form" data-testid="change-password-form" @submit.prevent="submitChangePassword">
          <p v-if="changeError" role="alert" class="sys-alert">{{ changeError }}</p>
          <p v-if="passwordChanged" role="status" class="sys-success"><Check class="size-4" aria-hidden="true" />{{ t('system.detail.passwordUpdated') }}</p>
          <label class="sys-field"><span>{{ t('auth.currentPasswordLabel') }}</span>
            <input v-model="currentPassword" :type="showPasswords ? 'text' : 'password'" autocomplete="current-password" class="admin-input" :disabled="changePending" required />
          </label>
          <label class="sys-field"><span>{{ t('auth.newPasswordLabel') }}</span>
            <input v-model="newPassword" :type="showPasswords ? 'text' : 'password'" autocomplete="new-password" class="admin-input" :disabled="changePending" required />
            <small>{{ t('auth.newPasswordHint') }}</small>
          </label>
          <footer class="security-password-actions">
            <button type="button" class="sys-text-button" :aria-pressed="showPasswords" @click="showPasswords = !showPasswords">
              <component :is="showPasswords ? EyeOff : Eye" class="size-3.5" aria-hidden="true" />{{ t(showPasswords ? 'system.detail.hidePassword' : 'system.detail.showPassword') }}
            </button>
            <Button type="submit" class="sys-primary" :disabled="!currentPassword || !newPassword || changePending">
              {{ changePending ? t('auth.savingPassword') : t('auth.savePassword') }}
            </Button>
          </footer>
        </form>
      </section>

      <section class="sys-panel security-sessions" aria-labelledby="sessions-heading">
        <header class="sys-panel-header"><span class="sys-section-number">03</span>
          <div><h2 id="sessions-heading">{{ t('system.detail.sessions') }}</h2><p>{{ t('system.detail.sessionsDescription') }}</p></div>
        </header>
        <div class="browser-identity"><Monitor class="size-6" aria-hidden="true" />
          <div><span class="sys-kicker">{{ t('system.detail.currentBrowser') }}</span><p>{{ t(auth.adminWebToken ? 'system.detail.signedIn' : 'system.detail.noToken') }}</p></div>
        </div>
        <div class="security-session-actions">
          <p v-if="everywhereError" role="alert" class="sys-alert">{{ everywhereError }}</p>
          <p v-if="sessionsRevoked" role="status" class="sys-success"><Check class="size-4" aria-hidden="true" />{{ t('system.detail.sessionUpdated') }}</p>
          <Button variant="outline" data-testid="logout-everywhere" :disabled="everywherePending" @click="signOutEverywhere">
            <ShieldCheck class="size-4" aria-hidden="true" />{{ everywherePending ? t('security.signingOutEverywhere') : t('security.signOutEverywhereAction') }}
          </Button>
          <p>{{ t('security.signOutEverywhereDescription') }}</p>
          <div class="session-divider" />
          <Button variant="ghost" data-testid="logout-this-browser" @click="signOut()"><LogOut class="size-4" aria-hidden="true" />{{ t('auth.logout') }}</Button>
          <p>{{ t('security.signOutThisBrowserDescription') }}</p>
        </div>
      </section>
    </div>
    <p class="security-footnote"><ShieldCheck class="size-4" aria-hidden="true" />{{ t('system.detail.securityNote') }}</p>

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
          class="sys-alert"
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
            :class="toggleTarget ? '' : 'sys-danger-button'"
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
