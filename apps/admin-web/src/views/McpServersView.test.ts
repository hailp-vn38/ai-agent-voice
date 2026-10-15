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
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/mcp', component: McpServersView }, { path: '/mcp/:key', component: { template: '<p>Details</p>' } }, { path: '/agents', component: { template: '<p>Agents</p>' } }] })
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

it('replaces only the item list with shimmer while keeping the header and filters mounted', async () => {
  let finish!: (value: { items: unknown[]; page_size: number }) => void
  api.list.mockImplementation(() => new Promise((resolve) => { finish = resolve }))
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/mcp', component: McpServersView }, { path: '/:pathMatch(.*)*', component: { template: '<p />' } }] })
  await router.push('/mcp')
  const wrapper = mount(McpServersView, { global: { plugins: [router], stubs: { McpServerFormModal: true, ConfirmDialog: true } } })
  try {
    await flushPromises()
    const header = wrapper.get('[data-detail-header]').element
    const search = wrapper.get('input[type="search"]').element
    const filter = wrapper.get('select').element
    const checkLoading = () => {
      expect(wrapper.findAll('.card-shimmer')).toHaveLength(6)
      expect(wrapper.get('[data-detail-header]').element).toBe(header)
      expect(wrapper.get('input[type="search"]').element).toBe(search)
      expect(wrapper.get('select').element).toBe(filter)
      expect(wrapper.find('[data-detail-header] .card-shimmer').exists()).toBe(false)
      expect(wrapper.find('article').exists()).toBe(false)
    }
    checkLoading()
    const server = { key: 'weather', name: 'Weather', url: 'https://example.test/mcp', auth: { type: 'none' }, enabled: true, revision: 1 }
    finish({ items: [server], page_size: 50 })
    await flushPromises()
    expect(wrapper.findAll('.card-shimmer')).toHaveLength(0)
    expect(wrapper.get('article').text()).toContain('Weather')

    await wrapper.get('[data-detail-header] button').trigger('click')
    checkLoading()
    finish({ items: [server], page_size: 50 })
    await flushPromises()
    expect(wrapper.findAll('.card-shimmer')).toHaveLength(0)
    expect(wrapper.get('article').text()).toContain('Weather')
  } finally {
    wrapper.unmount()
  }
})
