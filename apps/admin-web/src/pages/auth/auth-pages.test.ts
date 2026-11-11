import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import { ApiError } from '@/api/errors'
import { useAuthStore } from '@/stores/auth'

const authApi = vi.hoisted(() => ({ login: vi.fn(), changePassword: vi.fn(), status: vi.fn() }))
vi.mock('@/api/auth', () => ({ authApi }))

import ChangePasswordPage from './ChangePasswordPage.vue'
import LoginPage from './LoginPage.vue'

const stub = { template: '<div />' }

async function mountAt(page: 'login' | 'change', path: string) {
  const pinia = createPinia()
  setActivePinia(pinia)
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/login', name: 'login', component: LoginPage, meta: { public: true } },
      { path: '/change-password', name: 'change-password', component: ChangePasswordPage, meta: { public: true } },
      { path: '/overview', name: 'overview', component: stub },
      { path: '/jobs', name: 'jobs', component: stub },
    ],
  })
  await router.push(path)
  const wrapper = mount(page === 'login' ? LoginPage : ChangePasswordPage, { global: { plugins: [pinia, router] }, attachTo: document.body })
  await flushPromises()
  return { wrapper, router }
}

const apiError = (code: string, status: number) => new ApiError(`API request failed: ${code}`, status, code, 'req-1')

describe('admin sign-in pages', () => {
  beforeEach(() => {
    setLocale('en')
    localStorage.clear()
    authApi.login.mockReset()
    authApi.changePassword.mockReset()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  describe('LoginPage', () => {
    it('shows a dedicated message for a wrong password and keeps the Admin on the page', async () => {
      authApi.login.mockRejectedValue(apiError('invalid_password', 401))
      const { wrapper, router } = await mountAt('login', '/login')

      await wrapper.get('input[type="password"]').setValue('wrong-password')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(wrapper.get('[role="alert"]').text()).toContain('The password is incorrect.')
      expect(router.currentRoute.value.path).toBe('/login')
      expect(localStorage.getItem('voice-agent-admin-token')).toBeNull()
    })

    it('shows a lockout message when too many attempts were made', async () => {
      authApi.login.mockRejectedValue(apiError('too_many_attempts', 429))
      const { wrapper } = await mountAt('login', '/login')

      await wrapper.get('input[type="password"]').setValue('any-password')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(wrapper.get('[role="alert"]').text()).toContain('Too many failed attempts')
    })

    it('stores the token and returns to the requested route after a normal sign-in', async () => {
      authApi.login.mockResolvedValue({ token: 'fresh-token', token_type: 'Bearer', expires_at: 1, must_change_password: false })
      const { wrapper, router } = await mountAt('login', '/login?redirect=%2Fjobs')

      await wrapper.get('input[type="password"]').setValue('real-password')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(localStorage.getItem('voice-agent-admin-token')).toBe('fresh-token')
      expect(useAuthStore().loginStatus?.mustChangePassword).toBe(false)
      expect(router.currentRoute.value.path).toBe('/jobs')
    })

    it('sends the Admin to the password change page when the default password is still in use', async () => {
      authApi.login.mockResolvedValue({ token: 'default-token', token_type: 'Bearer', expires_at: 1, must_change_password: true })
      const { wrapper, router } = await mountAt('login', '/login')

      await wrapper.get('input[type="password"]').setValue('admin')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(router.currentRoute.value.name).toBe('change-password')
      expect(useAuthStore().mustChangePassword).toBe(true)
    })
  })

  describe('ChangePasswordPage', () => {
    it.each([
      ['password_too_short', 'at least 8 characters'],
      ['password_is_default', 'cannot be the default password'],
      ['password_unchanged', 'must differ from the current one'],
      ['invalid_current_password', 'current password is incorrect'],
      ['password_too_long', 'too long'],
    ])('shows a specific message for %s', async (code, text) => {
      authApi.changePassword.mockRejectedValue(apiError(code, 422))
      const { wrapper } = await mountAt('change', '/change-password')

      const inputs = wrapper.findAll('input[type="password"]')
      await inputs[0].setValue('admin')
      await inputs[1].setValue('new-password-1')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(wrapper.get('[role="alert"]').text().toLowerCase()).toContain(text.toLowerCase())
    })

    it('stores the new token, clears the forced flag and returns to the requested route', async () => {
      authApi.changePassword.mockResolvedValue({ token: 'new-token', token_type: 'Bearer', expires_at: 1, must_change_password: false })
      const { wrapper, router } = await mountAt('change', '/change-password?redirect=%2Fjobs')
      useAuthStore().setLoginStatus({ passwordLoginEnabled: true, mustChangePassword: true })

      const inputs = wrapper.findAll('input[type="password"]')
      await inputs[0].setValue('admin')
      await inputs[1].setValue('a-brand-new-password')
      await wrapper.get('form').trigger('submit')
      await flushPromises()

      expect(authApi.changePassword).toHaveBeenCalledWith('admin', 'a-brand-new-password')
      expect(localStorage.getItem('voice-agent-admin-token')).toBe('new-token')
      expect(useAuthStore().mustChangePassword).toBe(false)
      expect(router.currentRoute.value.path).toBe('/jobs')
    })
  })
})
