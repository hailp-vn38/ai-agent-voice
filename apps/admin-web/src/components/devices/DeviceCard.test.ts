import { mount, RouterLinkStub } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import type { Device, AgentTemplate } from '@/domain/admin'
import { setLocale } from '@/composables/useI18n'
import DeviceCard from './DeviceCard.vue'

const device: Device = {
  id: '5', agentId: 'home', name: 'test', deviceId: '67:28:43:1D:95:90',
  description: 'Living room', status: 'online', lastSeen: '',
}
const template: AgentTemplate = {
  id: 'default', name: 'Default voice', language: 'vi-VN', description: '', prompt: '',
  providerBindings: {}, createdAt: '', updatedAt: '',
}

function render() {
  return mount(DeviceCard, {
    props: { device, effectiveTemplate: template, agentName: 'home', showAgent: true, detailLink: true },
    global: {
      stubs: {
        RouterLink: RouterLinkStub,
        ActionMenu: { template: '<div><slot /></div>' },
        MenuItem: {
          emits: ['select'],
          template: '<button data-test-delete @click="$emit(\'select\')"><slot /></button>',
        },
      },
    },
  })
}

describe('shared DeviceCard', () => {
  beforeEach(() => setLocale('en'))

  it('links the entire card to hardware-ID Device Detail, preserving visual metadata', () => {
    const wrapper = render()
    expect(wrapper.get('[data-device-open]').attributes('aria-label')).toContain('test')
    expect(wrapper.findComponent(RouterLinkStub).props('to')).toEqual({
      name: 'device-detail',
      params: { deviceId: device.deviceId },
    })
    expect(wrapper.text()).toContain('home')
    expect(wrapper.text()).toContain('Default voice')
    expect(wrapper.text()).toContain('Connection permitted')
    expect(wrapper.text()).not.toContain('Online')
  })

  it('keeps edit and delete outside the navigational overlay', async () => {
    const wrapper = render()
    expect(wrapper.get('[data-device-open]').find('button').exists()).toBe(false)
    await wrapper.get('[data-device-edit]').trigger('click')
    await wrapper.get('[data-test-delete]').trigger('click')
    expect(wrapper.emitted('edit')).toEqual([[device]])
    expect(wrapper.emitted('delete')).toEqual([[device]])
  })
})
