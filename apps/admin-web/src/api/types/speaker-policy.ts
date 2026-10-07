import type { Page } from './common'

/** Requested policy mode.  `required` is rejected fail-closed until tickets 14/15. */
export type AgentSpeakerPolicyMode = 'off' | 'observe' | 'required'

export interface AgentSpeakerPolicy {
  agent_key: string
  mode: AgentSpeakerPolicyMode
  /** Policy revision, independent of the Agent revision that grants CAS against. */
  revision: number
  verification_scope: string
  speaker_change: string
  text_turns: string
  /** False while the qualification gate is missing; the UI must not enable `required`. */
  required_available: boolean
  required_blockers: string[]
}

/** One Agent Speaker binding row: the Speaker plus its explicit Template grants. */
export interface AgentSpeakerBinding {
  speaker_key: string
  template_keys: string[]
  enabled: boolean
  /** Any disabled resource (Speaker, Template, or assignment) ⇒ not usable. */
  usable: boolean
}

export interface AgentSpeakerBindingPage extends Page<AgentSpeakerBinding> {
  agent_revision: number
}

/** Replace-all grant response from `PUT /agents/{key}/speakers/{speaker_key}`. */
export interface PutAgentSpeakerBindingResult {
  agent_key: string
  speaker_key: string
  template_keys: string[]
  agent_revision: number
  activation: { new_connections: string; existing_connections: string }
}
