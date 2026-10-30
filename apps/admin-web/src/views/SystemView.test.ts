import { createPinia, setActivePinia } from 'pinia'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { systemApi } from '@/api/system'
import { assistantApi } from '@/api/assistant'
import { ApiError } from '@/api/errors'
import type { SystemResources } from '@/api/types/system'
import { useServerStore } from '@/stores/server'
import ResourceStatCard from '@/components/system/ResourceStatCard.vue'
import AssistantDeviceSettings from '@/components/system/AssistantDeviceSettings.vue'
import DatabaseBackupCard from '@/components/system/DatabaseBackupCard.vue'
import SystemView from './SystemView.vue'

const mockLoadAll = vi.fn()

vi.mock('@/api/system', () => ({
  systemApi: {
    health: vi.fn(),
    ready: vi.fn(),
    status: vi.fn(),
    resources: vi.fn(),
    exportBackup: vi.fn(),
    inspectBackup: vi.fn(),
    importBackup: vi.fn(),
  },
}))
vi.mock('@/api/assistant', () => ({ assistantApi: { settings: vi.fn(), save: vi.fn(), ticket: vi.fn() } }))
vi.mock('@/stores/assistant', () => ({ useAssistantStore: () => ({ open: vi.fn() }) }))
vi.mock('@/stores/admin', () => ({ useAdminStore: () => ({
  agents: [{ id: 'agent-one', name: 'One' }, { id: 'agent-two', name: 'Two' }],
  getTemplatesForAgent: () => [], getDefaultTemplate: () => undefined,
  loadAll: mockLoadAll,
}) }))

function snapshot(): SystemResources {
  return {
    sampled_at: new Date().toISOString(), sample_interval_ms: 5000,
    system: { cpu_percent: 34.5, logical_cores: 8, memory_total_bytes: 16 * 1024 ** 3, memory_used_bytes: 10 * 1024 ** 3 },
    process: { pid: 123, cpu_percent: 242.8, rss_bytes: 2 * 1024 ** 3 },
    storage: { total_bytes: 100 * 1024 ** 3, available_bytes: 40 * 1024 ** 3 },
  }
}
function page() {
  return mount(SystemView, { global: { plugins: [createPinia()], stubs: { RouterLink: true } } })
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-10-11T00:00:00Z'))
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  setActivePinia(createPinia())
  vi.mocked(systemApi.health).mockResolvedValue('ok')
  vi.mocked(systemApi.ready).mockResolvedValue('ready')
  vi.mocked(systemApi.status).mockResolvedValue({ status: 'ready', version: '0.1.0', uptime_seconds: 20,
    database: { enabled: true, status: 'ready' }, providers: { configured: 3, loaded: 2, failed: 1, stale: 0 },
    sessions: { active: 4 }, provider_runtime: { accounting: { reserved_bytes: 1024 ** 3, active_leases: 2 } } })
  vi.mocked(systemApi.resources).mockImplementation(async () => snapshot())
  vi.mocked(assistantApi.settings).mockResolvedValue({ configured: true, enabled: true, agent_key: 'agent-one', template_key: null, revision: 1, device_id: 'studio', active_connections: 0 })
})
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.clearAllMocks() })

