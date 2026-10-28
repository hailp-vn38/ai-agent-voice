import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'

import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountView,
  GatewayPoolDetailView,
  GatewayPoolModelView,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { setLocale } from '@/composables/useI18n'

import LlmGatewayPage from './LlmGatewayPage.vue'

const sampleCodexModel: GatewayPoolModelView = {
  provider_key: 'llm_codex_pool_1',
  display_name: 'GPT-5.3 Codex',
  vendor: 'openai_codex',
  adapter: 'llm_gateway',
  gateway_binding: 'pool:openai_codex',
  binding_mode: 'pool',
  model: 'gpt-5.3-codex',
  enabled: true,
  revision: 1,
  tags: ['chat', 'reasoning', 'vision'],
  reasoning_effort: 'high',
  max_tokens: 8192,
  context_limit_tokens: 200000,
  temperature_percent: null,
  timeout_ms: 60000,
  template_usage_count: 0,
  created_at: 1793000000,
  updated_at: 1793000000,
}

const sampleVendors: GatewayVendorInfo[] = [
  {
    vendor: 'anthropic',
    display_name: 'Anthropic Claude',
    login_mode: 'pkce',
    default_base_url: 'https://api.anthropic.com',
    pool_binding: 'pool:anthropic',
    total_accounts: 0,
    active_accounts: 0,
    cooling_down_accounts: 0,
    model_count: 0,
    models: [],
  },
  {
    vendor: 'openai_codex',
    display_name: 'OpenAI Codex (ChatGPT)',
    login_mode: 'pkce',
    default_base_url: 'https://chatgpt.com/backend-api/codex',
    pool_binding: 'pool:openai_codex',
    total_accounts: 1,
    active_accounts: 1,
    cooling_down_accounts: 0,
    model_count: 1,
    models: [sampleCodexModel],
  },
  {
    vendor: 'muse',
    display_name: 'Muse AI (Browser CDP)',
    login_mode: 'pkce',
    default_base_url: 'https://muse.ai',
    pool_binding: 'pool:muse',
    total_accounts: 1,
    active_accounts: 1,
    cooling_down_accounts: 0,
    model_count: 0,
    models: [],
  },
]

const sampleAccounts: GatewayAccountView[] = [
  {
    account_key: 'gw_openai_codex_145ef96e',
    vendor: 'openai_codex',
    label: 'EDU',
    account_email: 'lamphuchai98@gmail.com',
    credential_metadata: { key_version: 1 },
    enabled: true,
    priority: 100,
    status: 'active',
    cooldown_remaining_secs: 0,
    expires_at: 1794000000,
    last_refreshed_at: 1793000000,
    last_used_at: null,
    last_error: null,
    bound_providers: [
      {
        provider_key: 'codex',
        display_name: 'codex',
        binding_mode: 'account',
      },
    ],
  },
  {
    account_key: 'gw_muse_0ef9e16c',
    vendor: 'muse',
    label: 'Muse-lph02',
    account_email: null,
    credential_metadata: { key_version: 1 },
    enabled: true,
    priority: 100,
    status: 'active',
    cooldown_remaining_secs: 0,
    expires_at: 1793500000,
    last_refreshed_at: 1793000000,
    last_used_at: null,
    last_error: null,
    bound_providers: [
      {
        provider_key: 'muse_gw',
        display_name: 'Muse AI (Gateway)',
        binding_mode: 'pool',
      },
    ],
  },
]

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/llm-gateway', component: LlmGatewayPage },
      { path: '/providers/:key', component: { template: '<div />' } },
    ],
  })
}

