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

import LlmGatewayAccountDetailPage from './LlmGatewayAccountDetailPage.vue'
import LlmGatewayAccountsPage from './LlmGatewayAccountsPage.vue'
import LlmGatewayConnectPage from './LlmGatewayConnectPage.vue'
import LlmGatewayModelEditorPage from './LlmGatewayModelEditorPage.vue'
import LlmGatewayModelsPage from './LlmGatewayModelsPage.vue'
import LlmGatewayPage from './LlmGatewayPage.vue'
import LlmGatewayPoolDetailPage from './LlmGatewayPoolDetailPage.vue'
import LlmGatewayPoolsPage from './LlmGatewayPoolsPage.vue'
import LlmGatewayQuotasPage from './LlmGatewayQuotasPage.vue'
import { useNotifyState } from '@/composables/useNotify'

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
    display_name: 'Muse AI (Cookie)',
    login_mode: 'cookie_paste',
    default_base_url: 'https://muse.ai',
    pool_binding: 'pool:muse',
    total_accounts: 1,
    active_accounts: 0,
    cooling_down_accounts: 1,
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
    status: 'cooling_down',
    cooldown_remaining_secs: 90,
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
      default_max_tokens: 128000,
      default_context_limit_tokens: 1050000,
      default_reasoning_effort: 'high',
      default_timeout_ms: 90000,
    },
  ],
}

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/llm-gateway', name: 'llm-gateway', component: LlmGatewayPage },
      { path: '/llm-gateway/pools', name: 'llm-gateway-pools', component: LlmGatewayPoolsPage },
      {
        path: '/llm-gateway/pools/:vendor',
        name: 'llm-gateway-pool',
        component: LlmGatewayPoolDetailPage,
      },
      {
        path: '/llm-gateway/accounts',
        name: 'llm-gateway-accounts',
        component: LlmGatewayAccountsPage,
      },
      {
        path: '/llm-gateway/accounts/:accountKey',
        name: 'llm-gateway-account',
        component: LlmGatewayAccountDetailPage,
      },
      {
        path: '/llm-gateway/models',
        name: 'llm-gateway-models',
        component: LlmGatewayModelsPage,
      },
      {
        path: '/llm-gateway/models/new',
        name: 'llm-gateway-model-create',
        component: LlmGatewayModelEditorPage,
      },
      {
        path: '/llm-gateway/models/:providerKey',
        name: 'llm-gateway-model',
        component: LlmGatewayModelEditorPage,
      },
      {
        path: '/llm-gateway/quotas',
        name: 'llm-gateway-quotas',
        component: LlmGatewayQuotasPage,
      },
      {
        path: '/llm-gateway/connect',
        name: 'llm-gateway-connect',
        component: LlmGatewayConnectPage,
      },
      { path: '/providers/:key', component: { template: '<div />' } },
    ],
  })
}

