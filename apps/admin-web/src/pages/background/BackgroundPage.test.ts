import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { setLocale } from '@/composables/useI18n'
import BackgroundAgentDetailPage from './BackgroundAgentDetailPage.vue'
import BackgroundPage from './BackgroundPage.vue'

const { navigateMock, requestMock, routeParams } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  requestMock: vi.fn(),
  routeParams: { agentId: 'wiki_researcher' },
}))

const reactiveAgentId = ref('wiki_researcher')

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: navigateMock }),
  useRoute: () => ({
    get params() {
      return { agentId: reactiveAgentId.value || routeParams.agentId }
    },
    query: {},
  }),
}))
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
    {
      ref: 'llm_flux_image',
      name: 'Flux Image Gen',
      source: 'database',
      adapter: 'openai',
      model: 'flux-1-dev',
      tags: ['image'],
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
      allowed_subagents: ['wiki_researcher', 'media_creator'],
      description: 'Daily root coordinator',
      default_description: 'Daily root coordinator',
      system_prompt: 'You are daily_root.',
      default_system_prompt: 'You are daily_root.',
    },
    {
      id: 'wiki_researcher',
      flow_id: 'daily',
      parent_id: 'daily_root',
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
      default_allowed_skills: ['wiki-query'],
      available_skills: ['wiki-query', 'wiki-taxonomy'],
      allowed_subagents: [],
      description: 'Read-only Wiki researcher',
      default_description: 'Read-only Wiki researcher',
      system_prompt: '# Wiki Researcher\n\nYou are wiki_researcher.',
      default_system_prompt: '# Wiki Researcher\n\nYou are wiki_researcher.',
    },
    {
      id: 'media_creator',
      flow_id: 'daily',
      parent_id: 'daily_root',
      mode: 'subagent',
      enabled: true,
      provider_ref: null,
      effective_provider_ref: 'orchestrator:default',
      effective_model: 'gpt-4o-mini',
      tool_capability_tags: {
        image_create: 'image',
        video_create: 'video',
        tts_synthesize: 'audio',
      },
      tool_providers: {},
      effective_tool_providers: {
        image_create: 'llm_flux_image',
        video_create: null,
        tts_synthesize: null,
      },
      steps: 12,
      default_steps: 12,
      allowed_tools: ['image_create', 'video_create', 'tts_synthesize', 'artifact_create'],
      available_tools: ['skill', 'image_create', 'video_create', 'tts_synthesize', 'artifact_create'],
      allowed_skills: [],
      available_skills: ['daily-synthesis-staging'],
      allowed_subagents: [],
      description: 'Generates PNG/SVG/MP4/TTS artifacts',
      default_description: 'Generates PNG/SVG/MP4/TTS artifacts',
      system_prompt: 'You are media_creator.',
      default_system_prompt: 'You are media_creator.',
    },
  ],
})

describe('BackgroundPage & BackgroundAgentDetailPage subagent configuration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setLocale('en')
    reactiveAgentId.value = 'wiki_researcher'
    requestMock.mockImplementation((path: string) => {
      if (path === '/api/admin/background/settings') {
        return Promise.resolve(fixtureSettings())
      }
      return Promise.resolve({ items: [], total: 0, page: 1, page_size: 30 })
    })
  })

  it('renders Flow 1 (daily_root) on BackgroundPage and navigates to dedicated subagent config page on Configure', async () => {
    const wrapper = mount(BackgroundPage)
    await flushPromises()

    expect(wrapper.text()).toContain(
      'Flow 1 — Information Collection & Multi-Functional Assistant (daily_root)',
    )
    expect(wrapper.text()).toContain('daily_root')
    expect(wrapper.text()).toContain('wiki_researcher')
    expect(wrapper.text()).toContain('DeepSeek R1')
    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)

    await wrapper.get('[data-detail-back]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs' })

    const configButtons = wrapper
      .findAll('button')
      .filter(b => b.text().includes('Configure'))
    expect(configButtons.length).toBe(3)
    await configButtons[1]!.trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-agent',
      params: { agentId: 'wiki_researcher' },
    })
  })

  it('renders dedicated BackgroundAgentDetailPage for wiki_researcher and media_creator and saves overrides', async () => {
    reactiveAgentId.value = 'wiki_researcher'
    const wrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)
    expect(wrapper.text()).toContain('wiki_researcher')
    expect(wrapper.text()).toContain('Sub-Agent')
    expect(wrapper.text()).toContain('Coordinator: daily_root')

    // Back button returns to jobs-settings
    await wrapper.get('[data-detail-back]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings' })

    // Edit steps, description, and system prompt on the dedicated page
    const stepsInput = wrapper.get('input[data-agent-steps]')
    await stepsInput.setValue(8)
    const descInput = wrapper.get('textarea[data-agent-description]')
    await descInput.setValue('Custom Wiki & Web researcher description')
    const promptInput = wrapper.get('textarea[data-agent-system-prompt]')
    await promptInput.setValue('# Custom Prompt\n\nCustom researcher system prompt')

    // Switch to Preview mode and verify rendered Markdown
    const previewButton = wrapper
      .findAll('button')
      .find(b => b.text().includes('Preview'))!
    await previewButton.trigger('click')
    expect(wrapper.get('[data-agent-prompt-preview]').html()).toContain('Custom Prompt')

    // Save changes for wiki_researcher
    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        revision: 2,
      }),
    )
    const saveButton = wrapper.get('[data-save-agent-config]')
    await saveButton.trigger('click')
    await flushPromises()

    const putCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(putCall).toBeDefined()
    const body = JSON.parse(String(putCall![1].body))
    expect(body.agents.wiki_researcher.steps).toBe(8)
    expect(body.agents.wiki_researcher.description).toBe(
      'Custom Wiki & Web researcher description',
    )
    expect(body.agents.wiki_researcher.system_prompt).toBe(
      '# Custom Prompt\n\nCustom researcher system prompt',
    )

    // Switch to media_creator and configure capability-routed tool provider
    reactiveAgentId.value = 'media_creator'
    const mediaWrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    const imageToolSelect = mediaWrapper.get('select[data-tool-provider="image_create"]')
    expect(imageToolSelect.text()).toContain('Flux Image Gen')
    await imageToolSelect.setValue('llm_flux_image')

    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        revision: 3,
      }),
    )
    await mediaWrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()

    const mediaPutCalls = requestMock.mock.calls.filter(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(mediaPutCalls.length).toBe(2)
    const mediaBody = JSON.parse(String(mediaPutCalls[1]![1].body))
    expect(mediaBody.agents.media_creator.tool_providers).toEqual({
      image_create: 'llm_flux_image',
    })
  })
})
