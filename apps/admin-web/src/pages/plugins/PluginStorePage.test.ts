import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import PluginStorePage from './PluginStorePage.vue'
import { ApiError } from '@/api/errors'
import { setLocale, translate } from '@/composables/useI18n'

const { api, push } = vi.hoisted(() => ({
  api: { list: vi.fn(), install: vi.fn(), builtinList: vi.fn(), builtinSetEnabled: vi.fn() },
  push: vi.fn(),
}))
vi.mock('@/api/agent-plugins', () => ({ agentPluginsApi: api }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }))

beforeEach(() => {
  vi.clearAllMocks()
  setLocale('en')
  api.list.mockResolvedValue({ items: [], total: 0, page: 1, page_size: 30 })
  api.install.mockResolvedValue({ id: 'plugin-1' })
  api.builtinList.mockResolvedValue({ items: [
    { id: 'core.web', name: 'Core Web', kind: 'built_in', tags: ['core'],
      enabled: true, status: 'ready', revision: 1,
      runtime: { kind: 'in_process', language: 'rust' },
      operations: [{ name: 'web.fetch', contract_revision: '1' }, { name: 'web.extract', contract_revision: '1' }] },
  ], total: 1, operation_total: 2 })
  api.builtinSetEnabled.mockResolvedValue({
    id: 'core.web', name: 'Core Web', kind: 'built_in', tags: ['core'],
    enabled: false, status: 'disabled', revision: 2,
    runtime: { kind: 'in_process', language: 'rust' },
    operations: [{ name: 'web.fetch' }, { name: 'web.extract' }],
  })
})
afterEach(() => { document.body.innerHTML = '' })

async function render() {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.installZip'))!.trigger('click')
  return wrapper
}

it('selects a dropped ZIP and uploads only after clicking Install', async () => {
  const wrapper = await render()
  const file = new File(['zip'], 'plugin.ZIP', { type: 'application/zip' })
  const zone = wrapper.get('.border-dashed')
  await zone.trigger('dragover')
  expect(wrapper.get('.border-dashed').classes()).toContain('border-studio-violet')
  await wrapper.get('.border-dashed').trigger('drop', { dataTransfer: { files: [file] } })
  expect(wrapper.text()).toContain(file.name)
  expect(wrapper.get('.border-dashed').classes()).not.toContain('border-studio-violet')
  expect(api.install).not.toHaveBeenCalled()
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.install'))!.trigger('click')
  await flushPromises()
  expect(api.install).toHaveBeenCalledWith(file)
  expect(push).toHaveBeenCalledWith({ name: 'plugin-detail', params: { id: 'plugin-1' } })
  wrapper.unmount()
})

it.each(['invalid.txt', 'oversized.zip'])('rejects a dropped %s and clears the previous selection', async (name) => {
  const wrapper = await render()
  const input = wrapper.get('input[type="file"]')
  Object.defineProperty(input.element, 'files', { value: [new File(['zip'], 'valid.zip')] })
  await input.trigger('change')
  expect(wrapper.text()).toContain('valid.zip')
  const file = new File(['invalid'], name)
  if (name === 'oversized.zip') Object.defineProperty(file, 'size', { value: 20 * 1024 * 1024 + 1 })
  await wrapper.get('.border-dashed').trigger('drop', { dataTransfer: { files: [file] } })
  expect(wrapper.text()).toContain(translate('jobUi.invalidZip'))
  expect(wrapper.text()).not.toContain('valid.zip')
  expect(wrapper.findAll('button').find(button => button.text() === translate('jobUi.install'))!.attributes('disabled')).toBeDefined()
  expect(api.install).not.toHaveBeenCalled()
  wrapper.unmount()
})

