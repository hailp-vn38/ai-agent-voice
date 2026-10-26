import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import SystemMcpPage from './SystemMcpPage.vue'

const api = vi.hoisted(() => ({
  knowledge: vi.fn(),
  saveKnowledge: vi.fn(),
  createToken: vi.fn(),
  deleteToken: vi.fn(),
}))

vi.mock('@/api/systemMcp', () => ({ systemMcpApi: api }))

const initialState = {
  revision: 2,
  enabled: true,
  path: '/mcp/knowledge',
  base_url: 'http://localhost:8000/mcp/knowledge',
  allow_write: true,
  max_concurrency: 16,
  call_timeout_ms: 30000,
  max_result_chars: 12000,
  tokens: [
    {
      id: 'tok-1',
      name: 'Default Config Token',
      token: 'kmcp_default_secret_123456',
      access: 'read_write' as const,
      created_at: 1760000000,
    },
  ],
  tools: [
    {
      name: 'wiki_query',
      description: 'Search across wiki and sources.',
      access: 'read_only' as const,
    },
    {
      name: 'add_to_sources',
      description: 'Stage markdown source into sources/.',
      access: 'read_write' as const,
    },
  ],
  read_only_tools_count: 7,
  read_write_tools_count: 16,
}

beforeEach(() => {
  vi.clearAllMocks()
  setLocale('en')
  api.knowledge.mockResolvedValue(structuredClone(initialState))
})

describe('SystemMcpPage', () => {
  it('loads and displays Base URL, config snippet, active tokens, and tools', async () => {
    const wrapper = mount(SystemMcpPage, {
      global: { stubs: { RouterLink: true } },
    })
    await flushPromises()

    const baseUrlInput = wrapper.get('[data-testid="mcp-base-url-input"]')
    expect((baseUrlInput.element as HTMLInputElement).value).toContain('/mcp/knowledge')
    expect(wrapper.get('[data-testid="mcp-config-snippet"]').text()).toContain(
      'Bearer kmcp_default_secret_123456',
    )
    expect(wrapper.text()).toContain('Default Config Token')
    expect(wrapper.text()).toContain('wiki_query')
    expect(wrapper.text()).toContain('add_to_sources')
    wrapper.unmount()
  })

  it('creates a new token and saves web configuration', async () => {
    const createdToken = {
      id: 'tok-2',
      name: 'Cursor Agent',
      token: 'kmcp_cursor_secret_999999',
      access: 'read_only' as const,
      created_at: 1760000100,
    }
    api.createToken.mockResolvedValue({
      ...initialState,
      revision: 3,
      tokens: [...initialState.tokens, createdToken],
      created_token: createdToken,
    })
    api.saveKnowledge.mockResolvedValue({
      ...initialState,
      revision: 4,
      enabled: true,
      allow_write: false,
    })

    const wrapper = mount(SystemMcpPage, {
      global: { stubs: { RouterLink: true } },
    })
    await flushPromises()

    await wrapper.get('[data-testid="new-token-name-input"]').setValue('Cursor Agent')
    await wrapper.get('[data-testid="new-token-access-select"]').setValue('read_only')
    await wrapper.get('[data-testid="create-token-submit-btn"]').trigger('submit')
    await flushPromises()

    expect(api.createToken).toHaveBeenCalledWith({
      name: 'Cursor Agent',
      access: 'read_only',
      custom_token: undefined,
    })
    expect(wrapper.get('[data-testid="newly-created-token-banner"]').text()).toContain(
      'kmcp_cursor_secret_999999',
    )

    await wrapper.get('[data-testid="mcp-allow-write-checkbox"]').setValue(false)
    await wrapper.get('[data-testid="save-mcp-config-btn"]').trigger('submit')
    await flushPromises()

    expect(api.saveKnowledge).toHaveBeenCalledWith(
      expect.objectContaining({
        enabled: true,
        allow_write: false,
      }),
    )
    wrapper.unmount()
  })
})
