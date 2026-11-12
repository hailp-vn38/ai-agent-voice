import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import { ApiError } from '@/api/errors'
import { useAuthStore } from '@/stores/auth'

const authApi = vi.hoisted(() => ({
  changePassword: vi.fn(),
  logoutEverywhere: vi.fn(),
  setPasswordLogin: vi.fn(),
}))
vi.mock('@/api/auth', () => ({ authApi }))

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
  RouterLink: { props: ['to'], template: '<a :href="to"><slot /></a>' },
}))

import SystemSecurityPage from './SystemSecurityPage.vue'

const apiError = (code: string, status: number) => new ApiError(`API request failed: ${code}`, status, code, 'req-1')

function dialog(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>('[role="dialog"]')
}

function dialogPasswordInput(): HTMLInputElement {
  const input = dialog()?.querySelector<HTMLInputElement>('input[type="password"]')
  if (!input) throw new Error('confirmation dialog with a password input is not open')
  return input
}

async function mountPage(passwordLoginEnabled = true) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const auth = useAuthStore()
  auth.setLoginStatus({ passwordLoginEnabled, mustChangePassword: false })
  const wrapper = mount(SystemSecurityPage, {
    global: { plugins: [pinia] },
    attachTo: document.body,
  })
  await flushPromises()
  return { wrapper, auth }
}

describe('SystemSecurityPage', () => {
  beforeEach(() => {
    setLocale('en')
    localStorage.clear()
    authApi.changePassword.mockReset()
    authApi.logoutEverywhere.mockReset()
    authApi.setPasswordLogin.mockReset()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('does not send anything when the switch is clicked; the request waits for confirmation', async () => {
    const { wrapper } = await mountPage(true)

    await wrapper.get('[data-testid="password-login-switch"]').trigger('click')
    await flushPromises()

    expect(dialog()).not.toBeNull()
    expect(dialog()?.textContent).toContain('Anyone who can reach this server will have full access')
    expect(authApi.setPasswordLogin).not.toHaveBeenCalled()
  })

  it('turns Password Login off only after the current password is entered and confirmed', async () => {
    authApi.setPasswordLogin.mockResolvedValue({ password_login_enabled: false, must_change_password: false })
    const { wrapper, auth } = await mountPage(true)

    await wrapper.get('[data-testid="password-login-switch"]').trigger('click')
    await flushPromises()
    dialogPasswordInput().value = 'current-password'
    dialogPasswordInput().dispatchEvent(new Event('input'))
    await flushPromises()
    document.body.querySelector<HTMLButtonElement>('[data-testid="password-login-confirm"]')?.click()
    await flushPromises()

    expect(authApi.setPasswordLogin).toHaveBeenCalledWith(false, 'current-password')
    expect(auth.loginStatus?.passwordLoginEnabled).toBe(false)
    expect(wrapper.get('[data-testid="password-login-switch"]').attributes('aria-checked')).toBe('false')
    expect(dialog()).toBeNull()
  })

  it('shows the wrong-password error and leaves the switch where it was', async () => {
    authApi.setPasswordLogin.mockRejectedValue(apiError('invalid_current_password', 422))
    const { wrapper, auth } = await mountPage(true)

    await wrapper.get('[data-testid="password-login-switch"]').trigger('click')
    await flushPromises()
    dialogPasswordInput().value = 'wrong-password'
    dialogPasswordInput().dispatchEvent(new Event('input'))
    await flushPromises()
    document.body.querySelector<HTMLButtonElement>('[data-testid="password-login-confirm"]')?.click()
    await flushPromises()

    expect(document.body.querySelector('[role="dialog"] [role="alert"]')?.textContent).toContain(
      'The current password is incorrect.',
    )
    expect(auth.loginStatus?.passwordLoginEnabled).toBe(true)
    expect(wrapper.get('[data-testid="password-login-switch"]').attributes('aria-checked')).toBe('true')
    expect(dialog()).not.toBeNull()
  })

  it('turns Password Login back on with the current password and no other credential', async () => {
    authApi.setPasswordLogin.mockResolvedValue({ password_login_enabled: true, must_change_password: false })
    const { wrapper, auth } = await mountPage(false)

    await wrapper.get('[data-testid="password-login-switch"]').trigger('click')
    await flushPromises()
    dialogPasswordInput().value = 'current-password'
    dialogPasswordInput().dispatchEvent(new Event('input'))
    await flushPromises()
    document.body.querySelector<HTMLButtonElement>('[data-testid="password-login-confirm"]')?.click()
    await flushPromises()

    expect(authApi.setPasswordLogin).toHaveBeenCalledWith(true, 'current-password')
    expect(auth.loginStatus?.passwordLoginEnabled).toBe(true)
  })

  it('keeps this browser signed in with the new token after changing the password', async () => {
    authApi.changePassword.mockResolvedValue({ token: 'rotated-token', token_type: 'Bearer', expires_at: 1, must_change_password: false })
    const { wrapper } = await mountPage(true)

    const inputs = wrapper.findAll('input[type="password"]')
    await inputs[0].setValue('current-password')
    await inputs[1].setValue('brand-new-password')
    await wrapper.get('form[data-testid="change-password-form"]').trigger('submit')
    await flushPromises()

    expect(authApi.changePassword).toHaveBeenCalledWith('current-password', 'brand-new-password')
    expect(localStorage.getItem('voice-agent-admin-token')).toBe('rotated-token')
  })

  it('keeps this browser signed in with a fresh token after signing out everywhere', async () => {
    authApi.logoutEverywhere.mockResolvedValue({ token: 'fresh-token', token_type: 'Bearer', expires_at: 1, must_change_password: false })
    const { wrapper } = await mountPage(true)

    await wrapper.get('[data-testid="logout-everywhere"]').trigger('click')
    await flushPromises()

    expect(authApi.logoutEverywhere).toHaveBeenCalledTimes(1)
    expect(localStorage.getItem('voice-agent-admin-token')).toBe('fresh-token')
  })
})
