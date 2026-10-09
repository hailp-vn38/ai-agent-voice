import { afterEach, expect, it, vi } from 'vitest'
import { providersApi } from './providers'
import { mcpApi } from './mcp'

afterEach(() => vi.unstubAllGlobals())
it('posts an unsaved LLM configuration with cancellation and no CRUD', async () => {
  const fetch = vi.fn().mockResolvedValue(new Response('{"test_source":"draft","result":{"text":"hello"}}'))
  vi.stubGlobal('fetch', fetch)
  const signal = new AbortController().signal
  const provider = { type: 'llm' as const, adapter: 'openai', config_json: { model: 'demo' } }
  await providersApi.testDraftLlm(provider, { text: 'hi' }, signal)
  expect(fetch).toHaveBeenCalledTimes(1)
  expect(fetch.mock.calls[0][0]).toBe('/api/admin/provider-tests/llm')
  expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ provider, input: { text: 'hi' } })
  expect(fetch.mock.calls[0][1].signal).toBe(signal)
})
it('uses separate connection and discovery routes for saved and draft MCP', async () => {
  const fetch = vi.fn().mockImplementation(() => Promise.resolve(new Response('{}')))
  vi.stubGlobal('fetch', fetch)
  const server = { key: 'weather', url: 'https://example.test/mcp', auth: { type: 'none' as const } }
  await mcpApi.testDraftConnection(server)
  await mcpApi.discoverDraftTools(server)
  await mcpApi.testSavedConnection('weather')
  await mcpApi.discoverSavedTools('weather')
  expect(fetch.mock.calls.map(([url]) => url)).toEqual(['/api/admin/mcp-tests/connection', '/api/admin/mcp-tests/discover', '/api/admin/mcp-servers/weather/test/connection', '/api/admin/mcp-servers/weather/test/discover'])
})
it('blocks nested draft credentials over LAN HTTP before fetch', () => {
  vi.stubGlobal('location', { href: 'http://192.168.1.2/' })
  const fetch = vi.fn()
  vi.stubGlobal('fetch', fetch)
  expect(() => providersApi.testDraftLlm({ type: 'llm', adapter: 'openai', config_json: {}, api_key: 'secret' }, { text: 'hi' })).toThrow('HTTPS')
  expect(() => mcpApi.testDraftConnection({ key: 'test', url: 'http://localhost/mcp', auth: { type: 'bearer' }, api_key: 'secret' })).toThrow('HTTPS')
  expect(fetch).not.toHaveBeenCalled()
})

it.each([401, 400, 409, 413, 429, 502, 503, 504])('keeps bounded diagnostic HTTP %i errors', async (status) => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{"error":{"code":"diagnostic_failed","request_id":"req"}}', { status })))
  await expect(mcpApi.testSavedConnection('weather')).rejects.toMatchObject({ status, code: 'diagnostic_failed', requestId: 'req' })
})
it('sends ASR as two multipart parts without a fabricated content-type boundary', async () => {
  const fetch = vi.fn().mockResolvedValue(new Response('{}'))
  vi.stubGlobal('fetch', fetch)
  await providersApi.testDraftAsr({ type: 'asr', adapter: 'zipformer_sherpa', config_json: {} }, new Blob(['RIFF'], { type: 'audio/wav' }))
  const init = fetch.mock.calls[0][1]
  expect(init.body).toBeInstanceOf(FormData)
  expect(init.body.get('audio')).toBeInstanceOf(Blob)
  expect(JSON.parse(init.body.get('provider'))).toEqual({ type: 'asr', adapter: 'zipformer_sherpa', config_json: {} })
  expect(init.headers.get('Content-Type')).toBeNull()
})
it('returns TTS response timing with an actual WAV blob and rejects mislabeled audio', async () => {
  const fetch = vi.fn().mockResolvedValueOnce(new Response('RIFF', { headers: { 'Content-Type': 'audio/wav', 'X-Provider-Test-Elapsed-Ms': '42' } })).mockResolvedValueOnce(new Response('{}', { headers: { 'Content-Type': 'application/json' } }))
  vi.stubGlobal('fetch', fetch)
  await expect(providersApi.testDraftTts({ type: 'tts', adapter: 'zerotts_onnx', config_json: {} }, { text: 'hello' })).resolves.toMatchObject({ elapsedMs: 42, audio: { type: 'audio/wav' } })
  await expect(providersApi.testTtsAudio('saved', { text: 'hello' })).rejects.toThrow('invalid_audio_response')
})
