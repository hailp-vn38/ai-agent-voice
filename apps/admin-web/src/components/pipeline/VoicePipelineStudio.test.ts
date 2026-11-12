import { DOMWrapper, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import type { AgentTemplate, ProviderInstance } from '@/domain/admin'
import VoicePipelineStudio from './VoicePipelineStudio.vue'

const providers: ProviderInstance[] = (['vad', 'asr', 'llm', 'tts'] as const).map((type, i) => ({
  id: `p${i}`, name: `Provider ${i}`, type,
  adapter: 'test', model: 'model', description: '', status: 'ready',
}))
const template: AgentTemplate = {
  id: 'voice', name: 'Voice', description: '', language: 'vi-VN', prompt: 'Hello',
  providerBindings: { vad: 'p0', asr: 'p1', llm: 'p2', tts: 'p3' }, createdAt: '', updatedAt: '',
}
const mounted: ReturnType<typeof mount>[] = []
function render(item = template, catalog = providers) {
  const wrapper = mount(VoicePipelineStudio, { attachTo: document.body, props: { template: item, providers: catalog, sharedTemplates: () => [] } })
  mounted.push(wrapper)
  return wrapper
}
beforeEach(() => setLocale('en'))
afterEach(() => { mounted.splice(0).forEach(wrapper => wrapper.unmount()); document.body.innerHTML = '' })

describe('VoicePipelineStudio', () => {
  it('opens the provider belonging to the selected stage and allows pausing the illustration', async () => {
    const wrapper = render()
    await wrapper.get('[data-stage="llm"] .stage-provider article > button').trigger('click')
    expect(wrapper.emitted('openProvider')).toEqual([[providers[2]]])
    await wrapper.get('button[aria-label="Pause illustrations"]').trigger('click')
    expect(wrapper.get('button[aria-label="Resume illustrations"]').attributes('aria-pressed')).toBe('true')
    expect(wrapper.classes()).toContain('is-paused')
  })

  it('offers replacement for a missing binding without presenting disabled or missing stages as ready', async () => {
    const catalog = providers.map(provider => provider.type === 'tts' ? { ...provider, status: 'disabled' as const } : provider)
    const wrapper = render({ ...template, providerBindings: { ...template.providerBindings, asr: 'deleted' } }, catalog)
    expect(wrapper.get('.pipeline-coverage').text()).toBe('3 of 4 slots bound')
    expect(wrapper.get('[data-stage="asr"]').attributes('data-ready')).toBe('false')
    expect(wrapper.get('[data-stage="tts"]').attributes('data-ready')).toBe('false')
    expect(wrapper.get('[data-stage="llm"] .stage-connector').attributes('data-connected')).toBe('false')
    await wrapper.get('[data-stage="asr"] .stage-provider button').trigger('click')
    const menu = new DOMWrapper(document.body)
    const option = menu.findAll('[role="menuitem"]').find(item => item.text().includes('Provider 1'))
    expect(option).toBeDefined()
    await option!.trigger('click')
    expect(wrapper.emitted('select')).toEqual([['asr', 'p1']])
  })

  it('refreshes stage bindings when the selected template changes', async () => {
    const wrapper = render()
    await wrapper.setProps({ template: { ...template, id: 'empty', providerBindings: {} } })
    expect(wrapper.get('.pipeline-coverage').text()).toBe('0 of 4 slots bound')
    for (const stage of wrapper.findAll('[data-stage]')) {
      expect(stage.attributes('data-configured')).toBe('false')
      expect(stage.attributes('data-ready')).toBe('false')
    }
    expect(wrapper.text()).not.toContain('Provider 2')
  })
})
