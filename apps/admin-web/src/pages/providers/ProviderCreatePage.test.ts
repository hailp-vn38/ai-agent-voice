import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import ProviderCreatePage from './ProviderCreatePage.vue'

const adaptersApi = vi.hoisted(() => ({
  list: vi.fn(),
  get: vi.fn(),
  discoverCapabilities: vi.fn(),
}))

const llmGatewayApiMock = vi.hoisted(() => ({
  listVendors: vi.fn(),
  listAccounts: vi.fn(),
}))

const createProviderFromSetup = vi.hoisted(() => vi.fn())

vi.mock('@/api/provider-adapters', () => ({
  providerAdaptersApi: adaptersApi,
}))

vi.mock('@/api/llm-gateway', () => ({
  llmGatewayApi: llmGatewayApiMock,
}))

vi.mock('@/stores/admin', () => ({
  useAdminStore: () => ({
    createProviderFromSetup,
  }),
}))

function buildRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/providers', name: 'providers', component: { template: '<div>Catalog</div>' } },
      { path: '/providers/create', name: 'provider-create', component: ProviderCreatePage },
      {
        path: '/providers/:key',
        name: 'provider-detail',
        component: { template: '<div>Detail</div>' },
      },
    ],
  })
}

describe('ProviderCreatePage (Variant A wizard)', () => {
  beforeEach(() => {
    adaptersApi.list.mockReset()
    adaptersApi.get.mockReset()
    adaptersApi.discoverCapabilities.mockReset()
    llmGatewayApiMock.listVendors.mockReset()
    llmGatewayApiMock.listAccounts.mockReset()
    llmGatewayApiMock.listVendors.mockResolvedValue({ vendors: [] })
    llmGatewayApiMock.listAccounts.mockResolvedValue({ accounts: [] })
    createProviderFromSetup.mockReset()
  })

  it('renders Variant A header without a theme toggle button and supports the 4-step wizard flow', async () => {
    adaptersApi.list.mockResolvedValue([
      { adapter: 'openai', type: 'llm', display_name: 'OpenAI-compatible LLM' },
      { adapter: 'opencode_free', type: 'llm', display_name: 'OpenCode Free' },
    ])
    adaptersApi.get.mockResolvedValue({
      adapter: 'openai',
      type: 'llm',
      display_name: 'OpenAI-compatible LLM',
      config_schema: {
        fields: [
          { key: 'base_url', label: 'Base URL', type: 'string', required: true },
          { key: 'model', label: 'Model', type: 'string', required: true },
          {
            key: 'timeout_ms',
            label: 'Timeout (ms)',
            type: 'integer',
            required: false,
            minimum: 1000,
            maximum: 600000,
          },
        ],
      },
    })
    createProviderFromSetup.mockResolvedValue({ key: 'llm_prod_1' })

    const router = buildRouter()
    await router.push('/providers/create')
    await router.isReady()

    const wrapper = mount(
      defineComponent({ components: { RouterView }, template: '<RouterView />' }),
      {
        global: {
          plugins: [router],
          stubs: { ProviderTestPanel: { template: '<div data-test-panel />' } },
        },
      },
    )

    await flushPromises()

    // Uses shared DetailHeader with back button and Reset action
    expect(wrapper.find('[data-detail-header]').exists()).toBe(true)
    expect(wrapper.find('[data-detail-back]').exists()).toBe(true)
    expect(wrapper.find('[data-detail-header] [data-action="reset"]').exists()).toBe(true)
    expect(wrapper.find('[data-detail-header] h1').text()).toBe('Tạo Provider mới')

    // 4 steps in stepbar
    const steps = wrapper.findAll('.stepbar .stepitem')
    expect(steps).toHaveLength(4)
    expect(steps.map((item) => item.find('strong').text())).toEqual([
      'Loại & adapter',
      'Cấu hình',
      'Kiểm tra',
      'Xác nhận',
    ])

    // Step 1 shows 4 type tiles and adapter cards
    expect(wrapper.findAll('.type-tile')).toHaveLength(4)
    expect(wrapper.findAll('.adapter-tile')).toHaveLength(2)
    expect(wrapper.find('.adapter-tile.selected .adapter-title').text()).toBe(
      'OpenAI-compatible LLM',
    )

    // Advance to Step 2
    await wrapper.get('[data-action="wizard-next"]').trigger('click')
    expect(wrapper.text()).toContain('02 / Cấu hình OpenAI-compatible LLM')

    await wrapper.get('input[data-general="name"]').setValue('GPT Production')
    await wrapper.get('input[data-field="base_url"]').setValue('https://api.openai.com/v1')
    await wrapper.get('input[data-field="model"]').setValue('gpt-4.1-mini')
    await wrapper.get('input[data-general="apiKey"]').setValue('sk-secret-key-999')

    // Advanced field toggle
    expect(wrapper.find('input[data-field="timeout_ms"]').exists()).toBe(false)
    await wrapper.get('[data-action="advanced"]').trigger('click')
    await wrapper.get('input[data-field="timeout_ms"]').setValue('45000')
    await wrapper.get('input[data-field="timeout_ms"]').setValue('')

    // Advance to Step 3 (Diagnostics)
    await wrapper.get('[data-action="wizard-next"]').trigger('click')
    expect(wrapper.text()).toContain('03 / Chạy kiểm tra cấu hình')
    expect(wrapper.find('[data-test-panel]').exists()).toBe(true)

    // Advance to Step 4 (Review & Create)
    await wrapper.get('[data-action="wizard-next"]').trigger('click')
    expect(wrapper.text()).toContain('04 / Xem lại và tạo Provider')
    const jsonPreview = wrapper.get('[data-live-json]').text()
    expect(jsonPreview).toContain('"name": "GPT Production"')
    expect(jsonPreview).not.toContain('sk-secret-key-999')
    expect(jsonPreview).not.toContain('timeout_ms')

    // Submit create
    await wrapper.get('[data-action="create"]').trigger('click')
    await flushPromises()

    expect(createProviderFromSetup).toHaveBeenCalledWith({
      name: 'GPT Production',
      type: 'llm',
      adapter: 'openai',
      config_json: {
        base_url: 'https://api.openai.com/v1',
        model: 'gpt-4.1-mini',
      },
      api_key: 'sk-secret-key-999',
    })
    expect(router.currentRoute.value.path).toBe('/providers/llm_prod_1')
    wrapper.unmount()
  })

  it('blocks advancing past Step 1 or Step 2 until each step meets all conditions', async () => {
    adaptersApi.list.mockResolvedValue([
      { adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' },
    ])
    adaptersApi.get.mockResolvedValue({
      adapter: 'zerotts_onnx',
      type: 'tts',
      display_name: 'ZeroTTS',
      config_schema: {
        fields: [
          { key: 'voice', label: 'Voice', type: 'string', required: true },
          {
            key: 'speed_percent',
            label: 'Speed (%)',
            type: 'integer',
            required: false,
            minimum: 50,
            maximum: 200,
          },
        ],
      },
    })

    const router = buildRouter()
    await router.push('/providers/create?type=tts')
    await router.isReady()

    const wrapper = mount(
      defineComponent({ components: { RouterView }, template: '<RouterView />' }),
      {
        global: {
          plugins: [router],
          stubs: { ProviderTestPanel: true },
        },
      },
    )

    await flushPromises()

    // On Step 1: Step 2 is enabled (adapter loaded), but Steps 3 & 4 are disabled until Step 2 is valid
    const step2Btn = wrapper.get('.stepbar [data-action="step"][data-value="2"]')
    const step3Btn = wrapper.get('.stepbar [data-action="step"][data-value="3"]')
    const step4Btn = wrapper.get('.stepbar [data-action="step"][data-value="4"]')
    expect(step2Btn.attributes('disabled')).toBeUndefined()
    expect(step3Btn.attributes('disabled')).toBeDefined()
    expect(step4Btn.attributes('disabled')).toBeDefined()

    // Advance to Step 2
    await wrapper.get('[data-action="wizard-next"]').trigger('click')
    expect(wrapper.text()).toContain('02 / Cấu hình ZeroTTS')

    // Next button on Step 2 is disabled while name and required fields are missing
    const nextBtn = wrapper.get('[data-action="wizard-next"]')
    expect(nextBtn.attributes('disabled')).toBeDefined()
    expect(wrapper.get('.wizard-foot').text()).toContain('Bạn cần nhập tên hiển thị.')

    // Entering only display name still keeps Next disabled because required "voice" is empty
    await wrapper.get('input[data-general="name"]').setValue('ZeroTTS Production')
    expect(nextBtn.attributes('disabled')).toBeDefined()
    expect(wrapper.get('.wizard-foot').text()).toContain('Trường “Voice” là bắt buộc.')

    // Filling required "voice" unlocks Step 3 & Step 4 and enables Next
    await wrapper.get('input[data-field="voice"]').setValue('vi_female_1')
    expect(nextBtn.attributes('disabled')).toBeUndefined()
    expect(step3Btn.attributes('disabled')).toBeUndefined()
    expect(step4Btn.attributes('disabled')).toBeUndefined()

    // Entering an invalid advanced integer value re-locks Next and Steps 3 & 4
    await wrapper.get('[data-action="advanced"]').trigger('click')
    await wrapper.get('input[data-field="speed_percent"]').setValue('10')
    expect(nextBtn.attributes('disabled')).toBeDefined()
    expect(step3Btn.attributes('disabled')).toBeDefined()
    expect(step4Btn.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Speed (%) phải từ 50.')

    // Fixing the advanced field unlocks progression again
    await wrapper.get('input[data-field="speed_percent"]').setValue('100')
    expect(nextBtn.attributes('disabled')).toBeUndefined()

    await nextBtn.trigger('click')
    expect(wrapper.text()).toContain('03 / Chạy kiểm tra cấu hình')
    expect(createProviderFromSetup).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('renders model as a Pool Model dropdown when using LLM Gateway and blocks empty pools', async () => {
    llmGatewayApiMock.listVendors.mockResolvedValue({
      vendors: [
        {
          vendor: 'anthropic',
          display_name: 'Anthropic Claude',
          default_base_url: 'https://api.anthropic.com/v1',
          supports_oauth: true,
          supports_browser_cdp: false,
          supports_api_key: true,
          pool_binding: 'pool:anthropic',
          total_accounts: 1,
          healthy_accounts: 1,
          models: [
            {
              key: 'gw_model_claude_sonnet',
              vendor: 'anthropic',
              model: 'claude-sonnet-4-20250514',
              display_name: 'Claude Sonnet 4',
              tags: ['vision', 'tools'],
              max_tokens: 8192,
              context_limit_tokens: 200000,
              reasoning_effort: 'medium',
              temperature_percent: 30,
              timeout_ms: 90000,
              updated_at: 1000,
            },
            {
              key: 'gw_model_claude_opus',
              vendor: 'anthropic',
              model: 'claude-opus-4-20250514',
              display_name: 'Claude Opus 4',
              tags: ['thinking', 'tools'],
              max_tokens: 16384,
              updated_at: 1000,
            },
          ],
        },
        {
          vendor: 'gemini',
          display_name: 'Google Gemini',
          default_base_url: 'https://generativelanguage.googleapis.com/v1beta',
          supports_oauth: true,
          supports_browser_cdp: false,
          supports_api_key: true,
          pool_binding: 'pool:gemini',
          total_accounts: 0,
          healthy_accounts: 0,
          models: [],
        },
      ],
    })
    llmGatewayApiMock.listAccounts.mockResolvedValue({ accounts: [] })

    adaptersApi.list.mockResolvedValue([
      { adapter: 'llm_gateway', type: 'llm', display_name: 'LLM Gateway' },
    ])
    adaptersApi.get.mockResolvedValue({
      adapter: 'llm_gateway',
      type: 'llm',
      display_name: 'LLM Gateway',
      config_schema: {
        fields: [
          { key: 'gateway_binding', label: 'Gateway Binding', type: 'string', required: true },
          { key: 'model', label: 'Model', type: 'string', required: true },
          { key: 'max_tokens', label: 'Max Tokens', type: 'integer', required: false },
          { key: 'timeout_ms', label: 'Timeout (ms)', type: 'integer', required: false },
        ],
      },
    })

    const router = buildRouter()
    await router.push('/providers/create')
    await router.isReady()

    const wrapper = mount(
      defineComponent({ components: { RouterView }, template: '<RouterView />' }),
      {
        global: {
          plugins: [router],
          stubs: { ProviderTestPanel: { template: '<div data-test-panel />' } },
        },
      },
    )

    await flushPromises()

    // Advance to Step 2
    await wrapper.get('[data-action="wizard-next"]').trigger('click')
    await wrapper.get('input[data-general="name"]').setValue('Claude Pool Provider')

    // Model must be a <select>, NOT a manual text <input>
    expect(wrapper.find('input[data-field="model"]').exists()).toBe(false)
    const modelSelect = wrapper.get('select[data-field="model"]')
    const optionValues = modelSelect.findAll('option').map((o) => o.attributes('value'))
    expect(optionValues).toEqual(['claude-sonnet-4-20250514', 'claude-opus-4-20250514'])
    expect((modelSelect.element as HTMLSelectElement).value).toBe('claude-sonnet-4-20250514')

    // Selecting the second pool model updates config with that pool model's presets
    await modelSelect.setValue('claude-opus-4-20250514')

    const nextBtn = wrapper.get('[data-action="wizard-next"]')
    expect(nextBtn.attributes('disabled')).toBeUndefined()

    // Switching vendor pool to Gemini (which has 0 models in Pool) disables model select and blocks Next
    const bindingSelect = wrapper.get('select[data-field="gateway_binding"]')
    await bindingSelect.setValue('pool:gemini')
    expect(modelSelect.attributes('disabled')).toBeDefined()
    expect(nextBtn.attributes('disabled')).toBeDefined()

    // Switching back to Anthropic restores valid model selection
    await bindingSelect.setValue('pool:anthropic')
    expect(modelSelect.attributes('disabled')).toBeUndefined()
    expect(nextBtn.attributes('disabled')).toBeUndefined()

    wrapper.unmount()
  })
})
