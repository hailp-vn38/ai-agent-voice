import { Bot, Boxes, LayoutTemplate, Mic, Server } from '@lucide/vue'

import type { MessageKey } from '@/i18n/messages'

export const navigation = [
  { label: 'nav.agents', to: '/agents', icon: Bot },
  { label: 'nav.templates', to: '/templates', icon: LayoutTemplate },
  { label: 'nav.providers', to: '/providers', icon: Boxes },
  { label: 'nav.speakers', to: '/speakers', icon: Mic },
  { label: 'nav.system', to: '/system', icon: Server },
] as const satisfies readonly { label: MessageKey; to: string; icon: unknown }[]
