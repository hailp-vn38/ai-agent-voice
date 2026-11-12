import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { setLocale } from '@/composables/useI18n'
import AssistantDeviceSettings from './AssistantDeviceSettings.vue'

vi.mock('@/api/assistant', () => ({ assistantApi: { settings: vi.fn(), save: vi.fn() } }))
vi.mock('@/stores/assistant', () => ({ useAssistantStore: () => ({ open: vi.fn() }) }))
vi.mock('@/stores/admin', () => ({
  useAdminStore: () => ({
    agents: [{ id: 'home', name: 'home' }],
    getTemplatesForAgent: () => [],
    getTemplate: () => undefined,
    getDefaultTemplate: () => undefined,
    getProvider: () => undefined,
  }),
}))

const configured: AssistantSettings = {
  configured: true, enabled: true, device_id: 'web_assistant_1', agent_key: 'home', template_key: null,
  memory_mode: 'read', revision: 7, active_connections: 0,
}

function mountSettings() {
  return mount(AssistantDeviceSettings, { global: { stubs: { RouterLink: true, ConfirmDialog: true } } })
}

describe('AssistantDeviceSettings', () => {
  beforeEach(() => {
    setLocale('en')
    vi.clearAllMocks()
  })

  it('saves the Virtual Device Memory mode through its own settings route, since PATCH refuses it', async () => {
    vi.mocked(assistantApi.settings).mockResolvedValue(configured)
    vi.mocked(assistantApi.save).mockResolvedValue({ ...configured, memory_mode: 'read_write', revision: 8 })
    const wrapper = mountSettings()
    await flushPromises()
    expect(wrapper.get('[data-testid="assistant-connection-state"]').text()).toBe('No active connection')
    const select = wrapper.get('[data-assistant-memory-mode]')
    expect((select.element as HTMLSelectElement).value).toBe('read')

    const submit = wrapper.get('button[type="submit"]')
    expect(submit.attributes('disabled')).toBeDefined()
    await select.setValue('read_write')
    expect(submit.attributes('disabled')).toBeUndefined()

    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(assistantApi.save).toHaveBeenCalledWith(
      { agent_key: 'home', template_key: null, enabled: true, memory_mode: 'read_write' },
      7,
    )
    wrapper.unmount()
  })

  it('starts at off when the Virtual Device is not configured yet', async () => {
    const { memory_mode: _absent, ...unconfigured } = configured
    vi.mocked(assistantApi.settings).mockResolvedValue({ ...unconfigured, configured: false, revision: null })
    const wrapper = mountSettings()
    await flushPromises()
    expect((wrapper.get('[data-assistant-memory-mode]').element as HTMLSelectElement).value).toBe('off')
    wrapper.unmount()
  })
})
