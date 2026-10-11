import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import SystemMcpSettings from './SystemMcpSettings.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { setLocale } from '@/composables/useI18n'
const api = vi.hoisted(() => ({ settings: vi.fn(), requests: vi.fn(), save: vi.fn(), confirm: vi.fn(), reject: vi.fn() }))
vi.mock('@/api/systemMcp', () => ({ systemMcpApi: api }))
const proposal = { id: 'one', command: { tool: 'mcp_request_server_change', server_key: 'weather', enabled: false, expected_revision: 7 }, expires_at: 1791734700, status: 'pending' }
beforeEach(() => {
  vi.clearAllMocks()
  setLocale('en')
  api.settings.mockResolvedValue({ enabled: true, revision: 3, grants: { system_read: true, mcp_read: true, mcp_discover: false, mcp_manage_request: false } })
  api.requests.mockResolvedValue([proposal])
  api.confirm.mockResolvedValue({ ...proposal, status: 'applied' })
})
it('displays the fixed target/effect/revision and requires a separate confirmation click', async () => {
  const wrapper = mount(SystemMcpSettings, { global: { stubs: { ConfirmDialog: true } } })
  await flushPromises()
  expect(wrapper.text()).toContain('weather')
  expect(wrapper.text()).toContain('Disabled')
  expect(wrapper.text()).toContain('Expected revision: 7')
  expect(api.confirm).not.toHaveBeenCalled()
  const confirm = wrapper.findAll('button').find(button => button.text() === 'Confirm')!
  await confirm.trigger('click')
  expect(api.confirm).not.toHaveBeenCalled()
  wrapper.getComponent(ConfirmDialog).vm.$emit('confirm')
  await flushPromises()
  expect(api.confirm).toHaveBeenCalledExactlyOnceWith('one')
  expect(wrapper.text()).toContain('Applied')
  wrapper.unmount()
})
it('saves all grants with the displayed revision', async () => {
  api.save.mockResolvedValue({ enabled: true, revision: 4, grants: { system_read: true, mcp_read: true, mcp_discover: true, mcp_manage_request: false } })
  const wrapper = mount(SystemMcpSettings, { global: { stubs: { ConfirmDialog: true } } })
  await flushPromises()
  await wrapper.findAll('input[type="checkbox"]')[2]!.setValue(true)
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  expect(api.save).toHaveBeenCalledWith(expect.objectContaining({ mcp_discover: true }), 3)
  wrapper.unmount()
})
