import { flushPromises, mount } from '@vue/test-utils'
import { reactive } from 'vue'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import ProviderDetailPage from './ProviderDetailPage.vue'

const route = reactive({ params: { key: 'llm_primary' } })
const push = vi.fn()
const replace = vi.fn()
vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => ({ push, replace }),
  RouterLink: { props: ['to'], template: '<a><slot /></a>' },
}))
const api = vi.hoisted(() => ({
  get: vi.fn(),
  templates: vi.fn(),
  prepare: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  testLlm: vi.fn(),
  testAsr: vi.fn(),
  testVad: vi.fn(),
  testTts: vi.fn(),
}))
vi.mock('@/api/providers', () => ({ providersApi: api }))
const adapters = vi.hoisted(() => ({ get: vi.fn() }))
vi.mock('@/api/provider-adapters', () => ({ providerAdaptersApi: adapters }))
const refreshAll = vi.fn()
vi.mock('@/stores/admin', () => ({
  useAdminStore: () => ({ providers: [], refreshAll }),
}))

const fixture = {
  id: 3,
  key: 'llm_primary',
  name: 'OpenAI Primary',
  type: 'llm',
  adapter: 'openai',
  config_json: JSON.stringify({
    model: 'gpt-4o-mini',
    base_url: 'https://user:secret@example.test/v1?token=private',
    api_key: 'must-not-display',
    temperature: 0.5,
  }),
  enabled: 1,
  revision: 4,
  created_at: 1,
  updated_at: 2,
  credential_env: 'OPENAI_API_KEY',
  runtime_status: 'not_loaded',
  runtime_matches_desired: false,
  requires_restart: false,
  runtime: {
    desired_revision: 4,
    desired_state: 'cold',
    ready_revisions: [],
    can_prepare: true,
    failure_code: null,
  },
}

function render() {
  return mount(ProviderDetailPage, {
    global: {
      stubs: {
        ProviderTestPanel: { template: '<div data-test-panel>Manual test, no persisted history</div>' },
        ProviderFormModal: true,
        ConfirmDialog: true,
      },
    },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  route.params.key = 'llm_primary'
  api.get.mockResolvedValue({ ...fixture })
  api.templates.mockResolvedValue({ items: [], total: 0, page: 1, page_size: 50 })
  adapters.get.mockResolvedValue({
    adapter: 'openai',
    type: 'llm',
    config_schema: { fields: [
      { key: 'temperature', type: 'integer', label: 'Temperature' },
      { key: 'api_key', type: 'string', label: 'API key' },
    ] },
  })
  api.prepare.mockResolvedValue({})
  refreshAll.mockResolvedValue(undefined)
})
afterEach(() => {
  vi.clearAllMocks()
})

it('loads a direct link without running tests or preparing runtime, and redacts secrets', async () => {
  const wrapper = render()
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith('llm_primary', expect.any(AbortSignal))
  expect(api.templates).toHaveBeenCalledWith('llm_primary', 1, 50, expect.any(AbortSignal))
  expect(api.testLlm).not.toHaveBeenCalled()
  expect(api.prepare).not.toHaveBeenCalled()
  expect(wrapper.text()).toContain('OpenAI Primary')
  expect(wrapper.text()).toContain('cold')
  expect(wrapper.text()).toContain('gpt-4o-mini')
  expect(wrapper.text()).toContain('https://***@example.test/v1?token=***')
  expect(wrapper.text()).not.toContain('must-not-display')
  expect(wrapper.text()).not.toContain('private')
  expect(wrapper.text()).not.toContain('Last test result')
  wrapper.unmount()
})

it('fetches the new provider when the key changes and does not carry a test result between pages', async () => {
  const wrapper = render()
  await flushPromises()
  route.params.key = 'llm_secondary'
  api.get.mockResolvedValueOnce({ ...fixture, key: 'llm_secondary', name: 'Secondary LLM' })
  await flushPromises()
  expect(api.get).toHaveBeenLastCalledWith('llm_secondary', expect.any(AbortSignal))
  expect(wrapper.text()).toContain('Secondary LLM')
  expect(wrapper.text()).not.toContain('OpenAI Primary')
  wrapper.unmount()
})

it('enables Prepare only on explicit click and prevents deletion while bound', async () => {
  api.templates.mockResolvedValue({ items: [{ key: 'default', name: 'Default', provider_type: 'llm', enabled: true }], total: 1 })
  const wrapper = render()
  await flushPromises()
  const prepareButton = wrapper.findAll('button').find((button) => button.text().includes('Prepare runtime'))
  expect(prepareButton).toBeDefined()
  await prepareButton!.trigger('click')
  await flushPromises()
  expect(api.prepare).toHaveBeenCalledWith('llm_primary')
  expect(api.remove).not.toHaveBeenCalled()
  expect(wrapper.text()).toContain('Default')
  wrapper.unmount()
})
