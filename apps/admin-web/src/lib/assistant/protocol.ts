export const clientHello = {
  type: 'hello', version: 1, transport: 'websocket',
  audio_params: { format: 'opus', sample_rate: 16000, channels: 1, frame_duration: 60 },
  features: { pipeline_status: false, speaker_status: false, aec: false, mcp: false, turn_status: true, tool_status: true },
}
export type TurnState = 'listening_ready' | 'rejected' | 'completed' | 'failed' | 'cancelled'
export type ToolStatusEvent = {
  type: 'tool'
  session_id: string
  turn_id: number
  name: string
  state: 'started' | 'completed' | 'failed'
}
export type ServerEvent =
  | { type: 'hello'; session_id: string }
  | { type: 'stt' | 'llm'; session_id: string; text: string }
  | { type: 'tts'; session_id: string; state: 'start' | 'stop' | 'sentence_start'; text?: string }
  | { type: 'turn_status'; session_id: string; state: TurnState; turn_id?: number; reason?: string }
  | ToolStatusEvent

export function parseServerEvent(raw: string, session?: string): ServerEvent | null {
  if (new TextEncoder().encode(raw).length > 65536) throw new Error('invalid_frame')
  const value: unknown = JSON.parse(raw)
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid_frame')
  const v = value as Record<string, unknown>
  if (typeof v.session_id !== 'string' || !v.session_id || v.session_id.length > 128) throw new Error('invalid_session')
  if (session && v.session_id !== session) throw new Error('invalid_session')
  switch (v.type) {
    case 'hello': {
      const a = v.audio_params as Record<string, unknown> | undefined
      if (session || v.transport !== 'websocket' || !a || a.format !== 'opus' || a.sample_rate !== 24000 || a.channels !== 1 || a.frame_duration !== 60) throw new Error('invalid_hello')
      return { type: 'hello', session_id: v.session_id }
    }
    case 'stt': case 'llm':
      if (typeof v.text !== 'string') throw new Error('invalid_frame')
      return { type: v.type, session_id: v.session_id, text: v.text }
    case 'tts':
      if (!['start', 'stop', 'sentence_start'].includes(String(v.state))) throw new Error('invalid_frame')
      if (v.text !== undefined && typeof v.text !== 'string') throw new Error('invalid_frame')
      return v as ServerEvent
    case 'tool':
      if (!Number.isSafeInteger(v.turn_id) || Number(v.turn_id) < 1
        || typeof v.name !== 'string' || !v.name || v.name.length > 128
        || !/^[A-Za-z0-9_.:-]+$/.test(v.name)
        || !['started', 'completed', 'failed'].includes(String(v.state))) throw new Error('invalid_frame')
      return v as ToolStatusEvent
    case 'turn_status':
      if (!['listening_ready', 'rejected', 'completed', 'failed', 'cancelled'].includes(String(v.state))) throw new Error('invalid_frame')
      if (v.turn_id !== undefined && (!Number.isSafeInteger(v.turn_id) || Number(v.turn_id) < 1)) throw new Error('invalid_frame')
      if (v.reason !== undefined && (typeof v.reason !== 'string' || !/^[a-z_]{1,64}$/.test(v.reason))) throw new Error('invalid_frame')
      return v as ServerEvent
    default: return null
  }
}
