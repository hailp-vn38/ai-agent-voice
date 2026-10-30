import type {
  BackgroundAgentConfig,
  BackgroundAgentConfigUpdate,
  BackgroundProviderOption,
} from '@/api/types/agent-jobs'

export function providerSupportsAnyTag(
  option: BackgroundProviderOption,
  requiredTags: string[],
): boolean {
  const tags = (option.tags && option.tags.length > 0 ? option.tags : ['chat']).map(t =>
    t.trim().toLowerCase(),
  )
  return requiredTags.some(rawTag => {
    const tag = rawTag.trim().toLowerCase()
    if (tags.includes(tag)) return true
    if ((tag === 'tts' || tag === 'audio') && (tags.includes('tts') || tags.includes('audio'))) {
      return true
    }
    if ((tag === 'asr' || tag === 'audio') && (tags.includes('asr') || tags.includes('audio'))) {
      return true
    }
    return false
  })
}

export function chatProviderOptions(
  providers: BackgroundProviderOption[],
): BackgroundProviderOption[] {
  return providers.filter(option => providerSupportsAnyTag(option, ['chat', 'reasoning']))
}

const BUILTIN_CAPABILITY_FALLBACKS: Record<
  string,
  { allowedTags: string[]; toolTags: Array<[string, string]> }
> = {
  media_creator: {
    allowedTags: ['chat', 'reasoning', 'image', 'video'],
    toolTags: [
      ['image_create', 'image'],
      ['video_create', 'video'],
    ],
  },
  media_processor: {
    allowedTags: ['chat', 'reasoning', 'tts', 'asr', 'audio'],
    toolTags: [
      ['tts_synthesize', 'tts'],
      ['asr_transcribe', 'asr'],
    ],
  },
}

export function allowedCapabilityTagsForAgent(agent: BackgroundAgentConfig | null): string[] {
  if (!agent) return ['chat', 'reasoning']
  if (agent.allowed_capability_tags && agent.allowed_capability_tags.length > 0) {
    return agent.allowed_capability_tags
  }
  return BUILTIN_CAPABILITY_FALLBACKS[agent.id]?.allowedTags ?? ['chat', 'reasoning']
}

export function providerOptionsForAgent(
  providers: BackgroundProviderOption[],
  agent: BackgroundAgentConfig | null,
): BackgroundProviderOption[] {
  if (!agent) return chatProviderOptions(providers)
  const allowedTags = allowedCapabilityTagsForAgent(agent)
  return providers.filter(option => providerSupportsAnyTag(option, allowedTags))
}

export function toolCapabilityEntries(
  agent: BackgroundAgentConfig | null,
): Array<[string, string]> {
  if (!agent) return []
  if (agent.tool_capability_tags && Object.keys(agent.tool_capability_tags).length > 0) {
    return Object.entries(agent.tool_capability_tags)
  }
  return BUILTIN_CAPABILITY_FALLBACKS[agent.id]?.toolTags ?? []
}

export function providerOptionsForTag(
  providers: BackgroundProviderOption[],
  tag: string,
): BackgroundProviderOption[] {
  return providers.filter(option => providerSupportsAnyTag(option, [tag]))
}

export function formatProviderOptionLabel(option: BackgroundProviderOption): string {
  const modelPart = option.model && option.model !== option.name ? ` (${option.model})` : ''
  const tagsPart = option.tags && option.tags.length > 0 ? ` [${option.tags.join(', ')}]` : ''
  return `${option.name}${modelPart}${tagsPart} · ${option.source}`
}

export function findProvider(
  providers: BackgroundProviderOption[],
  refId: string | null | undefined,
): BackgroundProviderOption | undefined {
  if (!refId) return undefined
  return providers.find(p => p.ref === refId)
}

export function domainToolsForAgent(agent: BackgroundAgentConfig): string[] {
  return agent.available_tools.filter(tool => tool !== 'skill')
}

export function defaultDomainToolsForAgent(agent: BackgroundAgentConfig): string[] {
  return (agent.default_allowed_tools ?? agent.available_tools).filter(tool => tool !== 'skill')
}

export function normalizeTools(tools: string[], skills: string[], isPrimary: boolean): string[] {
  if (isPrimary) return ['subagent']
  const set = new Set(tools.filter(t => t !== 'skill'))
  const out = Array.from(set)
  if (skills.length > 0) out.unshift('skill')
  return out
}

export function normalizeToolProviders(
  map: Record<string, string> | undefined,
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [tool, pref] of Object.entries(map ?? {})) {
    const trimmed = (pref ?? '').trim()
    if (trimmed) out[tool] = trimmed
  }
  return out
}

