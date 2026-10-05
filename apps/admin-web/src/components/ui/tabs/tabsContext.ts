import type { InjectionKey, Ref } from 'vue'

export interface TabsContext {
  active: Ref<string>
}

export const TABS_INJECTION_KEY: InjectionKey<TabsContext> = Symbol('tabs')