import type { InjectionKey } from 'vue'

export interface ActionMenuContext {
  close: (options?: { restore?: boolean }) => void
}

export const ACTION_MENU_INJECTION_KEY: InjectionKey<ActionMenuContext> = Symbol('action-menu')
