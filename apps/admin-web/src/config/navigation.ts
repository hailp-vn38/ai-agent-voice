import { Bot, Boxes, Cpu, LayoutDashboard, LayoutTemplate, Mic, Server, Workflow } from '@lucide/vue'

import type { MessageKey } from '@/i18n/messages'

export const navigationGroups = [
  {
    label: 'nav.group.workspace',
    items: [
      { label: 'nav.overview', to: '/overview', icon: LayoutDashboard },
      { label: 'nav.agents', to: '/agents', icon: Bot },
      { label: 'nav.templates', to: '/templates', icon: LayoutTemplate },
    ],
  },
  {
    label: 'nav.group.voice',
    items: [
      { label: 'nav.speakers', to: '/speakers', icon: Mic },
      { label: 'nav.devices', to: '/devices', icon: Cpu },
    ],
  },
  {
    label: 'nav.group.infrastructure',
    items: [
      { label: 'nav.providers', to: '/providers', icon: Boxes },
      { label: 'nav.mcp', to: '/mcp', icon: Workflow },
      { label: 'nav.system', to: '/system', icon: Server },
    ],
  },
] as const satisfies readonly {
  label: MessageKey
  items: readonly { label: MessageKey; to: string; icon: unknown }[]
}[]

/** Backward-compatible flat list for existing consumers. */
export const navigation = navigationGroups.map((group) => group.items).flat()
