import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import KnowledgeSettingsPage from './KnowledgeSettingsPage.vue'

const { navigateMock, requestMock } = vi.hoisted(() => ({ navigateMock: vi.fn(), requestMock: vi.fn() }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: navigateMock }) }))
vi.mock('@/api/client', async importOriginal => ({ ...await importOriginal<typeof import('@/api/client')>(), requestJson: requestMock }))
const settings = () => ({ enabled: false, analysis_provider_ref: null, publication_policy: 'review_all', trigger_mode: 'manual', max_candidates_per_source: 5, revision: 1, runtime_ready: true,
  llm_providers: [{ reference: 'analysis', name: 'Analysis', runtime_ready: true, structured_qualified: true }, { reference: 'unsupported', name: 'Unsupported', runtime_ready: false, structured_qualified: false }] })
beforeEach(() => { vi.clearAllMocks(); setLocale('en'); requestMock.mockResolvedValue(settings()) })

it('loads persisted settings and saves only Knowledge configuration with CAS', async () => {
  const wrapper = mount(KnowledgeSettingsPage)
  await flushPromises()
  expect(wrapper.get('h1').text()).toBe('Knowledge settings')
  expect(requestMock).toHaveBeenCalledWith('/api/admin/knowledge/settings')
  expect(wrapper.get('option[value="unsupported"]').attributes('disabled')).toBeDefined()
  await wrapper.get('select[name="analysis_provider_ref"]').setValue('analysis')
  await wrapper.get('input[name="enabled"]').setValue(true)
  requestMock.mockResolvedValueOnce({ ...settings(), enabled: true, analysis_provider_ref: 'analysis', revision: 2 })
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  const [path, init, options] = requestMock.mock.calls.at(-1)!
  expect(path).toBe('/api/admin/knowledge/settings')
  expect(init.method).toBe('PUT')
  expect(JSON.parse(init.body)).toEqual({ enabled: true, analysis_provider_ref: 'analysis', publication_policy: 'review_all', trigger_mode: 'manual', max_candidates_per_source: 5 })
  expect(options).toEqual({ revision: 1 })
  expect(wrapper.text()).toContain('Settings saved')
  wrapper.unmount()
})
it('keeps save failures visible and preserves the existing back navigation', async () => {
  const wrapper = mount(KnowledgeSettingsPage)
  await flushPromises()
  requestMock.mockRejectedValueOnce(new Error('conflict'))
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).not.toBe('')
  await wrapper.get('button[aria-label="Back to Knowledge"]').trigger('click')
  expect(navigateMock).toHaveBeenCalledExactlyOnceWith('/knowledge')
  wrapper.unmount()
})

it('explicitly opts into native Tools without inheriting legacy publication fields', async () => {
  requestMock.mockResolvedValueOnce({ ...settings(), llm_providers: [{ reference: 'tools', name: 'Tools only', runtime_ready: false, structured_qualified: false, tool_calling: true, availability: 'cold' }] })
  const wrapper = mount(KnowledgeSettingsPage)
  await flushPromises()
  await wrapper.get('select[name="execution_version"]').setValue('2')
  expect(wrapper.find('select[name="publication_policy"]').exists()).toBe(false)
  expect(wrapper.get('option[value="tools"]').attributes('disabled')).toBeUndefined()
  await wrapper.get('select[name="analysis_provider_ref"]').setValue('tools')
  await wrapper.get('input[name="max_pages_per_source"]').setValue(3)
  await wrapper.get('form').trigger('submit')
  await flushPromises()
  expect(JSON.parse(requestMock.mock.calls.at(-1)![1].body)).toEqual({ execution_version: 2, enabled: false, analysis_provider_ref: 'tools', trigger_mode: 'manual', max_pages_per_source: 3 })
  wrapper.unmount()
})
