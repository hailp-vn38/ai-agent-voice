import { inject, type InjectionKey, type Ref } from 'vue'
import { knowledgeMessages } from './i18n'

export type KnowledgeTranslator = (key: string, params?: Record<string, string | number>) => string

export const KNOWLEDGE_TRANSLATOR_KEY: InjectionKey<KnowledgeTranslator> = Symbol('KnowledgeTranslator')
export const KNOWLEDGE_DARK_MODE_KEY: InjectionKey<Readonly<Ref<boolean>>> = Symbol('KnowledgeDarkMode')
export const KNOWLEDGE_CHART_FETCHER_KEY: InjectionKey<(fileName: string) => Promise<unknown>> = Symbol('KnowledgeChartFetcher')
export const KNOWLEDGE_NAVIGATION_KEY: InjectionKey<{
  query: Readonly<Ref<Record<string, unknown>>>
  replace: (query: Record<string, unknown>) => void
}> = Symbol('KnowledgeNavigation')



export function useKnowledgeI18n() {
  const translate = inject(KNOWLEDGE_TRANSLATOR_KEY, (key: string, params?: Record<string, string | number>) => {
    const locale = typeof document !== 'undefined' && document.documentElement.lang === 'vi' ? 'vi' : 'en'
    let value: string = knowledgeMessages[locale][key as keyof typeof knowledgeMessages.en] ?? key
    for (const [name, replacement] of Object.entries(params ?? {})) value = value.replaceAll(`{${name}}`, String(replacement))
    return value
  })
  return { t: translate }
}
export const useI18n = useKnowledgeI18n

export function useKnowledgeTheme() {
  const fallback = { value: typeof document !== 'undefined' && document.documentElement.classList.contains('dark') } as Readonly<Ref<boolean>>
  return { dark: inject(KNOWLEDGE_DARK_MODE_KEY, fallback) }
}
export const useTheme = useKnowledgeTheme

export function useKnowledgeChartFetcher() {
  return inject(KNOWLEDGE_CHART_FETCHER_KEY, undefined)
}

export function useKnowledgeNavigation() {
  const navigation = inject(KNOWLEDGE_NAVIGATION_KEY, {
    query: { value: {} } as Readonly<Ref<Record<string, unknown>>>,
    replace: () => undefined,
  })
  const route = { get query() { return navigation.query.value } }
  const router = { replace: ({ query }: { query: Record<string, unknown> }) => navigation.replace(query) }
  return { route, router }
}
