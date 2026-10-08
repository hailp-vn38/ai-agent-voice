import { nextTick, watch } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const agentsApi = vi.hoisted(() => ({ list: vi.fn(), templates: vi.fn(), setDefaultTemplate: vi.fn() }))
const templatesApi = vi.hoisted(() => ({ list: vi.fn(), providers: vi.fn() }))
const providersApi = vi.hoisted(() => ({ list: vi.fn(), update: vi.fn() }))
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

  it('preserves speaker config when the metadata form sends generic provider fields', async () => {
    const provider = { id: 3, key: 'speaker_test', name: 'Voice', type: 'speaker', adapter: 'campplus_sherpa', config_json: '{"min_speech_ms":3000,"target_speech_ms":4000,"max_window_ms":6000}', enabled: 1, revision: 1, runtime_status: 'not_loaded' }
    providersApi.list.mockResolvedValue({ items: [provider] })
    providersApi.update.mockResolvedValue({ ...provider, name: 'Renamed', revision: 2 })
    const store = useAdminStore()
    await store.loadAll()
    await store.updateProvider('speaker_test', { name: 'Renamed', model: '', description: 'Voice description', endpoint: undefined, status: 'ready' })
    expect(providersApi.update).toHaveBeenCalledWith('speaker_test', {
      name: 'Renamed', adapter: undefined, config_json: undefined, enabled: true,
    }, 1)
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
