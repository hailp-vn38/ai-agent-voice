import { flushPromises, mount } from '@vue/test-utils'
import { reactive } from 'vue'
import { expect, it, vi } from 'vitest'
import McpServerDetailPage from './McpServerDetailPage.vue'
const route = reactive({ params: { key: 'weather' } })
vi.mock('vue-router', () => ({ useRoute: () => route, useRouter: () => ({ push: vi.fn() }), RouterLink: { template: '<a><slot /></a>' } }))
const api = vi.hoisted(() => ({ get: vi.fn(), update: vi.fn(), remove: vi.fn(), testSavedConnection: vi.fn(), discoverSavedTools: vi.fn() }))
vi.mock('@/api/mcp', () => ({ mcpApi: api }))
it('loads a deep link without probing and hides unsafe endpoint parts', async () => {
  api.get.mockResolvedValue({ key: 'weather', name: 'Weather', url: 'https://user:secret@example.test/mcp?token=hidden#fragment', auth: { type: 'none' }, enabled: true, revision: 2, connect_timeout_ms: 5000, request_timeout_ms: 30000 })
  const wrapper = mount(McpServerDetailPage, { global: { stubs: { BaseModal: true } } })
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith('weather', expect.any(AbortSignal))
  expect(api.testSavedConnection).not.toHaveBeenCalled()
  expect(api.discoverSavedTools).not.toHaveBeenCalled()
  expect(wrapper.text()).toContain('https://example.test/mcp')
  expect(wrapper.text()).not.toMatch(/secret|hidden|fragment/)
  route.params.key = 'next'
  await flushPromises()
  expect(api.get).toHaveBeenLastCalledWith('next', expect.any(AbortSignal))
  wrapper.unmount()
})
