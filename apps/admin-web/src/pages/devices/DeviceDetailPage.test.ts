import { config, flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'

import { setLocale } from '@/composables/useI18n'
import { devicesApi } from '@/api/devices'
import type { AdminDevice } from '@/api/types/devices'
import DeviceDetailPage from './DeviceDetailPage.vue'

// shallowMount drops named slots, and the header's facts and actions live in DetailHero slots.
config.global.stubs = {
  ...config.global.stubs,
  DetailHero: { template: '<header><slot name="meta" /><slot name="actions" /><slot name="facts" /></header>' },
}

const route = reactive({ params: { deviceId: 'device-a' }, query: { edit: undefined as string | undefined } })
const push = vi.fn()
const replace = vi.fn()
vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => ({ push, replace }),
}))
const refreshAll = vi.fn()
vi.mock('@/stores/admin', () => ({
  useAdminStore: () => ({
    agents: [{ id: 'home', name: 'home' }],
    getAgent: (key: string) => key === 'home' ? { id: key, name: 'home' } : undefined,
    getTemplate: () => undefined,
    getDefaultTemplate: () => undefined,
    getTemplatesForAgent: () => [],
    refreshAll,
  }),
}))
vi.mock('@/api/devices', () => ({
  devicesApi: { speak: vi.fn(), get: vi.fn(), update: vi.fn(), remove: vi.fn() },
}))

function sample(id: string): AdminDevice {
  return {
    id: 1, device_id: id, agent_key: 'home', template_key: null,
    name: id, description: null, enabled: 1, revision: 5, metadata_json: null, memory_mode: 'off',
    created_at: 1_760_000_000, updated_at: 1_760_000_100,
  }
}

