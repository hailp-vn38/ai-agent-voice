import type { Page } from './common'

/** Speaker only identifies a voice; it never authenticates or authorizes. */
export type AgentSpeakerPolicyMode = 'off' | 'observe'

export interface AgentSpeakerPolicy {
  agent_key: string
  mode: AgentSpeakerPolicyMode
  revision: number
  speaker_change: 'reconnect'
}

export interface AgentSpeakerBinding {
  speaker_key: string
  enabled: boolean
  usable: boolean
}

export interface AgentSpeakerBindingPage extends Page<AgentSpeakerBinding> {
  agent_revision: number
}

export interface PutAgentSpeakerBindingResult {
  agent_key: string
  speaker_key: string
  agent_revision: number
  activation: { new_connections: string; existing_connections: string }
}
