import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import McpToolCatalog from './McpToolCatalog.vue'
it('renders remote text safely and searches original names and descriptions', async () => {
  const wrapper = mount(McpToolCatalog, { props: { tools: [{ original_name: 'weather', llm_name: 'external.home.weather', description: '<img src=x onerror=alert(1)> forecast', input_schema: { type: 'object' } }, { original_name: 'light', llm_name: 'external.home.light', description: 'Switch lights', input_schema: { type: 'object' } }] } })
  expect(wrapper.find('img').exists()).toBe(false)
  expect(wrapper.text()).toContain('<img src=x onerror=alert(1)>')
  await wrapper.get('input[type="search"]').setValue('forecast')
  expect(wrapper.text()).toContain('weather')
  expect(wrapper.text()).not.toContain('Switch lights')
})
