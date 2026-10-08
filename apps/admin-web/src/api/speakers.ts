import { request, requestJson, withQuery } from './client'
import type {
  CreateSpeakerFromCaptureInput,
  CreateSpeakerFromCaptureResult,
  CreateSpeakerInput,
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
  bindings(key: string, signal?: AbortSignal) {
    return requestJson<SpeakerBindingPage>(`${speakerPath(key)}/bindings?page=1&page_size=50`, {}, { signal })
  },
}
