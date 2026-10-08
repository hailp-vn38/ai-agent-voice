import type { Page, PageQuery } from './common'

export type SpeakerEnrollmentStatus = 'enrolled' | 'unenrolled'

export type SpeakerListSort =
  | 'key'
  | '-key'
  | 'name'
  | '-name'
  | 'updated_at'
  | '-updated_at'
  | 'revision'
  | '-revision'

export interface SpeakerVoiceprint {
  revision: number
  sample_count: number
  embedding_space_id: string
  enrolled_at: number
}

export interface Speaker {
  key: string
  name: string
  description: string | null
  enabled: boolean
  revision: number
  voiceprints: SpeakerVoiceprint[]
  created_at: number
  updated_at: number
}

export interface SpeakerSummary {
  key: string
  name: string
  description: string | null
  enabled: boolean
  revision: number
  updated_at: number
}

export interface SpeakerPage extends Page<SpeakerSummary> {}

export interface SpeakerListQuery extends PageQuery {
  sort?: SpeakerListSort
  enabled?: boolean
  enrollment_status?: SpeakerEnrollmentStatus
}

export interface CreateSpeakerInput {
  key: string
  name: string
  description?: string
}

export interface SpeakerCapture {
  status: 'accepted'
  capture_id: string
  quality: { duration_ms: number; speech_ms: number }
  expires_at: number
}

export interface CreateSpeakerFromCaptureInput {
  capture_id: string
  name: string
  description?: string
}

export interface CreateSpeakerFromCaptureResult {
  speaker: Speaker
}

export interface UpdateSpeakerInput {
  name?: string
  description?: string | null
  enabled?: boolean
}

export interface SpeakerRecognitionEnrollmentConfig {
  content_type: string
  sample_rate: number
  channels: number
  bits_per_sample: number
  min_clip_ms: number
  max_clip_ms: number
  min_speech_ms: number
  max_window_ms: number
  max_body_bytes: number
}

export interface SpeakerRecognitionSummary {
  available: boolean
  embedding_space_id: string | null
  dimension: number | null
  enrollment: SpeakerRecognitionEnrollmentConfig
  limits: {
    max_speakers: number
    max_candidates_per_agent: number
  }
}

/** Agent using the Speaker as an identification candidate. */
export interface SpeakerBinding {
  agent_key: string
}

export interface SpeakerBindingPage extends Page<SpeakerBinding> {}