describe('DeviceDetailPage', () => {
  it('shows live presence and labels admitted direct speech as awaiting playback', async () => {
    vi.mocked(devicesApi.get).mockResolvedValue({ ...sample('device-a'), ws_online: true, direct_tts_supported: true })
    vi.mocked(devicesApi.speak).mockResolvedValue({ request_id: 'test', status: 'accepted' })
    const wrapper = shallowMount(DeviceDetailPage, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    expect(wrapper.text()).toContain('WS online')
    await wrapper.get('#direct-speech-text').setValue('hello from admin')
    await wrapper.get('form[data-direct-speech]').trigger('submit')
    await flushPromises()
    expect(devicesApi.speak).toHaveBeenCalledWith('device-a', 'hello from admin', expect.any(String))
    expect(wrapper.get('[role="status"]').text()).toContain('Accepted')
    expect(wrapper.get('[role="status"]').text()).toContain('Playback is not confirmed')
    wrapper.unmount()
  })

  beforeEach(() => {
    setLocale('en')
    route.params.deviceId = 'device-a'
    route.query.edit = undefined
    vi.clearAllMocks()
    vi.mocked(devicesApi.get).mockImplementation(async (id) => sample(id))
    vi.mocked(devicesApi.update).mockImplementation(async (id) => sample(id))
  })

  it('loads a deep link, re-fetches when navigating between hardware IDs', async () => {
    const wrapper = shallowMount(DeviceDetailPage, {
      global: { stubs: { RouterLink: true } },
    })
    await flushPromises()
    expect(devicesApi.get).toHaveBeenCalledWith('device-a', expect.any(AbortSignal))
    expect(wrapper.text()).toContain('device-a')
    route.params.deviceId = 'device-b'
    await flushPromises()
    expect(devicesApi.get).toHaveBeenLastCalledWith('device-b', expect.any(AbortSignal))
    expect(wrapper.text()).toContain('device-b')
    wrapper.unmount()
  })

  it('keeps Delete only in the header menu and requires confirmation before DELETE', async () => {
    vi.mocked(devicesApi.remove).mockResolvedValue(undefined)
    const wrapper = shallowMount(DeviceDetailPage, {
      global: {
        stubs: {
          RouterLink: true,
          DetailHeader: { template: '<header><slot name="actions" /><slot /></header>' },
          ActionMenu: { template: '<div data-detail-actions><slot /></div>' },
          MenuItem: {
            emits: ['select'],
            template: '<button type="button" data-device-delete-option @click="$emit(\'select\')"><slot /></button>',
          },
        },
      },
    })
    await flushPromises()

    expect(wrapper.find('[data-detail-actions]').exists()).toBe(true)
    expect(wrapper.findAll('section.studio-panel h2').map((heading) => heading.text())).toContain('Device information')
    expect(wrapper.findAll('[data-device-delete-option]')).toHaveLength(1)
    await wrapper.get('[data-device-delete-option]').trigger('click')
    expect(devicesApi.remove).not.toHaveBeenCalled()

    const dialog = wrapper.findComponent({ name: 'ConfirmDialog' })
    expect(dialog.props('modelValue')).toBe(true)
    dialog.vm.$emit('confirm')
    await flushPromises()

    expect(devicesApi.remove).toHaveBeenCalledWith('device-a', 5)
    expect(push).toHaveBeenCalledWith({ name: 'devices' })
    wrapper.unmount()
  })

  it('updates only admission using revision (without falsely reporting WebSocket online)', async () => {
    const wrapper = shallowMount(DeviceDetailPage, {
      global: { stubs: { RouterLink: true } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('Live presence is not available')
    await wrapper.get('[data-device-admission-switch]').trigger('click')
    await flushPromises()
    expect(devicesApi.update).toHaveBeenCalledWith('device-a', { enabled: false }, 5)
    expect(devicesApi.get).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  it.each([
    { ws_online: true, state: 'online', label: 'WS online' },
    { ws_online: false, state: 'offline', label: 'WS offline' },
    { ws_online: undefined, state: 'unknown', label: 'Live presence is not available' },
  ])('shows $state presence independently of enabled admission', async ({ ws_online, state, label }) => {
    vi.mocked(devicesApi.get).mockResolvedValue({ ...sample('device-a'), ws_online })
    const wrapper = shallowMount(DeviceDetailPage, { global: { stubs: { RouterLink: true } } })
    try {
      await flushPromises()
      const presence = wrapper.get('[data-device-presence]')
      expect(presence.attributes('data-presence')).toBe(state)
      expect(presence.text()).toContain(label)
      expect(wrapper.get('.device-admission-label').text()).toBe('Connection permitted')
    } finally { wrapper.unmount() }
  })

  it('warns that deleting a Device deletes its Memory vault', async () => {
    const wrapper = shallowMount(DeviceDetailPage, { global: { stubs: { RouterLink: true } } })
    await flushPromises()
    const dialog = wrapper.findComponent({ name: 'ConfirmDialog' })
    expect(dialog.props('description')).toMatch(/Memory vault is deleted/)
    wrapper.unmount()
  })

  it('shows the Memory mode and saves a change to it with the revision', async () => {
    route.query.edit = '1'
    vi.mocked(devicesApi.get).mockImplementation(async (id) => ({ ...sample(id), memory_mode: 'read' }))
    const wrapper = shallowMount(DeviceDetailPage, {
      global: { stubs: { RouterLink: true, BaseModal: { template: '<div><slot /></div>' } } },
    })
    await flushPromises()
    expect(wrapper.get('[data-device-memory-mode-value]').text()).toBe('Read')

    expect(wrapper.get<HTMLInputElement>('[data-device-memory-mode][value="read"]').element.checked).toBe(true)
    await wrapper.get('[data-device-memory-mode][value="read_write"]').setValue(true)
    await wrapper.get('#device-detail-edit-form').trigger('submit')
    await flushPromises()
    expect(devicesApi.update).toHaveBeenCalledWith(
      'device-a',
      expect.objectContaining({ memory_mode: 'read_write' }),
      5,
    )
    wrapper.unmount()
  })
})
