export const providerTypes = ['vad', 'asr', 'llm', 'tts', 'vision'] as const
export type ProviderType = (typeof providerTypes)[number] | 'speaker' // legacy deserialization only

export const llmCapabilityTags = [
  'chat',
  'vision',
  'image',
  'video',
  'audio',
  'reasoning',
  'embedding',
] as const
export type LlmCapabilityTag = (typeof llmCapabilityTags)[number]

export type ProviderStatus = 'ready' | 'disabled' | 'error'
export type DeviceStatus = 'online' | 'offline'

/** A provider instance in the global catalog. Templates bind to these by id. */
export interface ProviderInstance {
  source?: import('@/api/types/providers').AdminProvider['source']
  configJson?: Record<string, unknown>
  id: string
  name: string
  type: ProviderType
  adapter: string
  tags?: string[]
  credentialEnv?: string
  credential?: import('@/api/types/providers').CredentialMetadata | null
  model: string
  description: string
  status: ProviderStatus
  desiredRevision?: number
  runtime?: import('@/api/types/providers').ProviderRuntime
  endpoint?: string
}

export function effectiveLlmTags(
  provider: Pick<ProviderInstance, 'type' | 'tags' | 'configJson'>,
): string[] {
  if (provider.type !== 'llm') return []
  if (Array.isArray(provider.tags) && provider.tags.length > 0) return provider.tags
  const raw = provider.configJson?.tags
  if (Array.isArray(raw)) {
    const valid = raw.filter(
      (item): item is string => typeof item === 'string' && item.trim().length > 0,
    )
    if (valid.length > 0) return valid
  }
  return ['chat']
}

export function providerSupportsLlmTag(
  provider: Pick<ProviderInstance, 'type' | 'tags' | 'configJson'>,
  tag: string,
): boolean {
  return effectiveLlmTags(provider).includes(tag)
}

export function templateCandidateProviders(
  providers: readonly ProviderInstance[],
  type: ProviderType,
): ProviderInstance[] {
  return providers.filter(
    (provider) =>
      provider.type === type && (type !== 'llm' || providerSupportsLlmTag(provider, 'chat')),
  )
}

export function selectedMultiSelectValues(
  config: Record<string, unknown>,
  key: string,
): string[] {
  const value = config[key]
  return Array.isArray(value) ? value.map(String) : []
}

export function toggleMultiSelectConfigValue(
  config: Record<string, unknown>,
  key: string,
  item: string,
): Record<string, unknown> {
  const current = new Set(selectedMultiSelectValues(config, key))
  if (current.has(item)) current.delete(item)
  else current.add(item)
  const next = { ...config }
  if (current.size === 0) delete next[key]
  else next[key] = Array.from(current)
  return next
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
  kind?: 'physical' | 'web_assistant'
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