describe('System resource monitoring', () => {
  it('renders four cards and raw multicore process CPU without a process gauge', async () => {
    const wrapper = page()
    await flushPromises()
    const cards = wrapper.findAllComponents(ResourceStatCard)
    expect(cards).toHaveLength(4)
    expect(cards[0]!.text()).toContain('34.5%')
    expect(cards[1]!.text()).toContain('10.0 GiB / 16.0 GiB')
    expect(cards[2]!.text()).toContain('242.8%')
    expect(cards[2]!.find('[role="progressbar"]').exists()).toBe(false)
    expect(cards[3]!.text()).toContain('60.0%')
    expect(wrapper.text()).toContain('Estimated reservation: 1.0 GiB')
    wrapper.unmount()
  })

  it('pauses hidden polling, resumes immediately, refreshes status every 30 seconds and aborts on unmount', async () => {
    const wrapper = page()
    await flushPromises()
    await vi.advanceTimersByTimeAsync(5000)
    expect(systemApi.resources).toHaveBeenCalledTimes(2)
    expect(systemApi.status).toHaveBeenCalledTimes(1)
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(true)
    await vi.advanceTimersByTimeAsync(20_000)
    expect(systemApi.resources).toHaveBeenCalledTimes(2)
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
    document.dispatchEvent(new Event('visibilitychange'))
    await flushPromises()
    expect(systemApi.resources).toHaveBeenCalledTimes(3)
    await vi.advanceTimersByTimeAsync(5000)
    expect(systemApi.status).toHaveBeenCalledTimes(2)
    const signal = vi.mocked(systemApi.resources).mock.calls[0]![0]!
    wrapper.unmount()
    expect(signal.aborted).toBe(true)
    const count = vi.mocked(systemApi.resources).mock.calls.length
    document.dispatchEvent(new Event('visibilitychange'))
    await vi.advanceTimersByTimeAsync(30_000)
    expect(systemApi.resources).toHaveBeenCalledTimes(count)
  })

  it('skips overlapping requests and preserves unsaved Assistant settings across polling and refresh', async () => {
    let resolve!: (value: SystemResources) => void
    vi.mocked(systemApi.resources).mockReturnValueOnce(new Promise(done => { resolve = done }))
    const wrapper = page()
    await flushPromises()
    const settings = wrapper.findComponent(AssistantDeviceSettings)
    const form = settings.find('form').element
    await settings.findAll('select')[0]!.setValue('agent-two')
    await vi.advanceTimersByTimeAsync(10_000)
    expect(systemApi.resources).toHaveBeenCalledTimes(1)
    resolve(snapshot())
    await flushPromises()
    await vi.advanceTimersByTimeAsync(5000)
    await wrapper.find('button').trigger('click')
    await flushPromises()
    expect(wrapper.findComponent(AssistantDeviceSettings).find('form').element).toBe(form)
    expect(settings.findAll('select')[0]!.element.value).toBe('agent-two')
    expect(assistantApi.settings).toHaveBeenCalledTimes(1)
    expect(assistantApi.save).not.toHaveBeenCalled()
    expect(assistantApi.ticket).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('shows nullable and stale readings without inventing zero values', async () => {
    const data = snapshot()
    data.system.cpu_percent = null
    data.system.memory_total_bytes = 0
    data.system.memory_used_bytes = null
    data.storage.available_bytes = null
    vi.mocked(systemApi.resources).mockResolvedValue(data)
    const wrapper = page()
    await flushPromises()
    const cards = wrapper.findAllComponents(ResourceStatCard)
    expect(cards[0]!.text()).toContain('—')
    expect(cards[1]!.find('[role="progressbar"]').exists()).toBe(false)
    expect(cards[3]!.text()).toContain('Unavailable')
    await vi.advanceTimersByTimeAsync(20_000)
    expect(cards[2]!.text()).toContain('Stale')
    expect(cards[2]!.text()).not.toContain('242.8%')
    wrapper.unmount()
  })

  it.each([401, 503])('keeps health and runtime status when resources returns %s', async status => {
    vi.mocked(systemApi.resources).mockRejectedValue(new ApiError('failed', status, 'system_metrics_unavailable'))
    const wrapper = page()
    await flushPromises()
    expect(useServerStore().health).toBe('online')
    expect(useServerStore().status?.sessions.active).toBe(4)
    expect(wrapper.text()).toContain(status === 401 ? 'Admin authentication required' : 'Unavailable')
    wrapper.unmount()
  })

  it('clamps gauges and omits invalid percentages', () => {
    const wrapper = mount(ResourceStatCard, { props: { title: 'CPU', value: '100%', percentage: 200 } })
    expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBe('100')
    wrapper.unmount()
    const invalid = mount(ResourceStatCard, { props: { title: 'CPU', value: '—', percentage: NaN } })
    expect(invalid.find('[role="progressbar"]').exists()).toBe(false)
    invalid.unmount()
  })

  it('exports backup and inspects + imports a backup file with admin store reload', async () => {
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    vi.mocked(systemApi.exportBackup).mockResolvedValue({
      blob: new Blob(['{}'], { type: 'application/json' }),
      filename: 'voice-agent-backup.json',
    })
    const sampleInspection = {
      detected_format: 'json' as const,
      bundle_scope: 'full' as const,
      effective_scope: 'full' as const,
      mode: 'merge' as const,
      created_at: 1700000000000,
      server_version: '0.1.0',
      includes_background_settings: true,
      includes_knowledge_mcp_settings: true,
      resources: [
        { resource: 'agents', in_backup: 2, to_create: 1, to_update: 1, to_delete: 0 },
      ],
      totals: { to_create: 1, to_update: 1, to_delete: 0, unchanged: 0 },
      credentials: {
        total_encrypted: 1,
        decryptable: 0,
        unreadable: 1,
        unreadable_items: [{ resource_type: 'provider', resource_key: 'llm-1', action: 'clear_to_null' }],
      },
      warnings: ['1 credential could not be decrypted and will be scrubbed on import.'],
    }
    vi.mocked(systemApi.inspectBackup).mockResolvedValue(sampleInspection)
    vi.mocked(systemApi.importBackup).mockResolvedValue({
      detected_format: 'json',
      effective_scope: 'full',
      mode: 'merge',
      created_at: 1700000000000,
      server_version: '0.1.0',
      applied_background_settings: true,
      applied_knowledge_mcp_settings: true,
      resources: sampleInspection.resources,
      totals: sampleInspection.totals,
      credentials: sampleInspection.credentials,
      warnings: sampleInspection.warnings,
    })

    const wrapper = mount(DatabaseBackupCard, { global: { plugins: [createPinia()] } })
    await wrapper.get('[data-testid="backup-export-format"]').setValue('sqlite')
    await wrapper.get('[data-testid="backup-export-scope"]').setValue('config_only')
    await wrapper.get('[data-testid="backup-export-button"]').trigger('click')
    await flushPromises()
    expect(systemApi.exportBackup).toHaveBeenCalledWith('sqlite', 'config_only')

    const file = new File(['{"format":"voice-agent-backup"}'], 'backup.json', { type: 'application/json' })
    const input = wrapper.get('[data-testid="backup-file-input"]')
    Object.defineProperty(input.element, 'files', { value: [file], configurable: true })
    await input.trigger('change')
    await flushPromises()

    expect(systemApi.inspectBackup).toHaveBeenCalledTimes(1)
    expect(wrapper.get('[data-testid="backup-inspection-summary"]').text()).toContain('agents')
    expect(wrapper.get('[data-testid="backup-inspection-warnings"]').text()).toContain('1 credential')

    await wrapper.get('[data-testid="backup-import-button"]').trigger('click')
    await flushPromises()
    const confirmDialog = wrapper.findComponent({ name: 'ConfirmDialog' })
    confirmDialog.vm.$emit('confirm')
    await flushPromises()

    expect(systemApi.importBackup).toHaveBeenCalledTimes(1)
    expect(mockLoadAll).toHaveBeenCalledTimes(1)
    expect(wrapper.find('[data-testid="backup-import-success"]').exists()).toBe(true)
    wrapper.unmount()
  })
})
