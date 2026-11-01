import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { devicesApi } from '@/api/devices'
import { ApiError } from '@/api/errors'
import { memoryApi, type MemoryFact } from '@/api/memory'
import type { AdminDevice } from '@/api/types/devices'
import { setLocale } from '@/composables/useI18n'
import MemoryPage from './MemoryPage.vue'

vi.mock('@/api/devices', () => ({ devicesApi: { list: vi.fn() } }))
vi.mock('@/api/memory', () => ({
  FACT_KINDS: ['user', 'preference', 'business', 'decision', 'reference'],
  memoryApi: {
    facts: vi.fn(),
    fact: vi.fn(),
    create: vi.fn(),
    update: vi.fn(),
    forget: vi.fn(),
    discard: vi.fn(),
    pin: vi.fn(),
    summary: vi.fn(),
    index: vi.fn(),
    rebuild: vi.fn(),
    profile: vi.fn(),
    saveProfile: vi.fn(),
  },
}))

const STORAGE_KEY = 'memory.selectedDevice'

function device(id: string, memoryMode: AdminDevice['memory_mode'] = 'off'): AdminDevice {
  return {
    id: 1, device_id: id, agent_key: 'home', template_key: null, name: `Name ${id}`, description: null,
    metadata_json: null, memory_mode: memoryMode, enabled: 1, revision: 1,
  }
}

function fact(overrides: Partial<MemoryFact> = {}): MemoryFact {
  return {
    id: 'fact_a', revision: 3, title: 'Lịch họp', hook: 'họp thứ hai', body: 'Họp thứ hai', kind: 'business',
    status: 'confirmed', pinned: false, origin: 'admin', created_at: '2026-01-01T00:00:00.000Z',
    updated_at: '2026-01-01T00:00:00.000Z', ...overrides,
  }
}

function receipt(action: string, cleanupPending = false) {
  return { action, fact_id: 'fact_a', title: 'Lịch họp', revision: 3, at: 'now', cleanup_pending: cleanupPending }
}

const conflict = () => new ApiError('conflict', 409, 'memory_revision_conflict')

async function mountWithDevice(id = 'kitchen') {
  localStorage.setItem(STORAGE_KEY, id)
  const wrapper = mount(MemoryPage)
  await flushPromises()
  return wrapper
}

async function openFirstFact(wrapper: Awaited<ReturnType<typeof mountWithDevice>>) {
  await wrapper.get('[data-fact-open]').trigger('click')
  await flushPromises()
}

