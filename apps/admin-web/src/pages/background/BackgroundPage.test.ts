import type { BackgroundSettings } from '@/api/types/agent-jobs'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { setLocale } from '@/composables/useI18n'
import BackgroundAgentDetailPage from './BackgroundAgentDetailPage.vue'
import BackgroundAgentsPage from './BackgroundAgentsPage.vue'
import BackgroundPage from './BackgroundPage.vue'
import BackgroundProvidersPage from './BackgroundProvidersPage.vue'
import BackgroundRuntimePage from './BackgroundRuntimePage.vue'
import BackgroundSkillDetailPage from './BackgroundSkillDetailPage.vue'
import BackgroundSkillsPage from './BackgroundSkillsPage.vue'

const { navigateMock, replaceMock, requestMock, routeParams } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  replaceMock: vi.fn(),
  requestMock: vi.fn(),
  routeParams: { agentId: 'wiki_researcher', skillId: 'wiki-query' },
}))

const reactiveAgentId = ref('wiki_researcher')
const reactiveSkillId = ref('wiki-query')
const reactiveRouteName = ref('jobs-settings-agent')

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: navigateMock, replace: replaceMock }),
  useRoute: () => ({
    get name() {
      return reactiveRouteName.value
    },
    get params() {
      return {
        agentId: reactiveAgentId.value || routeParams.agentId,
        skillId: reactiveSkillId.value || routeParams.skillId,
      }
    },
    query: {},
  }),
}))
vi.mock('@/api/client', async importOriginal => ({
  ...(await importOriginal<typeof import('@/api/client')>()),
  requestJson: requestMock,
}))

type SettingsFixture = BackgroundSettings & Required<Pick<BackgroundSettings, 'agents' | 'llm_providers' | 'skills'>>

const fixtureSettings = (): SettingsFixture => ({
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
    {
      ref: 'tts_vieneu',
      name: 'VieNeu TTS',
      source: 'database',
      adapter: 'vieneu',
      model: 'Binh',
      tags: ['tts'],
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
      allowed_subagents: ['wiki_researcher', 'media_creator', 'media_processor'],
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
      required_job_skills: ['wiki-query'],
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
      },
      tool_providers: {},
      effective_tool_providers: {
        image_create: 'llm_flux_image',
        video_create: null,
      },
      steps: 12,
      default_steps: 12,
      allowed_tools: ['image_create', 'video_create'],
      available_tools: ['skill', 'image_create', 'video_create'],
      allowed_skills: [],
      available_skills: ['daily-synthesis-staging'],
      allowed_subagents: [],
      description: 'Generates PNG/SVG/MP4 media artifacts',
      default_description: 'Generates PNG/SVG/MP4 media artifacts',
      system_prompt: 'You are media_creator.',
      default_system_prompt: 'You are media_creator.',
    },
    {
      id: 'media_processor',
      flow_id: 'daily',
      parent_id: 'daily_root',
      mode: 'subagent',
      enabled: true,
      provider_ref: null,
      effective_provider_ref: 'orchestrator:default',
      effective_model: 'gpt-4o-mini',
      tool_capability_tags: {
        tts_synthesize: 'tts',
        asr_transcribe: 'asr',
      },
      tool_providers: {},
      effective_tool_providers: {
        tts_synthesize: 'tts_vieneu',
        asr_transcribe: null,
      },
      steps: 12,
      default_steps: 12,
      allowed_tools: ['asr_transcribe', 'tts_synthesize', 'video_concat', 'artifact_create'],
      available_tools: [
        'skill',
        'asr_transcribe',
        'tts_synthesize',
        'video_concat',
        'artifact_create',
      ],
      allowed_skills: [],
      available_skills: ['daily-synthesis-staging'],
      allowed_subagents: [],
      description: 'Processes ASR, TTS, video_concat, and binary artifacts',
      default_description: 'Processes ASR, TTS, video_concat, and binary artifacts',
      system_prompt: 'You are media_processor.',
      default_system_prompt: 'You are media_processor.',
    },
  ],
  skills: [
    {
      id: 'wiki-query',
      name: 'wiki-query',
      description: 'Answer questions from the Karpathy Wiki.',
      is_builtin: true,
      is_modified: true,
      default_has_update: true,
      etag: 'etag-wiki-query',
      used_by_agents: ['wiki_researcher'],
      suggested_for_agents: ['wiki_researcher'],
    },
    {
      id: 'seo-audit',
      name: 'SEO Content Audit',
      description: 'Audit headings and links for SEO quality.',
      is_builtin: false,
      is_modified: true,
      default_has_update: false,
      etag: 'etag-seo-audit',
      used_by_agents: [],
      suggested_for_agents: [],
    },
  ],
})

