import { flushPromises, mount } from '@vue/test-utils'
import { createRouter, createMemoryHistory } from 'vue-router'
import { expect, it, vi } from 'vitest'
import McpServersView from './McpServersView.vue'
const api = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn(), update: vi.fn() }))
vi.mock('@/api/mcp', () => ({ mcpApi: api }))
it('links the card to details while edit controls remain outside its link', async () => {
  const server = { key: 'weather', name: 'Weather', url: 'https://example.test/mcp', auth: { type: 'none' }, enabled: true, revision: 1 }
  api.list.mockResolvedValue({ items: [server], page_size: 50 })
  api.get.mockResolvedValue(server)
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/mcp', component: McpServersView }, { path: '/mcp/:key', component: { template: '<p>Details</p>' } }] })
  await router.push('/mcp'); await router.isReady()
  const wrapper = mount(McpServersView, { global: { plugins: [router], stubs: { McpServerFormModal: true, ConfirmDialog: true } } })
  await flushPromises()
  const card = wrapper.get('article')
  expect(card.get('a').attributes('href')).toBe('/mcp/weather')
  expect(card.get('a').find('button').exists()).toBe(false)
  await card.get('button').trigger('click'); await flushPromises()
  expect(router.currentRoute.value.path).toBe('/mcp')
  await card.get('a').trigger('click'); await flushPromises()
  expect(router.currentRoute.value.path).toBe('/mcp/weather')
  wrapper.unmount()
})