describe('LLM Gateway Hub (Variant A) & Sub-pages', () => {
  beforeEach(() => {
    setLocale('vi')
    vi.restoreAllMocks()
    vi.spyOn(llmGatewayApi, 'listVendors').mockResolvedValue({ vendors: sampleVendors })
    vi.spyOn(llmGatewayApi, 'listAccounts').mockResolvedValue({ accounts: sampleAccounts })
    vi.spyOn(llmGatewayApi, 'listQuotas').mockResolvedValue({ quotas: [] })
    vi.spyOn(llmGatewayApi, 'getPoolDetail').mockResolvedValue({ pool: samplePoolDetail })
  })

  it('shows shimmer skeletons while page data is loading, then renders Variant A Hub with clickable cards', async () => {
    let finishVendors!: (value: { vendors: GatewayVendorInfo[] }) => void
    let finishAccounts!: (value: { accounts: GatewayAccountView[] }) => void
    let finishQuotas!: (value: { quotas: never[] }) => void

    vi.spyOn(llmGatewayApi, 'listVendors').mockReturnValue(
      new Promise((resolve) => {
        finishVendors = resolve
      }),
    )
    vi.spyOn(llmGatewayApi, 'listAccounts').mockReturnValue(
      new Promise((resolve) => {
        finishAccounts = resolve
      }),
    )
    vi.spyOn(llmGatewayApi, 'listQuotas').mockReturnValue(
      new Promise((resolve) => {
        finishQuotas = resolve
      }),
    )

    const router = makeRouter()
    await router.push('/llm-gateway')
    await router.isReady()

    const wrapper = mount(LlmGatewayPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    try {
      await flushPromises()
      expect(wrapper.find('[data-llm-gateway-skeleton]').exists()).toBe(true)
      expect(wrapper.get('[data-llm-gateway-skeleton]').attributes('aria-busy')).toBe('true')
      expect(wrapper.findAll('[data-llm-gateway-skeleton] .card-shimmer').length).toBeGreaterThan(0)
      expect(wrapper.findAll('[data-pool-group]')).toHaveLength(0)

      finishVendors({ vendors: sampleVendors })
      finishAccounts({ accounts: sampleAccounts })
      await flushPromises()

      expect(wrapper.find('[data-llm-gateway-skeleton]').exists()).toBe(false)
      expect(wrapper.find('[data-testid="llm-gateway-hero"]').exists()).toBe(true)
      expect(wrapper.findAll('[data-pool-group]')).toHaveLength(3)
      expect(wrapper.findAll('[data-quota-skeleton]').length).toBeGreaterThan(0)

      finishQuotas({ quotas: [] })
      await flushPromises()
      expect(wrapper.findAll('[data-quota-skeleton]')).toHaveLength(0)

      // Clicking a Vendor Pool card navigates to Pool Detail page
      await wrapper.get('[data-pool="openai_codex"]').trigger('click')
      await flushPromises()
      expect(router.currentRoute.value.name).toBe('llm-gateway-pool')
      expect(router.currentRoute.value.params.vendor).toBe('openai_codex')

      // Clicking a KPI card navigates to its dedicated sub-page
      await wrapper.get('[data-kpi-card="quotas"]').trigger('click')
      await flushPromises()
      expect(router.currentRoute.value.name).toBe('llm-gateway-quotas')
    } finally {
      wrapper.unmount()
    }
  })

  it('supports Pool Detail tabs, Model Editor creation from Preset and editing, and deleting models', async () => {
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
    await router.push('/llm-gateway/pools/openai_codex')
    await router.isReady()

    const poolWrapper = mount(LlmGatewayPoolDetailPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    await flushPromises()

    expect(poolWrapper.get('[data-testid="pool-detail-content"]').text()).toContain(
      'https://chatgpt.com/backend-api/codex',
    )
    expect(poolWrapper.get('[data-testid="pool-detail-accounts"]').text()).toContain(
      'lamphuchai98@gmail.com',
    )

    // Switch to Pool Models tab and toggle / delete model
    await poolWrapper.get('[data-pool-tab="models"]').trigger('click')
    await flushPromises()
    expect(poolWrapper.get('[data-pool-model-row="llm_codex_pool_1"]').text()).toContain(
      'gpt-5.3-codex',
    )

    await poolWrapper.get('[data-toggle-pool-model="llm_codex_pool_1"]').trigger('click')
    await flushPromises()
    expect(updateModelSpy).toHaveBeenCalledWith('openai_codex', 'llm_codex_pool_1', {
      enabled: false,
    })

    await poolWrapper.get('[data-delete-pool-model="llm_codex_pool_1"]').trigger('click')
    await flushPromises()
    const confirmButtons = poolWrapper
      .findAll('button')
      .filter((b) => b.text().includes('Xóa Model'))
    expect(confirmButtons.length).toBeGreaterThan(0)
    await confirmButtons[confirmButtons.length - 1]!.trigger('click')
    await flushPromises()
    expect(deleteModelSpy).toHaveBeenCalledWith('openai_codex', 'llm_codex_pool_1')

    // Switch to Presets tab and click a preset card to navigate to Model Editor create page
    await poolWrapper.get('[data-pool-tab="presets"]').trigger('click')
    await flushPromises()
    await poolWrapper.get('[data-preset-model="gpt-5.6-terra"]').trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.name).toBe('llm-gateway-model-create')
    expect(router.currentRoute.value.query.preset).toBe('gpt-5.6-terra')
    poolWrapper.unmount()

    // Mount Model Editor in create mode with preset query
    const createEditorWrapper = mount(LlmGatewayModelEditorPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    await flushPromises()

    const modelInput = createEditorWrapper.get('[data-testid="pool-model-id-input"]')
    expect((modelInput.element as HTMLInputElement).value).toBe('gpt-5.6-terra')
    const maxTokensInput = createEditorWrapper.get('[data-testid="pool-model-max-tokens-input"]')
    expect((maxTokensInput.element as HTMLInputElement).value).toBe('128000')

    await createEditorWrapper.get('[data-testid="pool-model-form"]').trigger('submit.prevent')
    await flushPromises()
    expect(createModelSpy).toHaveBeenCalledWith(
      'openai_codex',
      expect.objectContaining({
        model: 'gpt-5.6-terra',
        display_name: 'GPT-5.6 Terra (Codex)',
        gateway_binding: 'pool:openai_codex',
        tags: ['chat', 'reasoning', 'vision'],
        reasoning_effort: 'high',
        max_tokens: 128000,
        context_limit_tokens: 1050000,
        timeout_ms: 90000,
      }),
    )
    createEditorWrapper.unmount()

    // Mount Model Editor in edit mode for llm_codex_pool_1
    await router.push('/llm-gateway/models/llm_codex_pool_1?vendor=openai_codex')
    await flushPromises()
    const editEditorWrapper = mount(LlmGatewayModelEditorPage, {
      global: {
        plugins: [router],
        stubs: { Teleport: true },
      },
    })
    await flushPromises()

    await editEditorWrapper
      .get('[data-testid="pool-model-name-input"]')
      .setValue('GPT-5.3 Codex Updated')
    await editEditorWrapper
      .get('[data-testid="pool-model-base-url-input"]')
      .setValue('https://proxy.example.com/v1')
    await editEditorWrapper.get('[data-testid="pool-model-temperature-input"]').setValue('75')
    await editEditorWrapper.get('[data-testid="pool-model-form"]').trigger('submit.prevent')
    await flushPromises()
    expect(updateModelSpy).toHaveBeenCalledWith(
      'openai_codex',
      'llm_codex_pool_1',
      expect.objectContaining({
        model: 'gpt-5.3-codex',
        display_name: 'GPT-5.3 Codex Updated',
        base_url: 'https://proxy.example.com/v1',
        temperature_percent: 75,
      }),
    )
    editEditorWrapper.unmount()
  })

  it('renders dedicated Vendor Pools, OAuth Accounts, Account Detail, Models, Quotas, and Connect pages', async () => {
    vi.spyOn(llmGatewayApi, 'getAccountQuota').mockResolvedValue({
      quota: {
        account_key: 'gw_openai_codex_145ef96e',
        vendor: 'openai_codex',
        plan: 'ChatGPT Plus',
        limit_reached: false,
        windows: [
          {
            name: '5h window',
            used: 18,
            total: 100,
            remaining_percentage: 82,
            reset_at: null,
            unlimited: false,
          },
        ],
        fetched_at: 1793000000,
      },
    })

    const router = makeRouter()
    await router.push('/llm-gateway/pools')
    await router.isReady()

    const poolsWrapper = mount(LlmGatewayPoolsPage, { global: { plugins: [router] } })
    await flushPromises()
    expect(poolsWrapper.find('[data-testid="llm-gateway-pools-page"]').exists()).toBe(true)
    expect(poolsWrapper.findAll('[data-pool]')).toHaveLength(3)
    await poolsWrapper.get('[data-filter="openai_codex"]').trigger('click')
    expect(poolsWrapper.findAll('[data-pool]')).toHaveLength(1)
    poolsWrapper.unmount()

    await router.push('/llm-gateway/accounts')
    await flushPromises()
    const accountsWrapper = mount(LlmGatewayAccountsPage, { global: { plugins: [router] } })
    await flushPromises()
    expect(accountsWrapper.find('[data-testid="llm-gateway-accounts-page"]').exists()).toBe(true)
    await accountsWrapper.get('[data-account="gw_openai_codex_145ef96e"]').trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.name).toBe('llm-gateway-account')
    accountsWrapper.unmount()

    const accountDetailWrapper = mount(LlmGatewayAccountDetailPage, {
      global: { plugins: [router], stubs: { Teleport: true } },
    })
    await flushPromises()
    expect(accountDetailWrapper.find('[data-testid="llm-gateway-account-detail-page"]').exists()).toBe(
      true,
    )
    expect(accountDetailWrapper.text()).toContain('lamphuchai98@gmail.com')
    expect(accountDetailWrapper.text()).toContain('ChatGPT Plus')
    accountDetailWrapper.unmount()

    await router.push('/llm-gateway/models')
    await flushPromises()
    const modelsWrapper = mount(LlmGatewayModelsPage, { global: { plugins: [router] } })
    await flushPromises()
    expect(modelsWrapper.find('[data-testid="llm-gateway-models-page"]').exists()).toBe(true)
    expect(modelsWrapper.text()).toContain('GPT-5.3 Codex')
    modelsWrapper.unmount()

    await router.push('/llm-gateway/quotas')
    await flushPromises()
    const quotasWrapper = mount(LlmGatewayQuotasPage, { global: { plugins: [router] } })
    await flushPromises()
    expect(quotasWrapper.find('[data-testid="llm-gateway-quotas-page"]').exists()).toBe(true)
    expect(quotasWrapper.text()).toContain('Account Quota Matrix')
    quotasWrapper.unmount()

    await router.push('/llm-gateway/connect')
    await flushPromises()
    const connectWrapper = mount(LlmGatewayConnectPage, { global: { plugins: [router] } })
    await flushPromises()
    expect(connectWrapper.find('[data-testid="llm-gateway-connect-page"]').exists()).toBe(true)
    expect(connectWrapper.text()).toContain('Connection Guide')
    connectWrapper.unmount()
  })

  it('connects Muse from pasted cookies without starting a browser login', async () => {
    const startOAuth = vi.spyOn(llmGatewayApi, 'startOAuth')
    const importCookieAccount = vi
      .spyOn(llmGatewayApi, 'importCookieAccount')
      .mockResolvedValue({ account: sampleAccounts[1]! })
    const windowOpen = vi.spyOn(window, 'open')

    const router = makeRouter()
    await router.push('/llm-gateway/connect?vendor=muse&label=Muse-lph02')
    await router.isReady()
    const wrapper = mount(LlmGatewayConnectPage, { global: { plugins: [router] } })
    await flushPromises()

    const cookies = wrapper.find('[data-input="connect-cookies"]')
    expect(cookies.exists()).toBe(true)

    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(importCookieAccount).not.toHaveBeenCalled()
    expect(useNotifyState().toasts.filter(toast => toast.tone === 'error').map(toast => toast.message).join('\n')).not.toBe('')

    await cookies.setValue('  hatch_sess=s1; hatch_gw=g1  ')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(importCookieAccount).toHaveBeenCalledWith({
      vendor: 'muse',
      label: 'Muse-lph02',
      cookies: 'hatch_sess=s1; hatch_gw=g1',
    })
    expect(startOAuth).not.toHaveBeenCalled()
    expect(windowOpen).not.toHaveBeenCalled()
    expect(wrapper.find('[data-input="connect-cookies"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Muse-lph02')
    expect(wrapper.text()).toContain('pool:muse')
    wrapper.unmount()
  })
})
