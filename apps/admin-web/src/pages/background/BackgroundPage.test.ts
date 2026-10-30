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
      etag: 'etag-seo-audit',
      used_by_agents: [],
      suggested_for_agents: [],
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
    expect(configButtons.length).toBe(4)
    await configButtons[1]!.trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({
      name: 'jobs-settings-agent',
      params: { agentId: 'wiki_researcher' },
    })
  })

  it('renders dedicated BackgroundAgentDetailPage for wiki_researcher, media_creator, and media_processor and saves overrides', async () => {
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

  it('manages Job Skills on BackgroundPage (view catalog, create custom, edit built-in + restore default, delete custom only)', async () => {
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

    const wrapper = mount(BackgroundPage)
    await flushPromises()

    // Catalog renders both built-in and custom skills
    const catalog = wrapper.get('[data-skill-catalog]')
    expect(catalog.text()).toContain('wiki-query')
    expect(catalog.text()).toContain('Built-in')
    expect(catalog.text()).toContain('seo-audit')
    expect(catalog.text()).toContain('Custom')

    // Built-in skill card must NOT render a delete button; custom skill card MUST render one
    expect(wrapper.find('[data-skill-delete-card="wiki-query"]').exists()).toBe(false)
    expect(wrapper.find('[data-skill-delete-card="seo-audit"]').exists()).toBe(true)

    // 1. Open Edit Modal for built-in skill wiki-query, verify no delete button, restore default, and save
    await wrapper.get('[data-skill-edit="wiki-query"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-skill-editor-modal]').exists()).toBe(true)
    expect(wrapper.find('[data-skill-delete]').exists()).toBe(false)
    expect((wrapper.get('[data-skill-name-input]').element as HTMLInputElement).value).toBe(
      'wiki-query-custom',
    )

    await wrapper.get('[data-skill-restore-default]').trigger('click')
    expect((wrapper.get('[data-skill-name-input]').element as HTMLInputElement).value).toBe(
      'wiki-query',
    )

    await wrapper.get('[data-skill-save]').trigger('click')
    await flushPromises()

    const updateSkillCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/skills/wiki-query' && c[1]?.method === 'PUT',
    )
    expect(updateSkillCall).toBeDefined()
    expect(updateSkillCall![2]?.headers).toEqual({ 'If-Match': '"etag-wiki-query"' })

    // 2. Open Create Modal, fill in new custom skill, and save
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

    // 3. Delete custom skill from card
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

  it('creates and deletes a Custom Sub-Agent on BackgroundPage and BackgroundAgentDetailPage', async () => {
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

    const wrapper = mount(BackgroundPage)
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

    // Custom subagent card now renders with a delete button
    expect(wrapper.text()).toContain('arxiv_digest')
    expect(wrapper.find('[data-subagent-delete-card="arxiv_digest"]').exists()).toBe(true)

    // Make an unsaved draft edit on another agent (toggle global runtime off), then delete custom subagent from card
    const globalSwitch = wrapper.get('input[role="switch"]')
    await globalSwitch.setValue(false)
    expect((globalSwitch.element as HTMLInputElement).checked).toBe(false)

    await wrapper.get('[data-subagent-delete-card="arxiv_digest"]').trigger('click')
    await flushPromises()

    const deleteCall = requestMock.mock.calls.find(
      c => c[0] === '/api/admin/background/agents/arxiv_digest' && c[1]?.method === 'DELETE',
    )
    expect(deleteCall).toBeDefined()
    expect(deleteCall![2]?.revision).toBe(2)
    // Unsaved draft edit on BackgroundPage is preserved after deleting custom subagent card
    expect((wrapper.get('input[role="switch"]').element as HTMLInputElement).checked).toBe(false)

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
