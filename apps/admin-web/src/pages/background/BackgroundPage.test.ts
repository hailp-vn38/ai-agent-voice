import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import BackgroundPage from './BackgroundPage.vue'

const { navigateMock, requestMock } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  requestMock: vi.fn(),
}))

vi.mock('vue-router', () => ({ useRouter: () => ({ push: navigateMock }) }))
vi.mock('@/api/client', async importOriginal => ({
  ...(await importOriginal<typeof import('@/api/client')>()),
  requestJson: requestMock,
}))

const fixtureSettings = () => ({
  runtime_ready: true,
  enabled: true,
  orchestrator_llm_provider: 'orchestrator:default',
  worker_llm_provider: 'orchestrator:default',
  revision: 1,
  llm_providers: [
    {
      ref: 'orchestrator:default',
      name: 'gpt-4o-mini',
      source: 'config',
      adapter: 'openai',
      model: 'gpt-4o-mini',
      max_tokens: 4096,
      context_limit_tokens: 64000,
    },
    {
      ref: 'llm_deepseek',
      name: 'DeepSeek R1',
      source: 'database',
      adapter: 'openai',
      model: 'deepseek-reasoner',
      max_tokens: 16384,
      context_limit_tokens: 128000,
      reasoning_effort: 'high',
      temperature_percent: 60,
    },
  ],
  agents: [
    {
      id: 'daily_root',
      flow_id: 'daily',
      mode: 'primary',
      enabled: true,
      provider_ref: null,
      effective_provider_ref: 'orchestrator:default',
      effective_model: 'gpt-4o-mini',
      steps: 15,
      default_steps: 15,
      allowed_tools: ['subagent'],
      available_tools: ['subagent'],
      allowed_skills: [],
      available_skills: [],
      allowed_subagents: ['wiki_researcher'],
      description: 'Daily root coordinator',
      default_description: 'Daily root coordinator',
      system_prompt: 'You are daily_root.',
      default_system_prompt: 'You are daily_root.',
    },
    {
      id: 'wiki_researcher',
      flow_id: 'daily',
      mode: 'subagent',
      enabled: true,
      provider_ref: null,
      effective_provider_ref: 'orchestrator:default',
      effective_model: 'gpt-4o-mini',
      steps: 12,
      default_steps: 12,
      allowed_tools: ['skill', 'wiki_tree', 'wiki_query', 'wiki_get'],
      available_tools: ['skill', 'wiki_tree', 'wiki_query', 'wiki_get'],
      allowed_skills: ['wiki-query'],
      available_skills: ['wiki-query', 'wiki-taxonomy'],
      allowed_subagents: [],
      description: 'Read-only Wiki researcher',
      default_description: 'Read-only Wiki researcher',
      system_prompt: 'You are wiki_researcher.',
      default_system_prompt: 'You are wiki_researcher.',
    },
  ],
})

describe('BackgroundPage subagent & LLM provider configuration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setLocale('en')
    requestMock.mockImplementation((path: string) => {
      if (path === '/api/admin/background/settings') {
        return Promise.resolve(fixtureSettings())
      }
      return Promise.resolve({ items: [], total: 0, page: 1, page_size: 30 })
    })
  })

  it('renders Flow 1 (daily_root) and Flow 2 (wiki_root) titles, selects LLM provider per subagent, and saves overrides', async () => {
    const wrapper = mount(BackgroundPage, {
      global: {
        stubs: {
          Teleport: true,
        },
      },
    })
    await flushPromises()

    expect(wrapper.text()).toContain(
      'Flow 1 — Information Collection & Multi-Functional Assistant (daily_root)',
    )
    expect(wrapper.text()).toContain('daily_root')
    expect(wrapper.text()).toContain('wiki_researcher')
    expect(wrapper.text()).toContain('DeepSeek R1')
    expect(wrapper.text()).not.toContain('Execution Flow')
    expect(wrapper.text()).not.toContain('Runtime Capacity')

    // Open Configure modal for wiki_researcher (second Configure button)
    const configButtons = wrapper
      .findAll('button')
      .filter(b => b.text().includes('Configure'))
    expect(configButtons.length).toBe(2)
    await configButtons[1]!.trigger('click')
    await flushPromises()

    // Edit steps and system prompt inside the modal
    const numberInput = wrapper.get('input[type="number"]')
    await numberInput.setValue(8)
    const textarea = wrapper.get('textarea')
    await textarea.setValue('Custom researcher system prompt')

    const applyButton = wrapper
      .findAll('button')
      .find(b => b.text().includes('Apply to draft'))!
    await applyButton.trigger('click')
    await flushPromises()

    // Click Save changes
    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        revision: 2,
      }),
    )
    const saveButton = wrapper
      .findAll('button')
      .find(b => b.text().includes('Save changes'))!
    await saveButton.trigger('click')
    await flushPromises()

    const putCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(putCall).toBeDefined()
    const body = JSON.parse(String(putCall![1].body))
    expect(body.agents.wiki_researcher.steps).toBe(8)
    expect(body.agents.wiki_researcher.system_prompt).toBe(
      'Custom researcher system prompt',
    )
  })
})
