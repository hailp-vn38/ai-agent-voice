import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import McpServerFormModal from './McpServerFormModal.vue'
import McpDiagnosticPanel from './McpDiagnosticPanel.vue'
import type { AdminMcpServer } from '@/api/types/mcp'

const server: AdminMcpServer = { key: 'weather', name: 'Weather', transport: 'streamable_http', url: 'https://example.test/mcp', headers: {}, auth: { type: 'bearer' }, credential_env: null, connect_timeout_ms: 5000, request_timeout_ms: 30000, enabled: true, revision: 1, created_at: 1, updated_at: 1 }
const stubs = { BaseModal: { template: '<div><slot /></div>' }, Button: { template: '<button><slot /></button>' } }

describe('MCP resource credentials', () => {
  it('keeps an existing key write-only and sends a replacement with the server payload', async () => {
    const wrapper = mount(McpServerFormModal, { props: { modelValue: true, saving: false, server: { ...server, credential: { id: 'credential', masked_key: '…7A91', key_version: 1, status: 'active' } } }, global: { stubs } })
    expect(wrapper.text()).toContain('…7A91')
    expect(wrapper.get('input[type="password"]').element).toHaveProperty('value', '')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')![0]![0]).not.toHaveProperty('api_key')
    await wrapper.get('input[type="password"]').setValue('mcp-secret-2222')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')![1]![0]).toHaveProperty('api_key', 'mcp-secret-2222')
    await wrapper.setProps({ modelValue: false })
    await wrapper.setProps({ modelValue: true })
    expect(wrapper.get('input[type="password"]').element).toHaveProperty('value', '')
    await wrapper.get('input[type="password"]').setValue('discard-this-secret')
    await wrapper.get('select').setValue('none')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')![2]![0]).not.toHaveProperty('api_key')
  })
})

it('does not probe the saved auth when an edited header is invalid', async () => {
  const wrapper = mount(McpServerFormModal, { props: { modelValue: true, saving: false, server }, global: { stubs: { ...stubs, McpDiagnosticPanel: true } } })
  expect(wrapper.findComponent(McpDiagnosticPanel).props('draft')!.auth).toEqual({ type: 'bearer' })
  await wrapper.get('select').setValue('header')
  expect(wrapper.findComponent(McpDiagnosticPanel).exists()).toBe(false)
})