describe('MemoryPage', () => {
  beforeEach(() => {
    setLocale('en')
    localStorage.clear()
    vi.clearAllMocks()
    vi.mocked(devicesApi.list).mockResolvedValue({
      items: [device('kitchen'), device('bedroom', 'read_write')], total: 2, page: 1, page_size: 200,
    } as never)
    vi.mocked(memoryApi.facts).mockResolvedValue({ items: [fact()] })
    vi.mocked(memoryApi.fact).mockResolvedValue(fact())
    vi.mocked(memoryApi.summary).mockResolvedValue({ summary: '{"title":"Lịch họp"}', hidden: 0 })
    vi.mocked(memoryApi.index).mockResolvedValue({ facts: 1, quarantined: 2, quarantined_files: ['bad.md'], memory_mode: 'off' })
    vi.mocked(memoryApi.profile).mockResolvedValue({ body: 'Hồ sơ', revision: 4, updated_at: null })
    vi.spyOn(window, 'confirm').mockReturnValue(true)
  })

  it('calls no Device-scoped API until a Device is chosen, then remembers the choice', async () => {
    const wrapper = mount(MemoryPage)
    await flushPromises()
    expect(memoryApi.facts).not.toHaveBeenCalled()
    expect(memoryApi.summary).not.toHaveBeenCalled()
    expect(memoryApi.index).not.toHaveBeenCalled()

    await wrapper.get('[data-memory-device]').setValue('bedroom')
    await flushPromises()
    expect(memoryApi.facts).toHaveBeenCalledWith('bedroom', undefined)
    expect(memoryApi.summary).toHaveBeenCalledWith('bedroom')
    expect(localStorage.getItem(STORAGE_KEY)).toBe('bedroom')
    wrapper.unmount()

    vi.clearAllMocks()
    const again = mount(MemoryPage)
    await flushPromises()
    expect(memoryApi.facts).toHaveBeenCalledWith('bedroom', undefined)
    again.unmount()
  })

  it('ignores a remembered Device that no longer exists', async () => {
    const wrapper = await mountWithDevice('deleted-device')
    expect(memoryApi.facts).not.toHaveBeenCalled()
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull()
    wrapper.unmount()
  })

  it('shows real fact state, quarantine counts and the partition and provider notice', async () => {
    const wrapper = await mountWithDevice()
    vi.mocked(memoryApi.facts).mockResolvedValue({ items: [fact({ pinned: true })] })
    await wrapper.get('[data-memory-search]').trigger('submit')
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('r3')
    expect(text).toContain('confirmed')
    expect(text).toContain('business')
    expect(wrapper.find('[data-fact-pinned]').exists()).toBe(true)
    expect(wrapper.get('[data-memory-quarantined]').text()).toContain('2')
    expect(text).toContain('bad.md')
    const notice = wrapper.get('[data-memory-notice]').text()
    expect(notice).toMatch(/per Device/i)
    expect(notice).toMatch(/not a security/i)
    expect(notice).toMatch(/LLM provider/i)
    wrapper.unmount()
  })

  it('searches the chosen Device', async () => {
    const wrapper = await mountWithDevice()
    await wrapper.get('[data-memory-query]').setValue('  dat lich ')
    await wrapper.get('[data-memory-search]').trigger('submit')
    await flushPromises()
    expect(memoryApi.facts).toHaveBeenLastCalledWith('kitchen', 'dat lich')
    wrapper.unmount()
  })

  it('creates a fact with hook and kind, and offers no confirmed or source fields', async () => {
    vi.mocked(memoryApi.create).mockResolvedValue(receipt('remember') as never)
    const wrapper = await mountWithDevice()
    expect(wrapper.find('[data-fact-confirmed]').exists()).toBe(false)
    expect(wrapper.find('[data-fact-source]').exists()).toBe(false)
    await wrapper.get('[data-fact-title]').setValue('Sở thích')
    await wrapper.get('[data-fact-hook]').setValue('thích cà phê')
    await wrapper.get('[data-fact-kind]').setValue('preference')
    await wrapper.get('[data-fact-body]').setValue('Thích cà phê đen')
    await wrapper.get('[data-fact-form]').trigger('submit')
    await flushPromises()
    expect(memoryApi.create).toHaveBeenCalledWith('kitchen', {
      title: 'Sở thích', hook: 'thích cà phê', kind: 'preference', body: 'Thích cà phê đen',
    })
    wrapper.unmount()
  })

  it('sends expected_revision on edit', async () => {
    vi.mocked(memoryApi.update).mockResolvedValue(receipt('correct') as never)
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-body]').setValue('Họp thứ ba')
    await wrapper.get('[data-fact-form]').trigger('submit')
    await flushPromises()
    expect(memoryApi.update).toHaveBeenCalledWith('kitchen', 'fact_a', {
      title: 'Lịch họp', hook: 'họp thứ hai', body: 'Họp thứ ba', expected_revision: 3,
    })
    wrapper.unmount()
  })

  it('on a 409 it reports the conflict and reloads the fact instead of overwriting', async () => {
    vi.mocked(memoryApi.update).mockRejectedValue(conflict())
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    vi.mocked(memoryApi.fact).mockResolvedValue(fact({ revision: 4, body: 'Bản của người khác' }))
    await wrapper.get('[data-fact-body]').setValue('Bản của tôi')
    await wrapper.get('[data-fact-form]').trigger('submit')
    await flushPromises()
    expect(memoryApi.update).toHaveBeenCalledTimes(1)
    expect(wrapper.get('[role="alert"]').text()).toMatch(/changed/i)
    expect((wrapper.get('[data-fact-body]').element as HTMLTextAreaElement).value).toBe('Bản của người khác')
    expect(wrapper.text()).toContain('r4')
    wrapper.unmount()
  })

  it('tells Undo and Forget apart in their confirmations and calls the matching route', async () => {
    vi.mocked(memoryApi.discard).mockResolvedValue(receipt('discard') as never)
    vi.mocked(memoryApi.forget).mockResolvedValue(receipt('forget') as never)
    const wrapper = await mountWithDevice()
    const confirm = vi.mocked(window.confirm)

    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-discard]').trigger('click')
    await flushPromises()
    expect(memoryApi.discard).toHaveBeenCalledWith('kitchen', 'fact_a', 3)
    expect(memoryApi.forget).not.toHaveBeenCalled()
    const discardMessage = confirm.mock.calls[0]![0]

    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-forget]').trigger('click')
    await flushPromises()
    expect(memoryApi.forget).toHaveBeenCalledWith('kitchen', 'fact_a', 3)
    const forgetMessage = confirm.mock.calls[1]![0]

    expect(discardMessage).not.toBe(forgetMessage)
    expect(discardMessage).toMatch(/save the same content again/i)
    expect(forgetMessage).toMatch(/tombstone|will not come back/i)
    wrapper.unmount()
  })

  it('does nothing when a confirmation is declined', async () => {
    vi.mocked(window.confirm).mockReturnValue(false)
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-forget]').trigger('click')
    await wrapper.get('[data-fact-discard]').trigger('click')
    await flushPromises()
    expect(memoryApi.forget).not.toHaveBeenCalled()
    expect(memoryApi.discard).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('pins without asking for a revision', async () => {
    vi.mocked(memoryApi.pin).mockResolvedValue(receipt('set_pinned') as never)
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-pin]').trigger('click')
    await flushPromises()
    expect(memoryApi.pin).toHaveBeenCalledWith('kitchen', 'fact_a', true)
    wrapper.unmount()
  })

  it('says so when a committed change left cleanup unfinished', async () => {
    vi.mocked(memoryApi.forget).mockResolvedValue(receipt('forget', true) as never)
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    await wrapper.get('[data-fact-forget]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="status"]').text()).toMatch(/rebuild/i)
    wrapper.unmount()
  })

  it('clears the open fact when the Device changes, so a save cannot land on the wrong Device', async () => {
    const wrapper = await mountWithDevice()
    await openFirstFact(wrapper)
    await wrapper.get('[data-memory-device]').setValue('bedroom')
    await flushPromises()
    expect((wrapper.get('[data-fact-body]').element as HTMLTextAreaElement).value).toBe('')
    expect(wrapper.find('[data-fact-forget]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('drops a slow answer from the Device that was left', async () => {
    let finishKitchen: (value: { items: MemoryFact[] }) => void = () => {}
    vi.mocked(memoryApi.facts).mockImplementation((id) => id === 'kitchen'
      ? new Promise((resolve) => { finishKitchen = resolve })
      : Promise.resolve({ items: [fact({ id: 'fact_b', title: 'Của phòng ngủ' })] }))
    const wrapper = await mountWithDevice()
    await wrapper.get('[data-memory-device]').setValue('bedroom')
    await flushPromises()
    finishKitchen({ items: [fact({ title: 'Của nhà bếp' })] })
    await flushPromises()
    expect(wrapper.text()).toContain('Của phòng ngủ')
    expect(wrapper.text()).not.toContain('Của nhà bếp')
    wrapper.unmount()
  })

  it('rebuilds the chosen Device and saves the shared profile with its revision', async () => {
    vi.mocked(memoryApi.rebuild).mockResolvedValue({ facts: 1, quarantined: 0, tombstones_applied: 0, owner_edited: 0, temp_files_removed: 0 })
    vi.mocked(memoryApi.saveProfile).mockResolvedValue(receipt('set_profile') as never)
    const wrapper = await mountWithDevice()
    await wrapper.get('[data-memory-rebuild]').trigger('click')
    await flushPromises()
    expect(memoryApi.rebuild).toHaveBeenCalledWith('kitchen')

    await wrapper.get('[data-profile-body]').setValue('Hồ sơ mới')
    await wrapper.get('[data-profile-form]').trigger('submit')
    await flushPromises()
    expect(memoryApi.saveProfile).toHaveBeenCalledWith('Hồ sơ mới', 4)
    wrapper.unmount()
  })

  it('reloads the profile on a 409 instead of overwriting it', async () => {
    vi.mocked(memoryApi.saveProfile).mockRejectedValue(conflict())
    const wrapper = await mountWithDevice()
    vi.mocked(memoryApi.profile).mockResolvedValue({ body: 'Bản mới hơn', revision: 5, updated_at: null })
    await wrapper.get('[data-profile-body]').setValue('Bản cũ')
    await wrapper.get('[data-profile-form]').trigger('submit')
    await flushPromises()
    expect((wrapper.get('[data-profile-body]').element as HTMLTextAreaElement).value).toBe('Bản mới hơn')
    expect(wrapper.get('[role="alert"]').text()).toMatch(/changed/i)
    wrapper.unmount()
  })
})
