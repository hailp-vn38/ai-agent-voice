import { request, requestJson } from './client'
import type { PutAgentMcpBindingInput, AgentMcpBindings } from './types/mcp'
import type { AdminAgent, AgentTemplatePage, CreateAgentInput, UpdateAgentInput } from './types/agents'
import type { Page } from './types/common'
import type {
  AgentSpeakerBindingPage,
  AgentSpeakerPolicy,
  AgentSpeakerPolicyMode,
  PutAgentSpeakerBindingResult,
} from './types/speaker-policy'

function agentPath(key: string) {
  return `/api/admin/agents/${encodeURIComponent(key)}`
}

const agentsPath = '/api/admin/agents'

export const agentsApi = {
  list(signal?: AbortSignal) {
    return requestJson<Page<AdminAgent>>(`${agentsPath}?page=1&page_size=50&enabled=true&sort=name`, {}, { signal })
  },
  get(key: string, signal?: AbortSignal) {
    return requestJson<AdminAgent>(agentPath(key), {}, { signal })
  },
  create(input: CreateAgentInput) {
    return requestJson<AdminAgent>(agentsPath, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    })
  },
  update(key: string, input: UpdateAgentInput, revision: number) {
    return requestJson<AdminAgent>(agentPath(key), {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    }, { revision })
  },
  async remove(key: string, revision: number) {
    await request(agentPath(key), { method: 'DELETE' }, { revision })
  },
  templates(key: string, signal?: AbortSignal) {
    return requestJson<AgentTemplatePage>(`${agentPath(key)}/templates?page=1&page_size=50`, {}, { signal })
  },
  async assignTemplate(agentKey: string, templateKey: string, revision: number) {
    await request(`${agentPath(agentKey)}/templates/${encodeURIComponent(templateKey)}`, { method: 'PUT' }, { revision })
  },
  async unlinkTemplate(agentKey: string, templateKey: string, revision: number) {
    await request(`${agentPath(agentKey)}/templates/${encodeURIComponent(templateKey)}`, { method: 'DELETE' }, { revision })
  },
  async setDefaultTemplate(agentKey: string, templateKey: string, revision: number) {
    await request(`${agentPath(agentKey)}/default-template/${encodeURIComponent(templateKey)}`, { method: 'PUT' }, { revision })
  },
  mcpBindings(key: string, signal?: AbortSignal) {
    return requestJson<AgentMcpBindings>(`${agentPath(key)}/mcp-bindings`, {}, { signal })
  },
  async bindMcpServer(agentKey: string, mcpServerKey: string, input: PutAgentMcpBindingInput, revision: number) {
    await request(
      `${agentPath(agentKey)}/mcp-bindings/${encodeURIComponent(mcpServerKey)}`,
      { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input) },
      { revision },
    )
  },
  async unlinkMcpServer(agentKey: string, mcpServerKey: string, revision: number) {
    await request(`${agentPath(agentKey)}/mcp-bindings/${encodeURIComponent(mcpServerKey)}`, { method: 'DELETE' }, { revision })
  },
  speakerPolicy(key: string, signal?: AbortSignal) {
    return requestJson<AgentSpeakerPolicy>(`${agentPath(key)}/speaker-policy`, {}, { signal })
  },
  setSpeakerPolicy(key: string, mode: AgentSpeakerPolicyMode, revision: number) {
    return requestJson<AgentSpeakerPolicy>(`${agentPath(key)}/speaker-policy`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ mode }),
    }, { revision })
  },
  agentSpeakers(key: string, signal?: AbortSignal) {
    return requestJson<AgentSpeakerBindingPage>(`${agentPath(key)}/speakers?page=1&page_size=50`, {}, { signal })
  },
  setAgentSpeaker(agentKey: string, speakerKey: string, revision: number) {
    return requestJson<PutAgentSpeakerBindingResult>(
      `${agentPath(agentKey)}/speakers/${encodeURIComponent(speakerKey)}`,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      },
      { revision },
    )
  },
  async unlinkAgentSpeaker(agentKey: string, speakerKey: string, revision: number) {
    await request(`${agentPath(agentKey)}/speakers/${encodeURIComponent(speakerKey)}`, { method: 'DELETE' }, { revision })
  },
}
