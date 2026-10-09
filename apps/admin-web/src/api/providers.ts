import { assertCredentialTransport, requestAudio, jsonRequest, request, requestBlob, requestJson, withQuery } from './client'
import type {
  AdminProvider,
  ProviderTestDraft,
  ProviderTextTestResult,
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
  testDraftLlm(provider: ProviderTestDraft, input: { text: string }, signal?: AbortSignal) {
    if (provider.api_key) assertCredentialTransport()
    return requestJson<ProviderTextTestResult>('/api/admin/provider-tests/llm', jsonRequest('POST', { provider, input }), { signal })
  },
  testDraftTts(provider: ProviderTestDraft, input: TtsDiagnosticInput, signal?: AbortSignal) {
    if (provider.api_key) assertCredentialTransport()
    return requestAudio('/api/admin/provider-tests/tts', jsonRequest('POST', { provider, input }), { signal })
  },
  testDraftAsr(provider: ProviderTestDraft, audio: Blob, signal?: AbortSignal) {
    if (provider.api_key) assertCredentialTransport()
    const body = new FormData()
    body.append('provider', JSON.stringify(provider))
    body.append('audio', audio, 'clip.wav')
    return requestJson<ProviderTextTestResult>('/api/admin/provider-tests/asr', { method: 'POST', body }, { signal })
  },
  testTtsAudio(key: string, input: TtsDiagnosticInput, signal?: AbortSignal) {
    return requestAudio(`${providerPath(key)}/test/tts`, jsonRequest('POST', input), { signal })
  },
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
  testAsr(key: string, input: AsrDiagnosticInput, signal?: AbortSignal) {
    return requestJson<ProviderTextTestResult>(`${providerPath(key)}/test/asr`, {
      method: 'POST',
      headers: { 'Content-Type': 'audio/wav' },
      body: input.audio,
    }, { signal })
  },
  testLlm(key: string, input: LlmDiagnosticInput, signal?: AbortSignal) {
    return requestJson<ProviderTextTestResult>(`${providerPath(key)}/test/llm`, jsonRequest('POST', input), { signal })
  },
  testTts(key: string, input: TtsDiagnosticInput, signal?: AbortSignal) {
    return requestBlob(`${providerPath(key)}/test/tts`, jsonRequest('POST', input), { signal })
  },
}
