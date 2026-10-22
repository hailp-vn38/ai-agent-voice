import { mount } from '@vue/test-utils'
import { beforeEach, expect, it } from 'vitest'
import SidebarContent from './SidebarContent.vue'
import { navigationGroups } from '@/config/navigation'
import { setLocale, translate } from '@/composables/useI18n'

beforeEach(() => setLocale('en'))

function mountSidebar(collapsed: boolean, collapsible = true) {
  return mount(SidebarContent, {
    props: { collapsed, collapsible },
    global: {
      stubs: {
        RouterLink: {
          props: ['to'],
          template: '<a :href="to"><slot /></a>',
        },
        ThemeToggle: true,
        LanguageSwitcher: true,
      },
    },
  })
}

it('keeps all navigation icons available when the sidebar is collapsed', async () => {
  const wrapper = mountSidebar(true)
  try {
    const expected = navigationGroups.reduce((count, group) => count + group.items.length, 0)
    expect(wrapper.findAll('nav a[aria-label]')).toHaveLength(expected)
    expect(wrapper.find('a[href="/background"]').exists()).toBe(false)
    expect(wrapper.find('a[href="/jobs"]').exists()).toBe(true)
    expect(wrapper.get('a[href="/knowledge"]').attributes('title')).toBe(translate('nav.knowledge'))
    expect(wrapper.get('a[href="/knowledge"]').find('svg').exists()).toBe(true)
    expect(wrapper.get('a[href="/knowledge"]').text()).toBe('')
    expect(wrapper.get('button[aria-label="Expand sidebar"]').attributes('aria-expanded')).toBe('false')
    await wrapper.get('button[aria-label="Expand sidebar"]').trigger('click')
    expect(wrapper.emitted('toggle')).toHaveLength(1)

    await wrapper.setProps({ collapsed: false })
    expect(wrapper.get('a[href="/knowledge"]').text()).toBe(translate('nav.knowledge'))
    expect(wrapper.get('button[aria-label="Collapse sidebar"]').attributes('aria-expanded')).toBe('true')
  } finally { wrapper.unmount() }
})

it('keeps the mobile drawer fully expanded without a desktop collapse control', () => {
  const wrapper = mountSidebar(false, false)
  try {
    expect(wrapper.get('a[href="/knowledge"]').text()).toBe(translate('nav.knowledge'))
    expect(wrapper.find('button[aria-label="Collapse sidebar"]').exists()).toBe(false)
    expect(wrapper.find('a[href="/background"]').exists()).toBe(false)
    expect(wrapper.find('button[aria-label="Expand sidebar"]').exists()).toBe(false)
  } finally { wrapper.unmount() }
})