describe('LlmGatewayPage', () => {
  beforeEach(() => {
    setLocale('vi')
    vi.restoreAllMocks()
    vi.spyOn(llmGatewayApi, 'listVendors').mockResolvedValue({ vendors: sampleVendors })
    vi.spyOn(llmGatewayApi, 'listAccounts').mockResolvedValue({ accounts: sampleAccounts })
    vi.spyOn(llmGatewayApi, 'listQuotas').mockResolvedValue({ quotas: [] })
  })

  it('groups populated pools into collapsible sections and hides top pool cards', async () => {
    const router = makeRouter()
    await router.push('/llm-gateway')
    await router.isReady()

    const wrapper = mount(LlmGatewayPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    await flushPromises()

    // Only populated vendor pools render as group sections
    const poolGroups = wrapper.findAll('[data-pool-group]')
    expect(poolGroups).toHaveLength(2)
    expect(poolGroups[0]?.attributes('data-pool-group')).toBe('openai_codex')
    expect(poolGroups[1]?.attributes('data-pool-group')).toBe('muse')

    // Empty vendor pool (anthropic) is shown in the compact empty pools bar
    expect(wrapper.text()).toContain('Pool chưa có tài khoản:')
    expect(wrapper.text()).toContain('Anthropic Claude')

    // Toggle collapse on openai_codex pool group
    const codexToggle = wrapper.get('[data-pool-toggle="openai_codex"]')
    expect(codexToggle.attributes('aria-expanded')).toBe('true')
    await codexToggle.trigger('click')
    expect(codexToggle.attributes('aria-expanded')).toBe('false')

    // Toggle all pools collapse / expand
    const toggleAllBtn = wrapper.get('[data-testid="toggle-all-pools"]')
    await toggleAllBtn.trigger('click')
    expect(wrapper.get('[data-pool-toggle="openai_codex"]').attributes('aria-expanded')).toBe(
      'false',
    )
    expect(wrapper.get('[data-pool-toggle="muse"]').attributes('aria-expanded')).toBe('false')

    await toggleAllBtn.trigger('click')
    expect(wrapper.get('[data-pool-toggle="openai_codex"]').attributes('aria-expanded')).toBe(
      'true',
    )
    expect(wrapper.get('[data-pool-toggle="muse"]').attributes('aria-expanded')).toBe('true')
  })

  it('supports viewing Pool Detail and adding, editing, and deleting models in a Pool', async () => {
    const samplePoolDetail: GatewayPoolDetailView = {
      vendor: 'openai_codex',
      display_name: 'OpenAI Codex (ChatGPT)',
      login_mode: 'pkce',
      default_base_url: 'https://chatgpt.com/backend-api/codex',
      pool_binding: 'pool:openai_codex',
      total_accounts: 1,
      active_accounts: 1,
      cooling_down_accounts: 0,
      reauth_required_accounts: 0,
      disabled_accounts: 0,
      accounts: [sampleAccounts[0]!],
      models: [sampleCodexModel],
      preset_models: [
        {
          model: 'gpt-5.6-terra',
          display_name: 'GPT-5.6 Terra (Codex)',
          tags: ['chat', 'reasoning', 'vision'],
          supports_reasoning_effort: true,
          default_max_tokens: 4096,
          default_context_limit_tokens: 200000,
        },
      ],
    }

    const getPoolSpy = vi
      .spyOn(llmGatewayApi, 'getPoolDetail')
      .mockResolvedValue({ pool: samplePoolDetail })
    const createModelSpy = vi
      .spyOn(llmGatewayApi, 'createPoolModel')
      .mockResolvedValue({
        model: {
          ...sampleCodexModel,
          provider_key: 'llm_codex_pool_2',
          model: 'gpt-5.6-terra',
          display_name: 'GPT-5.6 Terra (Codex)',
        },
      })
    const updateModelSpy = vi
      .spyOn(llmGatewayApi, 'updatePoolModel')
      .mockResolvedValue({
        model: {
          ...sampleCodexModel,
          display_name: 'GPT-5.3 Codex Updated',
        },
      })
    const deleteModelSpy = vi
      .spyOn(llmGatewayApi, 'deletePoolModel')
      .mockResolvedValue({ deleted: true })

    const router = makeRouter()
    await router.push('/llm-gateway')
    await router.isReady()

    const wrapper = mount(LlmGatewayPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    await flushPromises()

    // 1. Inline Models in Pool sub-section renders the configured model
    expect(wrapper.get('[data-pool-model-count="openai_codex"]').text()).toContain('1 model')
    expect(wrapper.get('[data-pool-model-row="llm_codex_pool_1"]').text()).toContain(
      'gpt-5.3-codex',
    )

    // 2. Open Pool Detail modal
    await wrapper.get('[data-pool-detail="openai_codex"]').trigger('click')
    await flushPromises()
    expect(getPoolSpy).toHaveBeenCalledWith('openai_codex')
    expect(wrapper.get('[data-testid="pool-detail-content"]').text()).toContain(
      'https://chatgpt.com/backend-api/codex',
    )

    // 3. Click a Preset Model in Pool Detail to open Add Model modal prefilled
    await wrapper.get('[data-preset-model="gpt-5.6-terra"]').trigger('click')
    await flushPromises()
    const modelInput = wrapper.get('[data-testid="pool-model-id-input"]')
    expect((modelInput.element as HTMLInputElement).value).toBe('gpt-5.6-terra')

    await wrapper.get('[data-testid="pool-model-form"]').trigger('submit.prevent')
    await flushPromises()
    expect(createModelSpy).toHaveBeenCalledWith(
      'openai_codex',
      expect.objectContaining({
        model: 'gpt-5.6-terra',
        display_name: 'GPT-5.6 Terra (Codex)',
        gateway_binding: 'pool:openai_codex',
        tags: ['chat', 'reasoning', 'vision'],
      }),
    )

    // 4. Edit existing model in the Pool (including temperature_percent)
    await wrapper.get('[data-edit-pool-model="llm_codex_pool_1"]').trigger('click')
    await flushPromises()
    await wrapper.get('[data-testid="pool-model-name-input"]').setValue('GPT-5.3 Codex Updated')
    await wrapper.get('[data-testid="pool-model-temperature-input"]').setValue('75')
    await wrapper.get('[data-testid="pool-model-form"]').trigger('submit.prevent')
    await flushPromises()
    expect(updateModelSpy).toHaveBeenCalledWith(
      'openai_codex',
      'llm_codex_pool_1',
      expect.objectContaining({
        model: 'gpt-5.3-codex',
        display_name: 'GPT-5.3 Codex Updated',
        temperature_percent: 75,
      }),
    )

    // 4b. Toggle model enabled/disabled in the Pool
    await wrapper.get('[data-toggle-pool-model="llm_codex_pool_1"]').trigger('click')
    await flushPromises()
    expect(updateModelSpy).toHaveBeenCalledWith('openai_codex', 'llm_codex_pool_1', {
      enabled: false,
    })

    // 5. Delete model from the Pool
    await wrapper.get('[data-delete-pool-model="llm_codex_pool_1"]').trigger('click')
    await flushPromises()
    const confirmButtons = wrapper
      .findAll('button')
      .filter((b) => b.text().includes('Xóa Model'))
    expect(confirmButtons.length).toBeGreaterThan(0)
    await confirmButtons[confirmButtons.length - 1]!.trigger('click')
    await flushPromises()
    expect(deleteModelSpy).toHaveBeenCalledWith('openai_codex', 'llm_codex_pool_1')
  })
})
