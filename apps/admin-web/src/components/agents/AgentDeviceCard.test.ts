import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'
import AgentDeviceCard from './AgentDeviceCard.vue'

const device: Device = {
  id: '1',
  agentId: 'home',
  name: 'Voice node',
  deviceId: '67:28:43:1D:95:90',
  description: 'ESP32 voice device',
  status: 'online',
  lastSeen: '',
}

const template: AgentTemplate = {
  id: 'test', name: 'Default Voice', description: '', language: 'vi-VN', prompt: '',
  providerBindings: {}, createdAt: '', updatedAt: '',
}

function render(item: Device = device) {
  return mount(AgentDeviceCard, {
    props: { device: item, effectiveTemplate: template },
    global: {
      stubs: {
        ActionMenu: { template: '<div><slot /></div>' },
        MenuItem: {
          emits: ['select'],
          template: '<button v-bind="$attrs" @click="$emit(\'select\')"><slot /></button>',
        },
      },
    },
  })
}

describe('AgentDeviceCard', () => {
  beforeEach(() => setLocale('en'))

  it('displays admission instead of mislabeling enabled as live online status', () => {
    const wrapper = render()
    expect(wrapper.get('[data-device-admission]').text()).toBe('Connection permitted')
    expect(wrapper.text()).not.toContain('Online')
    expect(wrapper.text()).toContain('Default Voice')
    expect(wrapper.text()).toContain('Default')
    expect(wrapper.find('svg[viewBox="0 0 48 48"]').exists()).toBe(true)
  })

  it('labels disabled admission and template overrides correctly', () => {
    const wrapper = render({ ...device, status: 'offline', templateId: 'test' })
    expect(wrapper.get('[data-device-admission]').text()).toBe('Connection disabled')
    expect(wrapper.text()).toContain('Override')
  })

  it('preserves independent edit and delete actions', async () => {
    const wrapper = render()
    await wrapper.get('[data-device-edit]').trigger('click')
    expect(wrapper.emitted('edit')).toEqual([[device]])
    await wrapper.get('[data-device-delete]').trigger('click')
    expect(wrapper.emitted('delete')).toEqual([[device]])
  })

  it('copies the actual, unmasked device ID on demand', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true, value: { writeText },
    })
    const wrapper = render()
    await wrapper.get('[data-device-copy]').trigger('click')
    await Promise.resolve()
    expect(writeText).toHaveBeenCalledWith(device.deviceId)
    expect(wrapper.get('[data-device-copy]').attributes('aria-label')).toBe('Device ID copied')
  })
})
