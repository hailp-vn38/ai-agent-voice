import type { Page, PageQuery } from './common'

export type TemplateProviderType = 'vad' | 'asr' | 'llm' | 'tts'

export interface AdminTemplate {
  key: string
  name: string
  description: string | null
  language: string
  prompt: string
  enabled: boolean
  revision: number
}

export interface TemplateListQuery extends PageQuery {
  enabled?: boolean
  q?: string
  language?: string
  sort?: 'key' | '-key' | 'name' | '-name' | 'language'
}

export interface CreateTemplateInput {
  key: string
  name: string
  description?: string
  language: string
  prompt: string
}

export interface UpdateTemplateInput {
  name?: string
  description?: string
  language?: string
  prompt?: string
  enabled?: boolean
}

export interface TemplateAgent {
  key: string
  name: string
  enabled: boolean
  is_default: boolean
}

export interface TemplateAgentPage extends Page<TemplateAgent> {
  template_key: string
  revision: number
}

export interface TemplateProviderBinding {
  provider_key: string
  enabled: boolean
}

export interface TemplateProviderBindings {
  template_key: string
  revision: number
  bindings: Partial<Record<TemplateProviderType, TemplateProviderBinding>>
}

export interface BindTemplateProviderInput {
  provider_key: string
}

export type TemplatePage = Page<AdminTemplate>
