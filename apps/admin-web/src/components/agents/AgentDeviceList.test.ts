import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'
import AgentDeviceList from './AgentDeviceList.vue'

const device: Device = {
  id: '1', agentId: 'home', name: 'Voice node', deviceId: 'voice-1',
  description: '', status: 'online', lastSeen: '',
}
const template: AgentTemplate = {
  id: 'test', name: 'Default Voice', description: '', language: 'vi-VN', prompt: '',
  providerBindings: {}, createdAt: '', updatedAt: '',
}

describe('AgentDeviceList grid', () => {
  beforeEach(() => setLocale('en'))

  it('renders cards in a responsive grid and passes the resolved template', () => {
    const wrapper = mount(AgentDeviceList, {
      props: {
        devices: [device, { ...device, id: '2' }],
        effectiveTemplateById: () => template,
      },
      global: {
        stubs: {
          AgentDeviceCard: {
            props: ['device', 'effectiveTemplate'],
            template: '<li data-device-card-stub>{{ device.name }} - {{ effectiveTemplate.name }}</li>',
          },
        },
      },
    })
    expect(wrapper.get('[data-device-grid]').classes()).toContain('sm:grid-cols-2')
    expect(wrapper.findAll('[data-device-card-stub]')).toHaveLength(2)
    expect(wrapper.text()).toContain('Default Voice')
    expect(wrapper.text()).toContain('Enabled / Disabled')
  })

  it('keeps a visible empty state', () => {
    const wrapper = mount(AgentDeviceList, {
      props: { devices: [], effectiveTemplateById: () => undefined },
    })
    expect(wrapper.find('[data-device-grid]').exists()).toBe(false)
    expect(wrapper.text()).toContain('No devices linked yet')
  })
})
