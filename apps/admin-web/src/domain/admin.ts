export const providerTypes = ['vad', 'asr', 'llm', 'tts', 'vision'] as const
export type ProviderType = (typeof providerTypes)[number] | 'speaker' // legacy deserialization only

export type ProviderStatus = 'ready' | 'disabled' | 'error'
export type DeviceStatus = 'online' | 'offline'

/** A provider instance in the global catalog. Templates bind to these by id. */
export interface ProviderInstance {
  id: string
  name: string
  type: ProviderType
  adapter: string
  credentialEnv?: string
  model: string
  description: string
  status: ProviderStatus
  desiredRevision?: number
  runtime?: import('@/api/types/providers').ProviderRuntime
  endpoint?: string
}

/**
 * Agent owns identity only, plus which template it defaults to. Language,
 * prompt and provider bindings belong to the global AgentTemplate it links.
 */
export interface Agent {
  id: string
  name: string
  description: string

  /** Always one of the templates linked to this agent. */
  defaultTemplateId: string
  deviceIds: string[]

  createdAt: string
  updatedAt: string
}

/**
 * Global, reusable AI configuration. Not owned by any agent: the same template
 * is shared by every agent that links it.
 */
export interface AgentTemplate {
  id: string

  name: string
  description: string

  language: string
  prompt: string

  providerBindings: Partial<Record<ProviderType, string>>

  createdAt: string
  updatedAt: string
}

/**
 * Agent ↔ Template edge. The single source of truth for the relationship, so
 * `Agent` carries no template list and `AgentTemplate` no agent id.
 */
export interface AgentTemplateLink {
  agentId: string
  templateId: string
}

/**
 * Read-only projection of the templates binding one provider, assembled by a
 * page from the template bindings. Never stored: it is derived on every render.
 */
export interface ProviderUsageEntry {
  templateId: string
  templateName: string
  language: string
  agentNames: string[]
}

export interface Device {
  id: string
  agentId: string
  name: string
  deviceId: string
  description: string
  status: DeviceStatus
  /** `undefined` means the device follows the agent default template. */
  templateId?: string
  lastSeen: string
}

export interface AdminDatabase {
  agents: Agent[]
  templates: AgentTemplate[]
  agentTemplateLinks: AgentTemplateLink[]
  providers: ProviderInstance[]
  devices: Device[]
}

/** Data values, not UI copy: the language a template answers in. */
export const templateLanguageOptions = [
  'Vietnamese',
  'English',
  'Tiếng Việt',
  'Tiếng Anh',
] as const