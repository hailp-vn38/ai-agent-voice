import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { createMemoryHistory, createRouter, RouterLink } from 'vue-router'

import { setLocale } from '@/composables/useI18n'
import type { ProviderInstance } from '@/domain/admin'
import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'
import ProviderDetailModal from './ProviderDetailModal.vue'

const provider: ProviderInstance = {
  id: 'llm_home', name: 'Home LLM', type: 'llm', adapter: 'openai', model: '',
  description: '', status: 'ready', desiredRevision: 7,
  configJson: { model: 'chat-model', base_url: 'https://user:secret@example.test/v1?api_key=private-token' },
  credential: { id: 'cred', masked_key: 'sk-***abcd', key_version: 1, status: 'active' },
}

async function page(extra: { source?: 'config'; focusTest?: boolean } = {}) {
  setLocale('en')
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/', component: { template: '<div />' } },
    { path: '/templates/:templateId', name: 'template-detail', component: { template: '<div />' } },
  ] })
  await router.push('/')
  const wrapper = mount(ProviderDetailModal, {
    props: { modelValue: true, provider: { ...provider, source: extra.source }, focusTest: extra.focusTest,
      usage: [{ templateId: 'home', templateName: 'Home voice', language: 'vi', agentNames: ['Home'] }],
    },
    global: { plugins: [router], stubs: {
      BaseModal: { template: '<div><slot /><slot name="footer" /></div>' }, ProviderTestPanel: true,
    } },
  })
  await flushPromises()
  return { wrapper, router }
}

describe('Provider instance inspection', () => {
  it('shows the saved model and masked credentials, and routes to a bound template', async () => {
    const { wrapper, router } = await page()
    try {
      expect(wrapper.text()).toContain('chat-model')
      expect(wrapper.text()).toContain('sk-***abcd')
      expect(wrapper.text()).not.toContain('private-token')
      expect(wrapper.text()).not.toContain('user:secret')
      await wrapper.get('nav').findAll('button')[1]!.trigger('click')
      expect(wrapper.getComponent(RouterLink).isVisible()).toBe(true)
      await wrapper.getComponent(RouterLink).trigger('click')
      await flushPromises()
      expect(router.currentRoute.value.params.templateId).toBe('home')
      expect(wrapper.emitted('update:modelValue')).toEqual([[false]])
    } finally { wrapper.unmount() }
  })

  it('opens the requested test section with the saved identity and revision', async () => {
    const { wrapper } = await page({ focusTest: true })
    try {
      expect(wrapper.get('nav').findAll('button')[2]!.attributes('aria-pressed')).toBe('true')
      const test = wrapper.getComponent(ProviderTestPanel)
      expect(test.isVisible()).toBe(true)
      expect(test.props()).toMatchObject({ savedKey: provider.id, revision: 7, model: 'chat-model' })
      expect(test.props('draft')).toBeUndefined()
    } finally { wrapper.unmount() }
  })

  it('keeps server-managed instances locked while allowing saved diagnostics', async () => {
    const { wrapper } = await page({ source: 'config' })
    try {
      const edit = wrapper.findAll('button').find((item) => item.text() === 'Edit')!
      expect(edit.attributes('disabled')).toBeDefined()
      await edit.trigger('click')
      expect(wrapper.emitted('edit')).toBeUndefined()
      await wrapper.get('nav').findAll('button')[2]!.trigger('click')
      expect(wrapper.getComponent(ProviderTestPanel).isVisible()).toBe(true)
    } finally { wrapper.unmount() }
  })
})
