import { afterEach, describe, expect, it, vi } from 'vitest'

import { providerAdaptersApi } from './provider-adapters'

describe('provider adapter API', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('does not request an adapter endpoint before an adapter is selected', async () => {
    const fetch = vi.fn()
    vi.stubGlobal('fetch', fetch)

    expect(() => providerAdaptersApi.get(undefined as unknown as string)).toThrow('Adapter phải được chọn')

    expect(fetch).not.toHaveBeenCalled()
  })

  it('does not discover capabilities before an adapter is selected', async () => {
    const fetch = vi.fn()
    vi.stubGlobal('fetch', fetch)

    expect(() => providerAdaptersApi.discoverCapabilities('', { selection: {} })).toThrow('Adapter phải được chọn')

    expect(fetch).not.toHaveBeenCalled()
  })

  it('unwraps adapter lists returned by the server', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({
      items: [{ adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' }],
    }), { status: 200, headers: { 'Content-Type': 'application/json' } })))

    await expect(providerAdaptersApi.list('tts')).resolves.toEqual([
      { adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' },
    ])
  })
})
