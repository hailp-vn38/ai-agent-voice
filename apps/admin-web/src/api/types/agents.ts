import type { Page } from './common'

export interface AdminAgent {
  key: string
  name: string
  description: string | null
  enabled: boolean
  revision: number
}

export interface AgentTemplateLink {
  key: string
  name: string
  language: string
  enabled: boolean
  is_default: boolean
}

export interface AgentTemplatePage extends Page<AgentTemplateLink> {
  revision: number
}

export interface CreateAgentInput {
  key: string
  name: string
  description?: string
}

export interface UpdateAgentInput {
  name?: string
  description?: string
  enabled?: boolean
}
