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

const createProviderFromSetup = vi.hoisted(() => vi.fn())

vi.mock('@/api/provider-adapters', () => ({
  providerAdaptersApi: adaptersApi,
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

    // Header has Reset action and NO theme toggle button
    expect(wrapper.find('.header-block [data-action="theme"]').exists()).toBe(false)
    expect(wrapper.find('.header-block [data-action="reset"]').exists()).toBe(true)

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

  it('validates required fields on Step 4 and returns to Step 2 with an error alert', async () => {
    adaptersApi.list.mockResolvedValue([
      { adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' },
    ])
    adaptersApi.get.mockResolvedValue({
      adapter: 'zerotts_onnx',
      type: 'tts',
      display_name: 'ZeroTTS',
      config_schema: {
        fields: [{ key: 'voice', label: 'Voice', type: 'string', required: true }],
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

    // Jump directly to Step 4 via stepbar
    await wrapper.get('.stepbar [data-action="step"][data-value="4"]').trigger('click')
    await wrapper.get('[data-action="create"]').trigger('click')

    // Should jump back to Step 2 and show missing name error
    expect(wrapper.get('[role="alert"]').text()).toBe('Bạn cần nhập tên hiển thị.')
    expect(wrapper.text()).toContain('02 / Cấu hình ZeroTTS')
    expect(createProviderFromSetup).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
