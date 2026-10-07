import { request, requestJson, withQuery } from './client'
import type {
  CreateEnrollmentDraftInput,
  CreateSpeakerInput,
  EnrollmentDraft,
  Speaker,
  SpeakerBindingPage,
  SpeakerListQuery,
  SpeakerPage,
  SpeakerRecognitionSummary,
  UpdateSpeakerInput,
} from './types/speakers'

function speakerPath(key: string) {
  return `/api/admin/speakers/${encodeURIComponent(key)}`
}

function draftPath(speakerKey: string, draftId: string) {
  return `${speakerPath(speakerKey)}/enrollments/${encodeURIComponent(draftId)}`
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
}