export function sameStringRecord(
  a: Record<string, string> | undefined,
  b: Record<string, string> | undefined,
): boolean {
  const left = normalizeToolProviders(a)
  const right = normalizeToolProviders(b)
  const leftKeys = Object.keys(left).sort()
  const rightKeys = Object.keys(right).sort()
  return (
    leftKeys.length === rightKeys.length &&
    leftKeys.every((key, idx) => key === rightKeys[idx] && left[key] === right[key])
  )
}

export function sameStringArray(a: string[] | undefined, b: string[] | undefined): boolean {
  const left = [...(a ?? [])].sort()
  const right = [...(b ?? [])].sort()
  return left.length === right.length && left.every((val, idx) => val === right[idx])
}

export function toAgentUpdate(agent: BackgroundAgentConfig): BackgroundAgentConfigUpdate {
  const toolProviders = normalizeToolProviders(agent.tool_providers)
  return {
    enabled: agent.enabled,
    provider_ref: agent.provider_ref ?? null,
    ...(Object.keys(toolProviders).length > 0 ? { tool_providers: toolProviders } : {}),
    steps: agent.steps,
    allowed_tools: [...agent.allowed_tools],
    allowed_skills: [...agent.allowed_skills],
    description: agent.description,
    system_prompt: agent.system_prompt,
  }
}

export function isAgentDirty(
  agent: BackgroundAgentConfig,
  draft: BackgroundAgentConfigUpdate | undefined,
): boolean {
  if (!draft) return false
  return (
    draft.enabled !== agent.enabled ||
    (draft.provider_ref ?? null) !== (agent.provider_ref ?? null) ||
    !sameStringRecord(draft.tool_providers, agent.tool_providers) ||
    (draft.steps ?? agent.steps) !== agent.steps ||
    !sameStringArray(draft.allowed_tools, agent.allowed_tools) ||
    !sameStringArray(draft.allowed_skills, agent.allowed_skills) ||
    (draft.description ?? agent.description).trim() !== agent.description.trim() ||
    (draft.system_prompt ?? agent.system_prompt).trim() !== agent.system_prompt.trim()
  )
}

export const CUSTOM_DAILY_DOMAIN_TOOLS: string[] = [
  'anydoc_inspect_pdf',
  'yt_list_playlist',
  'yt_fetch_transcript',
  'fetch_html',
  'html_extract_metadata',
  'html_extract_article',
  'html_extract_feed',
  'html_extract_links',
  'html_select_elements',
  'add_to_sources',
  'wiki_tree',
  'wiki_query',
  'wiki_get',
  'wiki_multi_get',
  'wiki_metadata',
  'wiki_status',
  'wiki_graph_neighbors',
  'markdown_create',
  'markdown_read',
  'artifact_create',
  'reminder_create',
  'reminder_list',
  'reminder_complete',
  'reminder_delete',
]

export const CUSTOM_WIKI_DOMAIN_TOOLS: string[] = [
  'wiki_tree',
  'wiki_get',
  'wiki_multi_get',
  'wiki_query',
  'wiki_metadata',
  'wiki_status',
  'wiki_graph_neighbors',
  'wiki_ingest_source',
  'wiki_file_synthesis',
  'wiki_write_file',
  'wiki_patch_file',
  'wiki_delete_file',
  'wiki_lint',
  'wiki_schema',
]

export function domainToolsForCustomFlow(flowId: 'daily' | 'wiki'): string[] {
  return flowId === 'wiki' ? CUSTOM_WIKI_DOMAIN_TOOLS : CUSTOM_DAILY_DOMAIN_TOOLS
}

export function isAgentCustomizedFromDefault(
  agent: BackgroundAgentConfig,
  current?: BackgroundAgentConfigUpdate,
): boolean {
  if (agent.is_builtin === false) return false
  const enabled = current ? current.enabled : agent.enabled
  const providerRef = current ? (current.provider_ref ?? null) : (agent.provider_ref ?? null)
  const toolProviders = normalizeToolProviders(current ? current.tool_providers : agent.tool_providers)
  const steps = current?.steps ?? agent.steps
  const skills = current?.allowed_skills ?? agent.allowed_skills
  const tools = (current?.allowed_tools ?? agent.allowed_tools).filter(t => t !== 'skill')
  const defaultTools = defaultDomainToolsForAgent(agent)
  const defaultSkills = agent.default_allowed_skills ?? []
  const description = (current?.description ?? agent.description).trim()
  const systemPrompt = (current?.system_prompt ?? agent.system_prompt).trim()

  return (
    !enabled ||
    Boolean(providerRef) ||
    Object.keys(toolProviders).length > 0 ||
    steps !== agent.default_steps ||
    (agent.mode !== 'primary' && !sameStringArray(tools, defaultTools)) ||
    (agent.mode !== 'primary' && !sameStringArray(skills, defaultSkills)) ||
    description !== agent.default_description.trim() ||
    systemPrompt !== agent.default_system_prompt.trim()
  )
}