it('ignores drops without files and changes while uploading', async () => {
  const wrapper = await render()
  const zone = wrapper.get('.border-dashed')
  const file = new File(['zip'], 'selected.zip')
  await zone.trigger('drop', { dataTransfer: { files: [file] } })
  await zone.trigger('drop', { dataTransfer: { files: [] } })
  expect(wrapper.text()).toContain(file.name)
  let finish!: (value: { id: string }) => void
  api.install.mockImplementation(() => new Promise(resolve => { finish = resolve }))
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.install'))!.trigger('click')
  expect(wrapper.get('input[type="file"]').attributes('disabled')).toBeDefined()
  await zone.trigger('drop', { dataTransfer: { files: [new File(['zip'], 'replacement.zip')] } })
  expect(wrapper.text()).toContain(file.name)
  expect(wrapper.text()).not.toContain('replacement.zip')
  finish({ id: 'plugin-1' })
  await flushPromises()
  wrapper.unmount()
})

it.each(['close', 'failed install', 'successful install'])('opens an empty install modal after %s', async (action) => {
  const wrapper = await render()
  const file = new File(['zip'], 'previous.zip')
  await wrapper.get('.border-dashed').trigger('drop', { dataTransfer: { files: [file] } })
  await wrapper.get('.border-dashed').trigger('dragover')
  if (action === 'failed install') {
    api.install.mockRejectedValueOnce(new Error('upload failed'))
    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.install'))!.trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="dialog"]').text()).toContain('upload failed')
  }
  if (action === 'successful install') {
    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.install'))!.trigger('click')
    await flushPromises()
  } else {
    await wrapper.get('[role="dialog"]').findAll('button').find(button => button.text() === translate('jobUi.close'))!.trigger('click')
  }
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.installZip'))!.trigger('click')
  const dialog = wrapper.get('[role="dialog"]')
  expect(dialog.text()).not.toContain(file.name)
  expect(dialog.find('[role="alert"]').exists()).toBe(false)
  expect(dialog.get('.border-dashed').classes()).not.toContain('border-studio-violet')
  expect((dialog.get('input[type="file"]').element as HTMLInputElement).value).toBe('')
  expect(dialog.findAll('button').find(button => button.text() === translate('jobUi.install'))!.attributes('disabled')).toBeDefined()
  wrapper.unmount()
})

it('renders built-in CORE separately and toggles it with its revision', async () => {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  expect(wrapper.text()).toContain('Core Web')
  expect(wrapper.text()).toContain('web.fetch')
  await wrapper.get('button[aria-label="Disable Core Web"]').trigger('click')
  await flushPromises()
  expect(api.builtinSetEnabled).toHaveBeenCalledWith('core.web', false, 1)
  expect(wrapper.find('button[aria-label="Enable Core Web"]').exists()).toBe(true)
  expect(api.install).not.toHaveBeenCalled()
  wrapper.unmount()
})

it('refreshes Core revision on conflicting PATCH rather than assuming success', async () => {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  api.builtinSetEnabled.mockRejectedValueOnce(new ApiError('revision_conflict', 409, 'revision_conflict'))
  await wrapper.get('button[aria-label="Disable Core Web"]').trigger('click')
  await flushPromises()
  expect(api.builtinList).toHaveBeenCalledTimes(2)
  expect(wrapper.find('button[aria-label="Disable Core Web"]').exists()).toBe(true)
  wrapper.unmount()
})

it('keeps the committed PATCH revision if a concurrent refresh returns an older snapshot', async () => {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  let finishPatch!: (value: unknown) => void
  let finishRefresh!: (value: unknown) => void
  api.builtinSetEnabled.mockImplementationOnce(() =>
    new Promise(resolve => { finishPatch = resolve }),
  )
  await wrapper.get('button[aria-label="Disable Core Web"]').trigger('click')
  api.builtinList.mockImplementationOnce(() =>
    new Promise(resolve => { finishRefresh = resolve }),
  )
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.refresh'))!.trigger('click')
  finishPatch({
    id: 'core.web', name: 'Core Web', kind: 'built_in', tags: ['core'],
    enabled: false, status: 'disabled', revision: 2,
    runtime: { kind: 'in_process', language: 'rust' },
    operations: [{ name: 'web.fetch' }, { name: 'web.extract' }],
  })
  await flushPromises()
  finishRefresh({
    items: [{
      id: 'core.web', name: 'Core Web', kind: 'built_in', tags: ['core'],
      enabled: true, status: 'ready', revision: 1,
      runtime: { kind: 'in_process', language: 'rust' },
      operations: [{ name: 'web.fetch' }, { name: 'web.extract' }],
    }],
    total: 1, operation_total: 2,
  })
  await flushPromises()
  expect(wrapper.find('button[aria-label="Enable Core Web"]').exists()).toBe(true)
  expect(wrapper.text()).toContain(translate('jobUi.coreDisabled'))
  wrapper.unmount()
})

