import { afterEach, describe, expect, it, vi } from 'vitest'

import { speakersApi } from './speakers'

function installFetch() {
  const fetch = vi.fn(() =>
    Promise.resolve(
      new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } }),
    ),
  )
  vi.stubGlobal('fetch', fetch)
  return fetch
}

function call(fetch: ReturnType<typeof vi.fn>, index: number) {
  return fetch.mock.calls[index] as [string, RequestInit]
}

describe('speakersApi', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('lists with bounded query parameters and summary endpoint', async () => {
    const fetch = installFetch()
    await speakersApi.list({ page: 2, pageSize: 25, sort: '-updated_at', enrollment_status: 'draft' })
    await speakersApi.summary()

    const [listUrl] = call(fetch, 0)
    expect(listUrl).toContain('/api/admin/speakers?')
    expect(listUrl).toContain('page=2')
    expect(listUrl).toContain('page_size=25')
    expect(listUrl).toContain('sort=-updated_at')
    expect(listUrl).toContain('enrollment_status=draft')
    expect(call(fetch, 1)[0]).toBe('/api/admin/speaker-recognition')
  })

  it('encodes speaker keys and pins mutations to the revision', async () => {
    const fetch = installFetch()
    await speakersApi.get('a/b')
    await speakersApi.create({ key: 'owner', name: 'Owner' })
    await speakersApi.update('a/b', { name: 'Owner', description: null }, 2)
    await speakersApi.remove('a/b', 3)

    expect(call(fetch, 0)[0]).toBe('/api/admin/speakers/a%2Fb')
    expect(call(fetch, 1)[0]).toBe('/api/admin/speakers')
    const createInit = call(fetch, 1)[1]
    expect(createInit.method).toBe('POST')
    expect(createInit.body).toBe(JSON.stringify({ key: 'owner', name: 'Owner' }))
    expect((createInit.headers as Headers).get('If-Match')).toBeNull()

    const patchInit = call(fetch, 2)[1]
    expect(patchInit.method).toBe('PATCH')
    expect((patchInit.headers as Headers).get('If-Match')).toBe('"2"')
    expect(patchInit.body).toBe(JSON.stringify({ name: 'Owner', description: null }))

    const deleteInit = call(fetch, 3)[1]
    expect(deleteInit.method).toBe('DELETE')
    expect((deleteInit.headers as Headers).get('If-Match')).toBe('"3"')
  })

  it('pins draft creation and cancellation to their revisions', async () => {
    const fetch = installFetch()
    await speakersApi.createDraft('owner', { provider_key: 'spk', expected_provider_revision: 4 }, 5)
    await speakersApi.getDraft('owner', 'draft-1')
    await speakersApi.cancelDraft('owner', 'draft-1', 6)

    const [draftUrl, draftInit] = call(fetch, 0)
    expect(draftUrl).toBe('/api/admin/speakers/owner/enrollments')
    expect(draftInit.method).toBe('POST')
    expect((draftInit.headers as Headers).get('If-Match')).toBe('"5"')
    expect(draftInit.body).toBe(
      JSON.stringify({ provider_key: 'spk', expected_provider_revision: 4 }),
    )

    expect(call(fetch, 1)[0]).toBe('/api/admin/speakers/owner/enrollments/draft-1')
    const cancelInit = call(fetch, 2)[1]
    expect(cancelInit.method).toBe('DELETE')
    expect((cancelInit.headers as Headers).get('If-Match')).toBe('"6"')
  })

  it('purges every voiceprint with an explicit confirmation under revision control', async () => {
    const fetch = installFetch()
    await speakersApi.purgeVoiceprint('a/b', 9)

    const [url, init] = call(fetch, 0)
    expect(url).toBe('/api/admin/speakers/a%2Fb/voiceprint/purge')
    expect(init.method).toBe('POST')
    expect((init.headers as Headers).get('If-Match')).toBe('"9"')
    expect(init.body).toBe(JSON.stringify({ confirm: 'PURGE_SPEAKER_VOICEPRINT' }))
  })

  it('lists the agents/templates granting a speaker with bounded paging', async () => {    const fetch = installFetch()
    await speakersApi.bindings('a/b')

    expect(call(fetch, 0)[0]).toBe('/api/admin/speakers/a%2Fb/bindings?page=1&page_size=50')
  })

  it('uploads a WAV sample to a slot and deletes it under revision control', async () => {
    const fetch = installFetch()
    const wav = new Blob([new Uint8Array([1, 2, 3])], { type: 'audio/wav' })
    await speakersApi.uploadSample('owner', 'draft-1', 2, wav, 7)
    await speakersApi.deleteSample('owner', 'draft-1', 2, 8)

    const [uploadUrl, uploadInit] = call(fetch, 0)
    expect(uploadUrl).toBe('/api/admin/speakers/owner/enrollments/draft-1/samples/2')
    expect(uploadInit.method).toBe('PUT')
    expect((uploadInit.headers as Headers).get('If-Match')).toBe('"7"')
    expect((uploadInit.headers as Headers).get('Content-Type')).toBe('audio/wav')
    expect(uploadInit.body).toBe(wav)

    const [deleteUrl, deleteInit] = call(fetch, 1)
    expect(deleteUrl).toBe('/api/admin/speakers/owner/enrollments/draft-1/samples/2')
    expect(deleteInit.method).toBe('DELETE')
    expect((deleteInit.headers as Headers).get('If-Match')).toBe('"8"')
  })
})
