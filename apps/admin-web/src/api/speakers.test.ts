import { afterEach, describe, expect, it, vi } from 'vitest'
import { speakersApi } from './speakers'

function installFetch() {
  const fetch = vi.fn(() => Promise.resolve(
    new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } }),
  ))
  vi.stubGlobal('fetch', fetch)
  return fetch
}
function call(fetch: ReturnType<typeof vi.fn>, index: number) {
  return fetch.mock.calls[index] as [string, RequestInit]
}

describe('Speakers API: one-sample built-in CAM++', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('gets server-owned Speaker capability and paged profiles', async () => {
    const fetch = installFetch()
    await speakersApi.summary()
    await speakersApi.list({ page: 2, pageSize: 25, sort: '-updated_at' })
    expect(call(fetch, 0)[0]).toBe('/api/admin/speaker-recognition')
    expect(call(fetch, 1)[0]).toContain('page=2')
    expect(call(fetch, 1)[0]).toContain('page_size=25')
    expect(call(fetch, 1)[0]).not.toContain('provider_key')
  })

  it('extracts one WAV without selecting a Provider then saves metadata', async () => {
    const fetch = installFetch()
    const wav = new Blob(['wav'], { type: 'audio/wav' })
    await speakersApi.capture(wav)
    await speakersApi.createFromCapture({ capture_id: 'capture', name: 'Owner' })
    expect(call(fetch, 0)[0]).toBe('/api/admin/speakers/captures')
    expect(call(fetch, 0)[1].method).toBe('POST')
    expect((call(fetch, 0)[1].headers as Headers).get('If-Match')).toBeNull()
    expect((call(fetch, 0)[1].headers as Headers).get('Content-Type')).toBe('audio/wav')
    expect(call(fetch, 0)[1].body).toBe(wav)
    expect(call(fetch, 1)[0]).toBe('/api/admin/speakers/from-capture')
    expect(call(fetch, 1)[1].body).toBe(JSON.stringify({ capture_id: 'capture', name: 'Owner' }))
  })

  it('replaces one sample with Speaker revision CAS', async () => {
    const fetch = installFetch()
    await speakersApi.replaceVoiceprint('a/b', 'capture', 9)
    const [url, init] = call(fetch, 0)
    expect(url).toBe('/api/admin/speakers/a%2Fb/voiceprint')
    expect(init.method).toBe('PUT')
    expect((init.headers as Headers).get('If-Match')).toBe('"9"')
    expect(init.body).toBe(JSON.stringify({ capture_id: 'capture' }))
  })

  it('preserves Speaker profile CRUD and metadata-only update', async () => {
    const fetch = installFetch()
    await speakersApi.get('a/b')
    await speakersApi.create({ key: 'owner', name: 'Owner' })
    await speakersApi.update('a/b', { name: 'Owner' }, 2)
    await speakersApi.remove('a/b', 3)
    expect(call(fetch, 0)[0]).toBe('/api/admin/speakers/a%2Fb')
    expect(call(fetch, 1)[0]).toBe('/api/admin/speakers')
    expect(call(fetch, 2)[1].method).toBe('PATCH')
    expect((call(fetch, 2)[1].headers as Headers).get('If-Match')).toBe('"2"')
    expect((call(fetch, 3)[1].headers as Headers).get('If-Match')).toBe('"3"')
  })

  it('supports deleting a voiceprint without deleting Speaker metadata', async () => {
    const fetch = installFetch()
    await speakersApi.purgeVoiceprint('a/b', 9)
    const [url, init] = call(fetch, 0)
    expect(url).toBe('/api/admin/speakers/a%2Fb/voiceprint/purge')
    expect(init.method).toBe('POST')
    expect((init.headers as Headers).get('If-Match')).toBe('"9"')
  })

  it('lists Agent bindings without Template grants', async () => {
    const fetch = installFetch()
    await speakersApi.bindings('a/b')
    expect(call(fetch, 0)[0]).toBe('/api/admin/speakers/a%2Fb/bindings?page=1&page_size=50')
  })
})
