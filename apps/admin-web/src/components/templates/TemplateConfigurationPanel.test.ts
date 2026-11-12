import { DOMWrapper, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'
import TemplateConfigurationPanel from './TemplateConfigurationPanel.vue'

const template: AgentTemplate = { id: 'home', name: 'Home', language: 'vi', description: '', prompt: 'Keep it brief.', providerBindings: {}, createdAt: '', updatedAt: '' }
const mounted: ReturnType<typeof mount>[] = []
beforeEach(() => setLocale('en'))
afterEach(() => { mounted.splice(0).forEach(wrapper => wrapper.unmount()); document.body.innerHTML = '' })

it.each(['agent', 'template'] as const)('offers actions appropriate to the %s context', async context => {
  const wrapper = mount(TemplateConfigurationPanel, { attachTo: document.body, props: { template, providerCount: 0, agentCount: 1, deviceCount: 0, context } })
  mounted.push(wrapper)
  await wrapper.get('button[aria-haspopup="menu"]').trigger('click')
  const items = new DOMWrapper(document.body).findAll('[role="menuitem"]')
  const labels = items.map(item => item.text())
  expect(labels.includes('View template')).toBe(context === 'agent')
  expect(labels.includes('Remove from agent')).toBe(context === 'agent')
  expect(labels).toContain('Link to agent')
  expect(items.find(item => item.text() === 'Delete template')!.attributes('disabled')).toBeDefined()
  await items.find(item => item.text() === 'Edit template')!.trigger('click')
  expect(wrapper.emitted('editTemplate')).toHaveLength(1)
})
