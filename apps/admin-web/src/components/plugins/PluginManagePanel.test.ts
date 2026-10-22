import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import PluginManagePanel from './PluginManagePanel.vue'
const apis = vi.hoisted(() => ({ settings: vi.fn(), list: vi.fn(), updateSettings: vi.fn() }))
vi.mock('@/api/agent-plugins', () => ({ agentPluginsApi: apis }))
vi.mock('@/api/providers', () => ({ providersApi: apis }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }))
beforeEach(() => {
  vi.resetAllMocks()
  apis.settings.mockResolvedValue({ revision: 3, values: { provider_ref: 'config:asr:previous' }, secret_set: {},
    schema: { properties: { provider_ref: { type: 'string' } }, required: ['provider_ref'] } })
  apis.list.mockResolvedValue({ items: [
    { key: 'enabled', name: 'Enabled', enabled: 1 }, { key: 'disabled', name: 'Disabled', enabled: 0 },
  ] })
})
for (const type of ['asr', 'tts']) {
  it(`selects enabled ${type} providers and saves with settings revision`, async () => {
    const wrapper = mount(PluginManagePanel, { props: { pluginKey: `core.${type}`, enabled: false, operations: [], mode: 'settings' } })
    await flushPromises()
    expect(apis.list).toHaveBeenCalledWith({ type, pageSize: 100 }, expect.any(AbortSignal))
    const select = wrapper.get('select')
    expect(select.text()).toContain('Enabled')
    expect(select.text()).not.toContain('Disabled')
    expect(select.text()).toContain('previous selection')
    await select.setValue('enabled')
    await wrapper.get('button').trigger('click')
    expect(apis.updateSettings).toHaveBeenCalledWith(`core.${type}`, { provider_ref: 'enabled' }, 3)
    wrapper.unmount()
  })
}
it('retains editable settings when the provider catalog is unavailable', async () => {
  apis.list.mockRejectedValue(new Error('offline'))
  const wrapper = mount(PluginManagePanel, { props: { pluginKey: 'core.asr', enabled: false, operations: [], mode: 'settings' } })
  await flushPromises()
  expect(wrapper.get('input').element.value).toBe('config:asr:previous')
  expect(wrapper.text()).toContain('16,000 characters')
  wrapper.unmount()
})
