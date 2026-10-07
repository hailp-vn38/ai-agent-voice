import { nextTick, watch } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const agentsApi = vi.hoisted(() => ({ list: vi.fn(), templates: vi.fn(), setDefaultTemplate: vi.fn() }))
const templatesApi = vi.hoisted(() => ({ list: vi.fn(), providers: vi.fn() }))
const providersApi = vi.hoisted(() => ({ list: vi.fn() }))
const devicesApi = vi.hoisted(() => ({ list: vi.fn() }))

vi.mock('@/api/agents', () => ({ agentsApi }))
vi.mock('@/api/templates', () => ({ templatesApi }))
vi.mock('@/api/providers', () => ({ providersApi }))
vi.mock('@/api/devices', () => ({ devicesApi }))

import { useAdminStore } from './admin'

describe('admin store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.resetAllMocks()
    agentsApi.list.mockResolvedValue({ items: [{ key: 'agent', name: 'Agent', description: null, enabled: true, revision: 1 }] })
    agentsApi.templates
      .mockResolvedValueOnce({
        items: [
          { key: 'first', name: 'First', language: 'vi-VN', enabled: true, is_default: true },
          { key: 'next', name: 'Next', language: 'vi-VN', enabled: true, is_default: false },
        ],
        revision: 1,
      })
      .mockResolvedValueOnce({
        items: [
          { key: 'first', name: 'First', language: 'vi-VN', enabled: true, is_default: false },
          { key: 'next', name: 'Next', language: 'vi-VN', enabled: true, is_default: true },
        ],
        revision: 2,
      })
    templatesApi.list.mockResolvedValue({
      items: [
        { key: 'first', name: 'First', description: null, language: 'vi-VN', prompt: '', enabled: true, revision: 1 },
        { key: 'next', name: 'Next', description: null, language: 'vi-VN', prompt: '', enabled: true, revision: 1 },
      ],
    })
    templatesApi.providers.mockResolvedValue({ bindings: {} })
    providersApi.list.mockResolvedValue({ items: [] })
    devicesApi.list.mockResolvedValue({ items: [] })
    agentsApi.setDefaultTemplate.mockResolvedValue(undefined)
  })

  it('updates Agent Detail reactively after the server accepts a new default template', async () => {
    const store = useAdminStore()
    await store.loadAll()
    const renderedDefaults: string[] = []
    const stop = watch(
      () => store.getAgent('agent')?.defaultTemplateId,
      (defaultTemplateId) => renderedDefaults.push(defaultTemplateId ?? ''),
      { immediate: true },
    )

    await store.setAgentDefaultTemplate('agent', 'next')
    await nextTick()
    stop()

    expect(agentsApi.setDefaultTemplate).toHaveBeenCalledWith('agent', 'next', 1)
    expect(renderedDefaults).toEqual(['first', 'next'])
    expect(store.getAgent('agent')?.defaultTemplateId).toBe('next')
    expect(store.revisions.agents.agent).toBe(2)
  })
})