describe('Jobs Settings Hub (Variant A) & Sub-pages', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setLocale('en')
    reactiveAgentId.value = 'wiki_researcher'
    reactiveSkillId.value = 'wiki-query'
    reactiveRouteName.value = 'jobs-settings-agent'
    requestMock.mockImplementation((path: string) => {
      if (path === '/api/admin/background/settings') {
        return Promise.resolve(fixtureSettings())
      }
      return Promise.resolve({ items: [], total: 0, page: 1, page_size: 30 })
    })
  })

  it('renders Variant A Settings Hub on BackgroundPage and navigates to dedicated sub-pages', async () => {
    const wrapper = mount(BackgroundPage)
    await flushPromises()

    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)
    expect(wrapper.find('[data-hub-runtime-banner]').exists()).toBe(true)
    expect(wrapper.text()).toContain('Agents & Subagents')
    expect(wrapper.text()).toContain('Skill Library')
    expect(wrapper.text()).toContain('Runtime Settings')
    expect(wrapper.text()).toContain('LLM & Providers')

    await wrapper.get('[data-detail-back]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs' })

    await wrapper.get('[data-hub-tile="agents"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings-agents' })

    await wrapper.get('[data-hub-tile="skills"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings-skills' })

    await wrapper.get('[data-hub-tile="runtime"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings-runtime' })

    await wrapper.get('[data-hub-tile="providers"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings-providers' })

    await wrapper.get('[data-quick-agent]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-agent',
      params: { agentId: 'wiki_researcher' },
    })

    await wrapper.get('[data-quick-skill]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-skill',
      params: { skillId: 'wiki-query' },
    })
  })

  it('renders BackgroundRuntimePage and BackgroundProvidersPage and saves updates', async () => {
    const runtimeWrapper = mount(BackgroundRuntimePage)
    await flushPromises()

    expect(runtimeWrapper.text()).toContain('Runtime Settings')
    expect(runtimeWrapper.text()).toContain('Runtime readiness')
    expect(runtimeWrapper.text()).toContain('v1')

    const runtimeSwitch = runtimeWrapper.get('input[data-toggle="runtime"]')
    await runtimeSwitch.setValue(false)
    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        enabled: false,
        revision: 2,
      }),
    )
    await runtimeWrapper.get('[data-runtime-save]').trigger('click')
    await flushPromises()

    const runtimePut = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(runtimePut).toBeDefined()
    expect(JSON.parse(String(runtimePut![1].body)).enabled).toBe(false)

    // Test BackgroundProvidersPage
    const providersWrapper = mount(BackgroundProvidersPage)
    await flushPromises()

    expect(providersWrapper.text()).toContain('Default LLM Providers')
    expect(providersWrapper.text()).toContain('Orchestrator · Root Agents')
    expect(providersWrapper.text()).toContain('Worker · Subagents')

    await providersWrapper.get('select[data-orchestrator-provider]').setValue('llm_deepseek')
    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        orchestrator_llm_provider: 'llm_deepseek',
        revision: 3,
      }),
    )
    await providersWrapper.get('[data-providers-save]').trigger('click')
    await flushPromises()

    const putCalls = requestMock.mock.calls.filter(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(putCalls.length).toBe(2)
    expect(JSON.parse(String(putCalls[1]![1].body)).orchestrator_llm_provider).toBe('llm_deepseek')
  })

  it('renders Flow 1 (daily_root) on BackgroundAgentsPage and navigates to dedicated subagent config page on Configure', async () => {
    const wrapper = mount(BackgroundAgentsPage)
    await flushPromises()

    expect(wrapper.text()).toContain('Daily Assistant')
    expect(wrapper.text()).toContain('daily_root')
    expect(wrapper.text()).toContain('wiki_researcher')
    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)

    await wrapper.get('[data-detail-back]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings' })

    const configButton = wrapper.get('button[data-agent="wiki_researcher"]')
    await configButton.trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-agent',
      params: { agentId: 'wiki_researcher' },
    })
  })

  it('configures Muse Task through the provider catalog and resets cloud worker to disabled', async () => {
    const settings = fixtureSettings()
    const template = settings.agents.find(a => a.id === 'wiki_researcher')!
    settings.agents.push({
      ...template, id: 'cloud_worker', enabled: true, default_enabled: false,
      allowed_tools: ['muse_task'], available_tools: ['muse_task'],
      default_allowed_tools: ['muse_task'], allowed_skills: [], default_allowed_skills: [],
      tool_capability_tags: { muse_task: 'agent_task' }, tool_providers: {},
    })
    settings.llm_providers.push({
      ref: 'muse_cloud', name: 'Muse Cloud', source: 'database', model: 'muse',
      tags: ['agent_task'],
    })
    reactiveAgentId.value = 'cloud_worker'
    requestMock.mockImplementation((path: string) => Promise.resolve(
      path === '/api/admin/background/settings' ? settings : [],
    ))
    const wrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()
    const selector = wrapper.get('select[data-tool-provider="muse_task"]')
    expect(selector.text()).toContain('Muse Cloud')
    expect(selector.text()).not.toContain('Flux Image Gen')
    await selector.setValue('muse_cloud')
    await wrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()
    let calls = requestMock.mock.calls.filter(c => c[1]?.method === 'PUT')
    expect(JSON.parse(String(calls.at(-1)![1].body)).agents.cloud_worker.tool_providers)
      .toEqual({ muse_task: 'muse_cloud' })
    await wrapper.get('[data-reset-agent-default]').trigger('click')
    await wrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()
    calls = requestMock.mock.calls.filter(c => c[1]?.method === 'PUT')
    expect(JSON.parse(String(calls.at(-1)![1].body)).agents.cloud_worker.enabled).toBe(false)
  })

  it('renders dedicated BackgroundAgentDetailPage for wiki_researcher, media_creator, and media_processor and saves overrides', async () => {
    reactiveAgentId.value = 'wiki_researcher'
    const wrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)
    expect(wrapper.find('[data-anchor-nav]').exists()).toBe(true)
    expect(wrapper.text()).toContain('wiki_researcher')
    expect(wrapper.text()).toContain('Sub-Agent')
    expect(wrapper.text()).toContain('Coordinator: daily_root')

    // Back button returns to jobs-settings-agents
    await wrapper.get('[data-detail-back]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'jobs-settings-agents' })

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
    expect(mediaWrapper.find('select[data-tool-provider="tts_synthesize"]').exists()).toBe(false)

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

    // Switch to media_processor and configure capability-routed tool provider
    reactiveAgentId.value = 'media_processor'
    const procWrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    const ttsToolSelect = procWrapper.get('select[data-tool-provider="tts_synthesize"]')
    expect(ttsToolSelect.text()).toContain('Auto-select by tag (tts)')
    expect(ttsToolSelect.text()).toContain('VieNeu TTS')
    await ttsToolSelect.setValue('tts_vieneu')

    requestMock.mockImplementationOnce(() =>
      Promise.resolve({
        ...fixtureSettings(),
        revision: 4,
      }),
    )
    await procWrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()

    const allPutCalls = requestMock.mock.calls.filter(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(allPutCalls.length).toBe(3)
    const procBody = JSON.parse(String(allPutCalls[2]![1].body))
    expect(procBody.agents.media_processor.tool_providers).toEqual({
      tts_synthesize: 'tts_vieneu',
    })
  })

  it('keeps a Required Job Skill selected, explains why, and keeps both edit paths available', async () => {
    const settings = fixtureSettings()
    const researcher = settings.agents.find(agent => agent.id === 'wiki_researcher')!
    researcher.available_skills = ['wiki-taxonomy']
    researcher.default_allowed_skills = []
    requestMock.mockImplementation((path: string) => {
      if (path === '/api/admin/background/settings') return Promise.resolve(settings)
      if (path === '/api/admin/background/skills/wiki-query') {
        return Promise.resolve({
          id: 'wiki-query',
          name: 'Wiki query',
          description: 'Read and query the Wiki.',
          body: 'Read Wiki pages.',
          is_builtin: true,
          is_modified: false,
          default_has_update: false,
          etag: 'wiki-query-v1',
          used_by_agents: ['wiki_researcher'],
          suggested_for_agents: [],
          default: null,
        })
      }
      return Promise.resolve({ items: [], total: 0, page: 1, page_size: 30 })
    })

    const wrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    const required = wrapper.get<HTMLInputElement>('input[data-agent-skill="wiki-query"]')
    expect(required.element.checked).toBe(true)
    expect(required.element.disabled).toBe(true)
    expect(wrapper.text()).toContain('Required')
    expect(wrapper.text()).toContain('This skill is always included in this Sub-Agent system prompt.')
    await wrapper.get('[data-reset-agent-default]').trigger('click')
    expect(required.element.checked).toBe(true)
    const ordinary = wrapper.get<HTMLInputElement>('input[data-agent-skill="wiki-taxonomy"]')
    expect(ordinary.element.disabled).toBe(false)
    await ordinary.setValue(true)
    expect(ordinary.element.checked).toBe(true)

    await wrapper.get('[data-agent-required-skill-link="wiki-query"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-skill',
      params: { skillId: 'wiki-query' },
    })

    await wrapper.get('[data-agent-edit-skill="wiki-query"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-skill-editor-modal]').exists()).toBe(true)

    await wrapper.get('[data-skill-modal-close]').trigger('click')
    await wrapper.get('input[data-agent-steps]').setValue(9)
    await wrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()
    const putCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(putCall).toBeDefined()
    expect(JSON.parse(String(putCall![1].body)).agents.wiki_researcher.allowed_skills).toContain(
      'wiki-query',
    )
  })

  it('manages Job Skills on BackgroundSkillsPage and dedicated BackgroundSkillDetailPage', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)

    requestMock.mockImplementation((path: string, init?: RequestInit) => {
      if (path === '/api/admin/background/settings') {
        return Promise.resolve(fixtureSettings())
      }
      if (path === '/api/admin/background/skills/wiki-query' && (!init || !init.method)) {
        return Promise.resolve({
          id: 'wiki-query',
          name: 'wiki-query-custom',
          description: 'Modified wiki-query description',
          body: '## Custom Body\n\nModified instructions.',
          is_builtin: true,
          is_modified: true,
          default_has_update: true,
          etag: 'etag-wiki-query',
          used_by_agents: ['wiki_researcher'],
          suggested_for_agents: ['wiki_researcher'],
          default: {
            name: 'wiki-query',
            description: 'Answer questions from the Karpathy Wiki.',
            body: '## Default Body\n\nOriginal built-in instructions.',
          },
        })
      }
      if (path === '/api/admin/background/skills/wiki-query' && init?.method === 'PUT') {
        return Promise.resolve({
          id: 'wiki-query',
          name: 'wiki-query',
          description: 'Answer questions from the Karpathy Wiki.',
          body: '## Default Body\n\nOriginal built-in instructions.',
          is_builtin: true,
          is_modified: false,
          default_has_update: false,
          etag: 'etag-wiki-query-2',
          used_by_agents: ['wiki_researcher'],
          suggested_for_agents: ['wiki_researcher'],
          default: {
            name: 'wiki-query',
            description: 'Answer questions from the Karpathy Wiki.',
            body: '## Default Body\n\nOriginal built-in instructions.',
          },
        })
      }
      if (path === '/api/admin/background/skills/new-custom-skill' && init?.method === 'PUT') {
        return Promise.resolve({
          id: 'new-custom-skill',
          name: 'New Custom Skill',
          description: 'Custom skill created from modal',
          body: '## Custom Steps\n\n1. Run check.',
          is_builtin: false,
          is_modified: true,
          default_has_update: false,
          etag: 'etag-new-custom',
          used_by_agents: [],
          suggested_for_agents: [],
          default: null,
        })
      }
      if (path === '/api/admin/background/skills/seo-audit' && init?.method === 'DELETE') {
        return Promise.resolve(undefined)
      }
      return Promise.resolve({ items: [], total: 0, page: 1, page_size: 30 })
    })

    const wrapper = mount(BackgroundSkillsPage)
    await flushPromises()

    // Catalog table renders both built-in and custom skills
    const catalog = wrapper.get('[data-skill-catalog]')
    expect(catalog.text()).toContain('wiki-query')
    expect(catalog.text()).toContain('Built-in')
    expect(catalog.text()).toContain('seo-audit')
    expect(catalog.text()).toContain('Custom')
    expect(wrapper.get('[data-skill-default-update="wiki-query"]').text()).toContain(
      'Default update available',
    )

    // Built-in skill row must NOT render a delete button; custom skill row MUST render one
    expect(wrapper.find('[data-skill-delete-card="wiki-query"]').exists()).toBe(false)
    expect(wrapper.find('[data-skill-delete-card="seo-audit"]').exists()).toBe(true)

    // Clicking Details navigates to dedicated Skill Detail page
    await wrapper.get('[data-skill-edit="wiki-query"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-skill',
      params: { skillId: 'wiki-query' },
    })

    // 1. Mount BackgroundSkillDetailPage for built-in skill wiki-query, verify no delete button, restore default, and save
    reactiveRouteName.value = 'jobs-settings-skill'
    reactiveSkillId.value = 'wiki-query'
    const detailWrapper = mount(BackgroundSkillDetailPage)
    await flushPromises()

    expect(detailWrapper.get('[data-skill-default-update]').text()).toContain(
      'Default update available',
    )
    expect(detailWrapper.get('[data-skill-required-by]').text()).toContain('wiki_researcher')
    await detailWrapper.get('[data-skill-required-agent="wiki_researcher"]').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-agent',
      params: { agentId: 'wiki_researcher' },
    })
    expect(detailWrapper.find('[data-skill-delete]').exists()).toBe(false)
    expect(
      (detailWrapper.get('[data-skill-name-input]').element as HTMLInputElement).value,
    ).toBe('wiki-query-custom')

    await detailWrapper.get('[data-skill-restore-default]').trigger('click')
    expect(
      (detailWrapper.get('[data-skill-name-input]').element as HTMLInputElement).value,
    ).toBe('wiki-query')

    await detailWrapper.get('[data-skill-save]').trigger('click')
    await flushPromises()

    const updateSkillCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/skills/wiki-query' && c[1]?.method === 'PUT',
    )
    expect(updateSkillCall).toBeDefined()
    expect(updateSkillCall![2]?.headers).toEqual({ 'If-Match': '"etag-wiki-query"' })

    // 2. Open Create Modal on BackgroundSkillsPage, fill in new custom skill, and save
    await wrapper.get('[data-skill-create-btn]').trigger('click')
    await flushPromises()

    await wrapper.get('[data-skill-id-input]').setValue('new-custom-skill')
    await wrapper.get('[data-skill-name-input]').setValue('New Custom Skill')
    await wrapper.get('[data-skill-description-input]').setValue('Custom skill created from modal')
    await wrapper.get('[data-skill-body-textarea]').setValue('## Custom Steps\n\n1. Run check.')
    await wrapper.get('[data-skill-save]').trigger('click')
    await flushPromises()

    const createSkillCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/skills/new-custom-skill' && c[1]?.method === 'PUT',
    )
    expect(createSkillCall).toBeDefined()
    expect(createSkillCall![2]?.headers).toEqual({ 'If-None-Match': '*' })

    // 3. Delete custom skill from row
    await wrapper.get('[data-skill-delete-card="seo-audit"]').trigger('click')
    await flushPromises()

    const deleteSkillCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/skills/seo-audit' && c[1]?.method === 'DELETE',
    )
    expect(deleteSkillCall).toBeDefined()
    expect(deleteSkillCall![2]?.headers).toEqual({ 'If-Match': '"etag-seo-audit"' })
  })

  it('creates a new custom skill from BackgroundAgentDetailPage and auto-selects it in allowed_skills', async () => {
    reactiveAgentId.value = 'wiki_researcher'
    let created = false
    requestMock.mockImplementation((path: string, init?: RequestInit) => {
      if (path === '/api/admin/background/settings' && (!init || !init.method)) {
        const base = fixtureSettings()
        if (created) {
          base.agents[1]!.available_skills = ['wiki-query', 'wiki-taxonomy', 'fact-check']
          base.skills.push({
            id: 'fact-check',
            name: 'Fact Check',
            description: 'Cross-verify claims before synthesis.',
            is_builtin: false,
            is_modified: true,
            default_has_update: false,
            etag: 'etag-fact-check',
            used_by_agents: [],
            suggested_for_agents: [],
          })
        }
        return Promise.resolve(base)
      }
      if (path === '/api/admin/background/skills/fact-check' && init?.method === 'PUT') {
        created = true
        return Promise.resolve({
          id: 'fact-check',
          name: 'Fact Check',
          description: 'Cross-verify claims before synthesis.',
          body: '## Verify\n- Check primary source.',
          is_builtin: false,
          is_modified: true,
          default_has_update: false,
          etag: 'etag-fact-check',
          used_by_agents: [],
          suggested_for_agents: [],
          default: null,
        })
      }
      return Promise.resolve({ ...fixtureSettings(), revision: 2 })
    })

    const wrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()

    await wrapper.get('[data-agent-create-skill]').trigger('click')
    await flushPromises()

    await wrapper.get('[data-skill-id-input]').setValue('fact-check')
    await wrapper.get('[data-skill-name-input]').setValue('Fact Check')
    await wrapper
      .get('[data-skill-description-input]')
      .setValue('Cross-verify claims before synthesis.')
    await wrapper.get('[data-skill-body-textarea]').setValue('## Verify\n- Check primary source.')
    await wrapper.get('[data-skill-save]').trigger('click')
    await flushPromises()

    // Save the agent config and verify fact-check is included in allowed_skills
    await wrapper.get('[data-save-agent-config]').trigger('click')
    await flushPromises()

    const putCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/settings' && c[1]?.method === 'PUT',
    )
    expect(putCall).toBeDefined()
    const body = JSON.parse(String(putCall![1].body))
    expect(body.agents.wiki_researcher.allowed_skills).toContain('fact-check')
  })

  it('creates and deletes a Custom Sub-Agent on BackgroundAgentsPage and BackgroundAgentDetailPage', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true)

    const customAgentEntry = {
      id: 'arxiv_digest',
      flow_id: 'daily',
      parent_id: 'daily_root',
      mode: 'subagent',
      is_builtin: false,
      enabled: true,
      provider_ref: null,
      effective_provider_ref: 'orchestrator:default',
      effective_model: 'gpt-4o-mini',
      steps: 18,
      default_steps: 20,
      allowed_tools: ['skill', 'fetch_html', 'markdown_create'],
      default_allowed_tools: ['skill', 'fetch_html', 'markdown_create'],
      available_tools: ['skill', 'fetch_html', 'markdown_create'],
      allowed_skills: ['wiki-query'],
      default_allowed_skills: ['wiki-query'],
      available_skills: ['wiki-query', 'seo-audit'],
      allowed_subagents: [],
      description: 'Summarizes arXiv papers into Markdown briefs.',
      default_description: 'Summarizes arXiv papers into Markdown briefs.',
      system_prompt: '# arXiv Digest\n\nSummarize papers.',
      default_system_prompt: '# arXiv Digest\n\nSummarize papers.',
    }

    requestMock.mockImplementation((path: string, init?: RequestInit) => {
      if (path === '/api/admin/background/settings' && (!init || !init.method)) {
        return Promise.resolve(fixtureSettings())
      }
      if (path === '/api/admin/background/agents' && init?.method === 'POST') {
        const next = fixtureSettings()
        return Promise.resolve({
          ...next,
          revision: 2,
          agents: [
            {
              ...next.agents[0]!,
              allowed_subagents: ['wiki_researcher', 'media_creator', 'arxiv_digest'],
            },
            ...next.agents.slice(1),
            customAgentEntry,
          ],
        })
      }
      if (path === '/api/admin/background/agents/arxiv_digest' && init?.method === 'DELETE') {
        const next = fixtureSettings()
        next.revision = 3
        return Promise.resolve(next)
      }
      return Promise.resolve(fixtureSettings())
    })

    const wrapper = mount(BackgroundAgentsPage)
    await flushPromises()

    // Built-in subagents must NOT show a delete button
    expect(wrapper.find('[data-subagent-delete-card="wiki_researcher"]').exists()).toBe(false)

    // Open Create Custom Sub-Agent modal for Flow 1 (`daily`)
    await wrapper.get('[data-subagent-create-btn="daily"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-subagent-create-modal]').exists()).toBe(true)

    // Verify Flow 1 tool pool has collector tools and Flow 2 tool pool strips collector tools
    expect(wrapper.find('[data-subagent-tool="yt_fetch_transcript"]').exists()).toBe(true)
    await wrapper.get('[data-subagent-tool="yt_fetch_transcript"]').setValue(true)
    await wrapper.get('[data-subagent-flow-select]').setValue('wiki')
    await flushPromises()
    expect(wrapper.find('[data-subagent-tool="yt_fetch_transcript"]').exists()).toBe(false)
    expect(wrapper.find('[data-subagent-tool="wiki_lint"]').exists()).toBe(true)
    await wrapper.get('[data-subagent-flow-select]').setValue('daily')
    await flushPromises()
    expect(
      (wrapper.get('[data-subagent-tool="yt_fetch_transcript"]').element as HTMLInputElement)
        .checked,
    ).toBe(false)

    await wrapper.get('[data-subagent-id-input]').setValue('arxiv_digest')
    await wrapper.get('[data-subagent-steps-input]').setValue(18)
    await wrapper
      .get('[data-subagent-description-input]')
      .setValue('Summarizes arXiv papers into Markdown briefs.')
    await wrapper.get('[data-subagent-tool="fetch_html"]').setValue(true)
    await wrapper.get('[data-subagent-tool="markdown_create"]').setValue(true)
    await wrapper.get('[data-subagent-skill="wiki-query"]').setValue(true)
    await wrapper
      .get('[data-subagent-prompt-textarea]')
      .setValue('# arXiv Digest\n\nSummarize papers.')

    await wrapper.get('[data-subagent-create-submit]').trigger('click')
    await flushPromises()

    const createCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/agents' && c[1]?.method === 'POST',
    )
    expect(createCall).toBeDefined()
    expect(createCall![2]?.revision).toBe(1)
    const createBody = JSON.parse(String(createCall![1].body))
    expect(createBody.id).toBe('arxiv_digest')
    expect(createBody.flow_id).toBe('daily')
    expect(createBody.allowed_tools).toEqual(['skill', 'fetch_html', 'markdown_create'])

    // Custom subagent row now renders with a delete button
    expect(wrapper.text()).toContain('arxiv_digest')
    expect(wrapper.find('[data-subagent-delete-card="arxiv_digest"]').exists()).toBe(true)

    await wrapper.get('[data-subagent-delete-card="arxiv_digest"]').trigger('click')
    await flushPromises()

    const deleteCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/agents/arxiv_digest' && c[1]?.method === 'DELETE',
    )
    expect(deleteCall).toBeDefined()
    expect(deleteCall![2]?.revision).toBe(2)

    // Also verify BackgroundAgentDetailPage renders delete button (and hides reset default) for custom subagent
    reactiveAgentId.value = 'arxiv_digest'
    requestMock.mockImplementationOnce(() => {
      const withCustom = fixtureSettings()
      return Promise.resolve({
        ...withCustom,
        agents: [...withCustom.agents, customAgentEntry],
      })
    })
    const detailWrapper = mount(BackgroundAgentDetailPage)
    await flushPromises()
    expect(detailWrapper.find('[data-reset-agent-default]').exists()).toBe(false)
    expect(detailWrapper.find('[data-delete-subagent]').exists()).toBe(true)
    expect(detailWrapper.find('[data-delete-custom-subagent]').exists()).toBe(true)
  })
})
