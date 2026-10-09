import { flushPromises, mount } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'
import McpDiagnosticPanel from './McpDiagnosticPanel.vue'
const api = vi.hoisted(() => ({ testDraftConnection: vi.fn(), discoverDraftTools: vi.fn(), testSavedConnection: vi.fn(), discoverSavedTools: vi.fn() }))
vi.mock('@/api/mcp', () => ({ mcpApi: api }))
it('does not automatically probe and uses independent operations without saving', async () => {
  api.testDraftConnection.mockResolvedValue({ connected_at_test_time: true, elapsed_ms: 3 })
  api.discoverDraftTools.mockResolvedValue({ complete: true, tools: [], dropped_tools: 0, elapsed_ms: 4 })
  const wrapper = mount(McpDiagnosticPanel, { props: { draft: { key: 'weather', url: 'https://example.test/mcp', auth: { type: 'none' } } } })
  expect(api.testDraftConnection).not.toHaveBeenCalled()
  await wrapper.get('[data-test-connection]').trigger('click')
  await flushPromises()
  expect(api.testDraftConnection).toHaveBeenCalledTimes(1)
  expect(api.discoverDraftTools).not.toHaveBeenCalled()
  await wrapper.get('[data-discover-tools]').trigger('click')
  await flushPromises()
  expect(wrapper.text()).toContain('0 tool')
  await wrapper.setProps({ draft: { key: 'weather', url: 'https://other.test/mcp', auth: { type: 'none' } } })
  expect(wrapper.text()).not.toContain('0 tool')
  wrapper.unmount()
})
