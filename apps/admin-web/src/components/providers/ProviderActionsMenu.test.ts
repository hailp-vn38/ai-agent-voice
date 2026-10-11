import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import { MenuItem } from '@/components/ui/action-menu'
import ProviderActionsMenu from './ProviderActionsMenu.vue'
import PipelineActionsMenu from '@/components/pipeline/ProviderActionsMenu.vue'
import ProviderDetailModal from '@/components/admin/ProviderDetailModal.vue'

const provider = { id: 'config:llm:primary', name: 'Config', source: 'config' as const, type: 'llm' as const, adapter: 'openai', model: '', description: '', status: 'ready' as const }
const global = { stubs: {
  ActionMenu: { template: '<div><slot /></div>' },
  MenuItem: { props: ['disabled'], emits: ['select'], template: `<button :disabled="disabled" @click="$emit('select')"><slot /></button>` },
  BaseModal: { template: '<div><slot /></div>' }, ProviderTestPanel: true,
} }

it('catalog mutations are disabled and reject direct callbacks; view and link remain available', async () => {
  const wrapper = mount(ProviderActionsMenu, { props: { provider }, global })
  const items = wrapper.findAllComponents(MenuItem)
  const buttons = wrapper.findAll('button')
  expect(buttons.map((button) => button.attributes('disabled') !== undefined)).toEqual([false, true, false, true, true])
  for (const item of items) item.vm.$emit('select')
  for (const event of ['edit', 'delete', 'duplicate']) expect(wrapper.emitted(event)).toBeUndefined()
  await buttons[0]!.trigger('click')
  await buttons[2]!.trigger('click')
  expect(wrapper.emitted('view')).toBeDefined()
  expect(wrapper.emitted('link')).toBeDefined()
})

it('pipeline locks edit but preserves unlink', async () => {
  const wrapper = mount(PipelineActionsMenu, { props: { provider, type: 'llm' }, global })
  const buttons = wrapper.findAll('button')
  expect(buttons.map((button) => button.attributes('disabled') !== undefined)).toEqual([false, true, false])
  await buttons[2]!.trigger('click')
  expect(wrapper.emitted('unlink')).toEqual([['llm']])
})

it('shared detail modal cannot emit a config edit', async () => {
  const wrapper = mount(ProviderDetailModal, { props: { provider, modelValue: false }, global })
  const edit = wrapper.findAll('button').find((button) => /Edit/.test(button.text()))!
  expect(edit.attributes('disabled')).toBeDefined()
  await edit.trigger('click')
  expect(wrapper.emitted('edit')).toBeUndefined()
})
