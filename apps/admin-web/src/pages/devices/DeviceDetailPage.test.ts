import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'

import { setLocale } from '@/composables/useI18n'
import { devicesApi } from '@/api/devices'
import type { AdminDevice } from '@/api/types/devices'
import DeviceDetailPage from './DeviceDetailPage.vue'

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
  devicesApi: { get: vi.fn(), update: vi.fn(), remove: vi.fn() },
}))

function sample(id: string): AdminDevice {
  return {
    id: 1, device_id: id, agent_key: 'home', template_key: null,
    name: id, description: null, enabled: 1, revision: 5, metadata_json: null,
    created_at: 1_760_000_000, updated_at: 1_760_000_100,
  }
}

describe('DeviceDetailPage', () => {
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
          ActionMenu: { template: '<div data-detail-actions><slot /></div>' },
          MenuItem: {
            emits: ['select'],
            template: '<button type="button" data-device-delete-option @click="$emit(\'select\')"><slot /></button>',
          },
        },
      },
    })
    await flushPromises()

    expect(wrapper.get('[data-detail-actions]').exists()).toBe(true)
    expect(wrapper.find('section.studio-panel h2').text()).toBe('Device information')
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
})
