import { jsonRequest, request, requestBlob, requestJson, withQuery } from './client'
import type {
  AdminProvider,
  AsrDiagnosticInput,
  CreateProviderInput,
  LlmDiagnosticInput,
  ProviderListQuery,
  ProviderPage,
  ProviderPrepareResult,
  ProviderTemplatePage,
  TtsDiagnosticInput,
  UpdateProviderInput,
  VadDiagnosticResult,
} from './types/providers'

const providersPath = '/api/admin/providers'

function providerPath(key: string) {
  return `${providersPath}/${encodeURIComponent(key)}`
}

export const providersApi = {
  list(query: ProviderListQuery = {}, signal?: AbortSignal) {
    return requestJson<ProviderPage>(withQuery(providersPath, {
      page: query.page ?? 1,
      page_size: query.pageSize ?? 50,
      enabled: query.enabled,
      q: query.q,
      type: query.type,
      sort: query.sort,
    }), {}, { signal })
  },
  get(key: string, signal?: AbortSignal) {
    return requestJson<AdminProvider>(providerPath(key), {}, { signal })
  },
  create(input: CreateProviderInput) {
    return requestJson<AdminProvider>(providersPath, jsonRequest('POST', input))
  },
  update(key: string, input: UpdateProviderInput, revision: number) {
    return requestJson<AdminProvider>(providerPath(key), jsonRequest('PATCH', input), { revision })
  },
  async remove(key: string, revision: number) {
    await request(providerPath(key), { method: 'DELETE' }, { revision })
  },
  templates(key: string, page = 1, pageSize = 50, signal?: AbortSignal) {
    return requestJson<ProviderTemplatePage>(withQuery(`${providerPath(key)}/templates`, { page, page_size: pageSize }), {}, { signal })
  },
  prepare(key: string, signal?: AbortSignal) {
    return requestJson<ProviderPrepareResult>(`${providerPath(key)}/prepare`, jsonRequest('POST', {}), { signal })
  },
  capabilities(key: string, signal?: AbortSignal) {
    return requestJson<unknown>(`${providerPath(key)}/capabilities`, {}, { signal })
  },
  testVad(key: string, signal?: AbortSignal) {
    return requestJson<VadDiagnosticResult>(`${providerPath(key)}/test/vad`, { method: 'POST' }, { signal })
  },
  testSpeaker(key: string, audio: Blob, revision: number, signal?: AbortSignal) {
    return requestJson<unknown>(`${providerPath(key)}/test/speaker`, { method: 'POST', body: audio, headers: { 'Content-Type': 'audio/wav', 'If-Match': `"${revision}"` } }, { signal })
  },
  testAsr(key: string, input: AsrDiagnosticInput, signal?: AbortSignal) {
    return requestJson<unknown>(`${providerPath(key)}/test/asr`, {
      method: 'POST',
      headers: { 'Content-Type': 'audio/wav' },
      body: input.audio,
    }, { signal })
  },
  testLlm(key: string, input: LlmDiagnosticInput, signal?: AbortSignal) {
    return requestJson<unknown>(`${providerPath(key)}/test/llm`, jsonRequest('POST', input), { signal })
  },
  testTts(key: string, input: TtsDiagnosticInput, signal?: AbortSignal) {
    return requestBlob(`${providerPath(key)}/test/tts`, jsonRequest('POST', input), { signal })
  },
}
