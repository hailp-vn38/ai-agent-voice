import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import DetailHeader from './DetailHeader.vue'

describe('DetailHeader', () => {
  it('renders the shared identity layout and emits its optional back action', async () => {
    const wrapper = mount(DetailHeader, {
      props: { title: 'Home', backLabel: 'Agents' },
      slots: {
        icon: '<span data-header-icon>Icon</span>',
        details: '<p>Voice AI</p>',
        actions: '<button data-header-action>Save</button>',
      },
    })

    expect(wrapper.get('[data-detail-header]').text()).toContain('Home')
    expect(wrapper.find('[data-header-icon]').exists()).toBe(true)
    expect(wrapper.find('[data-header-action]').exists()).toBe(true)
    await wrapper.get('[data-detail-back]').trigger('click')
    expect(wrapper.emitted('back')).toEqual([[]])
  })
})
