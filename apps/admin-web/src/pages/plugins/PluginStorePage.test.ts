import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import PluginStorePage from './PluginStorePage.vue'
import { setLocale, translate } from '@/composables/useI18n'

const { api, push } = vi.hoisted(() => ({
  api: { list: vi.fn(), install: vi.fn() },
  push: vi.fn(),
}))
vi.mock('@/api/agent-plugins', () => ({ agentPluginsApi: api }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }))

beforeEach(() => {
  vi.clearAllMocks()
  setLocale('en')
  api.list.mockResolvedValue({ items: [], total: 0, page: 1, page_size: 30 })
  api.install.mockResolvedValue({ id: 'plugin-1' })
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
