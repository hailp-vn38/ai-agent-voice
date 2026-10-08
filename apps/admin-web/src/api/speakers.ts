import { request, requestJson, withQuery } from './client'
import type {
  CreateEnrollmentDraftInput,
  CreateSpeakerFromCaptureInput,
  CreateSpeakerFromCaptureResult,
  CreateSpeakerInput,
  EnrollmentDraft,
  EnrollmentFinalizeResult,
  EnrollmentValidationResult,
  Speaker,
  SpeakerBindingPage,
  SpeakerListQuery,
  SpeakerPage,
  SpeakerRecognitionSummary,
  SpeakerCapture,
  UpdateSpeakerInput,
} from './types/speakers'

function speakerPath(key: string) {
  return `/api/admin/speakers/${encodeURIComponent(key)}`
}

function draftPath(speakerKey: string, draftId: string) {
  return `${speakerPath(speakerKey)}/enrollments/${encodeURIComponent(draftId)}`
}

function samplePath(speakerKey: string, draftId: string, slot: number) {
  return `${draftPath(speakerKey, draftId)}/samples/${slot}`
}

const speakersPath = '/api/admin/speakers'

export const speakersApi = {
  summary(signal?: AbortSignal) {
    return requestJson<SpeakerRecognitionSummary>('/api/admin/speaker-recognition', {}, { signal })
  },
  list(query: SpeakerListQuery = {}, signal?: AbortSignal) {
    return requestJson<SpeakerPage>(
      withQuery(speakersPath, {
        page: query.page ?? 1,
        page_size: query.pageSize ?? 50,
        enabled: query.enabled,
        enrollment_status: query.enrollment_status,
        provider_key: query.provider_key,
        sort: query.sort,
      }),
      {},
      { signal },
    )
  },
  get(key: string, signal?: AbortSignal) {
    return requestJson<Speaker>(speakerPath(key), {}, { signal })
  },
  create(input: CreateSpeakerInput) {
    return requestJson<Speaker>(speakersPath, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    })
  },
  capture(wav: Blob, signal?: AbortSignal) {
    return requestJson<SpeakerCapture>('/api/admin/speakers/captures', {
      method: 'POST', headers: { 'Content-Type': 'audio/wav' }, body: wav, signal,
    })
  },
  replaceVoiceprint(key: string, captureId: string, revision: number, signal?: AbortSignal) {
    return requestJson<{ speaker: Speaker }>(`${speakerPath(key)}/voiceprint`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ capture_id: captureId }), signal,
    }, { revision })
  },
  createFromCapture(input: CreateSpeakerFromCaptureInput, signal?: AbortSignal) {
    return requestJson<CreateSpeakerFromCaptureResult>('/api/admin/speakers/from-capture', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input), signal,
    })
  },
  update(key: string, input: UpdateSpeakerInput, revision: number) {
    return requestJson<Speaker>(speakerPath(key), {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    }, { revision })
  },
  async remove(key: string, revision: number) {
    await request(speakerPath(key), { method: 'DELETE' }, { revision })
  },
  purgeVoiceprint(key: string, revision: number) {
    return requestJson<Speaker>(
      `${speakerPath(key)}/voiceprint/purge`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ confirm: 'PURGE_SPEAKER_VOICEPRINT' }),
      },
      { revision },
    )
  },
  createDraft(speakerKey: string, input: CreateEnrollmentDraftInput, revision: number) {
    return requestJson<EnrollmentDraft>(`${speakerPath(speakerKey)}/enrollments`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    }, { revision })
  },
  getDraft(speakerKey: string, draftId: string, signal?: AbortSignal) {
    return requestJson<EnrollmentDraft>(draftPath(speakerKey, draftId), {}, { signal })
  },
  async cancelDraft(speakerKey: string, draftId: string, revision: number) {
    await request(draftPath(speakerKey, draftId), { method: 'DELETE' }, { revision })
  },
  bindings(key: string, signal?: AbortSignal) {
    return requestJson<SpeakerBindingPage>(`${speakerPath(key)}/bindings?page=1&page_size=50`, {}, { signal })
  },
  uploadSample(
    speakerKey: string,
    draftId: string,
    slot: number,
    wav: Blob,
    revision: number,
    signal?: AbortSignal,
  ) {
    return requestJson<EnrollmentDraft>(samplePath(speakerKey, draftId, slot), {
      method: 'PUT',
      headers: { 'Content-Type': 'audio/wav' },
      body: wav,
      signal,
    }, { revision })
  },
  deleteSample(speakerKey: string, draftId: string, slot: number, revision: number, signal?: AbortSignal) {
    return requestJson<EnrollmentDraft>(samplePath(speakerKey, draftId, slot), {
      method: 'DELETE',
      signal,
    }, { revision })
  },
  validateHoldout(
    speakerKey: string,
    draftId: string,
    wav: Blob,
    revision: number,
    signal?: AbortSignal,
  ) {
    return requestJson<EnrollmentValidationResult>(`${draftPath(speakerKey, draftId)}/validate`, {
      method: 'POST',
      headers: { 'Content-Type': 'audio/wav' },
      body: wav,
      signal,
    }, { revision })
  },
  finalizeDraft(
    speakerKey: string,
    draftId: string,
    expectedSpeakerRevision: number,
    revision: number,
    signal?: AbortSignal,
  ) {
    return requestJson<EnrollmentFinalizeResult>(`${draftPath(speakerKey, draftId)}/finalize`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ expected_speaker_revision: expectedSpeakerRevision }),
      signal,
    }, { revision })
  },
}
