import { mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import AdminShell from './AdminShell.vue'

vi.mock('vue-router', () => ({
  useRoute: () => ({ path: '/overview' }),
}))

const global = {
  stubs: {
    AppSidebar: {
      props: ['collapsed'],
      emits: ['toggle'],
      template: '<button data-test="sidebar-toggle" @click="$emit(\'toggle\')">{{ collapsed ? "collapsed" : "expanded" }}</button>',
    },
    SidebarContent: true,
    AssistantWidget: true,
    ThemeToggle: true,
    Button: true,
  },
}

beforeEach(() => window.localStorage.clear())

it('remembers the desktop sidebar collapse state after remounting the admin shell', async () => {
  const first = mount(AdminShell, { global })
  expect(first.get('[data-test="sidebar-toggle"]').text()).toBe('expanded')
  await first.get('[data-test="sidebar-toggle"]').trigger('click')
  expect(first.get('[data-test="sidebar-toggle"]').text()).toBe('collapsed')
  expect(window.localStorage.getItem('voice-agent-admin-sidebar-collapsed')).toBe('true')
  first.unmount()

  const second = mount(AdminShell, { global })
  try {
    expect(second.get('[data-test="sidebar-toggle"]').text()).toBe('collapsed')
    await second.get('[data-test="sidebar-toggle"]').trigger('click')
    expect(window.localStorage.getItem('voice-agent-admin-sidebar-collapsed')).toBe('false')
  } finally { second.unmount() }
})

it('ignores an unknown saved sidebar value', () => {
  window.localStorage.setItem('voice-agent-admin-sidebar-collapsed', 'invalid')
  const wrapper = mount(AdminShell, { global })
  try {
    expect(wrapper.get('[data-test="sidebar-toggle"]').text()).toBe('expanded')
  } finally { wrapper.unmount() }
})