it('localizes built-in Core labels and actions in Vietnamese', async () => {
  setLocale('vi')
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  expect(wrapper.text()).toContain(translate('jobUi.installedHint'))
  expect(wrapper.text()).toContain(translate('jobUi.coreEnabled'))
  await wrapper.get('button[aria-label="Tắt Core Web"]').trigger('click')
  await flushPromises()
  expect(wrapper.find('button[aria-label="Bật Core Web"]').exists()).toBe(true)
  wrapper.unmount()
})

it('shows Core and installed external plugins together on the Installed tab by default', async () => {
  api.list.mockResolvedValueOnce({
    items: [{
      id: 'external-demo', name: 'External Demo', description: 'Example worker plugin',
      version: '1.0.0', status: 'ready', revision: 1,
      runtime: { kind: 'shared_process', language: 'python' },
      operations: [{ name: 'example.run' }],
    }],
    total: 1, page: 1, page_size: 30,
  })
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  expect(wrapper.get('[role="tab"][aria-selected="true"]').text())
    .toContain(translate('jobUi.installedTab'))
  const panel = wrapper.get('[role="tabpanel"]')
  expect(panel.text()).toContain('Core Web')
  expect(panel.text()).toContain('External Demo')
  expect(panel.text()).toContain(translate('jobUi.externalLabel'))
  expect(panel.text()).toContain('CORE')
  expect(panel.text()).not.toContain(translate('jobUi.availableHint'))
  wrapper.unmount()
})

it('moves all Core plugins out of the Not installed tab and keeps ZIP installation accessible', async () => {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  await wrapper.findAll('[role="tab"]')
    .find(tab => tab.text().includes(translate('jobUi.notInstalledTab')))!.trigger('click')
  const panel = wrapper.get('[role="tabpanel"]')
  expect(panel.text()).toContain(translate('jobUi.availableHint'))
  expect(panel.text()).not.toContain('Core Web')
  await panel.findAll('button')
    .find(button => button.text().includes(translate('jobUi.installZip')))!.trigger('click')
  expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
  expect(api.install).not.toHaveBeenCalled()
  await wrapper.findAll('[role="tab"]')
    .find(tab => tab.text().includes(translate('jobUi.installedTab')))!.trigger('click')
  expect(wrapper.get('[role="tabpanel"]').text()).toContain('Core Web')
  wrapper.unmount()
})

it('applies installed search and status filter to Core without changing its activation state', async () => {
  const wrapper = mount(PluginStorePage, { global: { stubs: { teleport: true } } })
  await flushPromises()
  await wrapper.get('input[type="search"]').setValue('document')
  expect(wrapper.get('[role="tabpanel"]').text()).not.toContain('Core Web')
  await wrapper.findAll('button').find(button => button.text() === translate('jobUi.search'))!.trigger('click')
  await flushPromises()
  expect(api.list).toHaveBeenLastCalledWith(
    { page: 1, q: 'document', status: undefined },
    expect.anything(),
  )
  await wrapper.get('input[type="search"]').setValue('')
  await wrapper.get('select').setValue('disabled')
  await flushPromises()
  expect(wrapper.get('[role="tabpanel"]').text()).not.toContain('Core Web')
  expect(api.builtinSetEnabled).not.toHaveBeenCalled()
  wrapper.unmount()
})
