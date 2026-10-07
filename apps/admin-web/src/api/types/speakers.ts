import type { Page, PageQuery } from './common'

export type SpeakerEnrollmentStatus = 'enrolled' | 'unenrolled' | 'draft'

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
  enrolled_with_provider_key: string
  enrolled_with_provider_revision: number
  browser_validation_status: string
  enrolled_at: number
}

export interface SpeakerEnrollmentDraftSummary {
  id: string
  status: 'collecting'
  revision: number
  expires_at: number
}

export interface Speaker {
  key: string
  name: string
  description: string | null
  enabled: boolean
  revision: number
  voiceprints: SpeakerVoiceprint[]
  enrollment_drafts: SpeakerEnrollmentDraftSummary[]
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
  provider_key?: string
}

export interface CreateSpeakerInput {
  key: string
  name: string
  description?: string
}

export interface UpdateSpeakerInput {
  name?: string
  description?: string | null
  enabled?: boolean
}

export interface EnrollmentDraft {
  id: string
  speaker_key: string
  provider_key: string
  desired_provider_revision: number
  loaded_provider_revision: number | null
  runtime_id: string
  embedding_space_id: string
  dimension: number | null
  revision: number
  status: 'collecting'
  base_speaker_revision: number
  base_voiceprint_revision: number | null
  expires_at: number
  samples: unknown[]
}

export interface CreateEnrollmentDraftInput {
  provider_key: string
  expected_provider_revision: number
}

export interface SpeakerRecognitionEnrollmentConfig {
  content_type: string
  sample_rate: number
  channels: number
  bits_per_sample: number
  min_samples: number
  max_samples: number
  min_clip_ms: number
  max_clip_ms: number
  min_speech_ms: number
  ttl_ms: number
  max_body_bytes: number
}

export interface SpeakerRecognitionProvider {
  provider_key: string
  provider_revision: number
  adapter: string
  state: string
}

export interface SpeakerRecognitionSummary {
  available: boolean
  runtime_mode: string
  providers: SpeakerRecognitionProvider[]
  enrollment: SpeakerRecognitionEnrollmentConfig
  limits: {
    max_speakers: number
    max_voiceprint_spaces_per_speaker: number
    max_candidates_per_agent: number
  }
  catalog_revision: number
}

/** One Agent/Template pair that grants a Speaker (`GET /speakers/{key}/bindings`). */
export interface SpeakerBinding {
  agent_key: string
  template_key: string
}

export interface SpeakerBindingPage extends Page<SpeakerBinding> {}
