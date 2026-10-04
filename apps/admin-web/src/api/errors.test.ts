import { describe, expect, it } from 'vitest'

import { ApiError, formatApiError, isApiError, toApiError } from './errors'

describe('API errors', () => {
  it('parses the server envelope and falls back for non-JSON failures', async () => {
    await expect(toApiError(new Response(JSON.stringify({ error: { code: 'not_found', request_id: 'req-9' } }), { status: 404 })))
      .resolves.toEqual(new ApiError('API request failed: not_found', 404, 'not_found', 'req-9'))
    await expect(toApiError(new Response('gateway failure', { status: 502 })))
      .resolves.toEqual(new ApiError('API request failed: http_502', 502, 'http_502'))
  })

  it('formats known error codes, request ids, and generic errors for the UI', () => {
    const error = new ApiError('ignored', 409, 'revision_conflict', 'req-3')
    expect(isApiError(error)).toBe(true)
    expect(formatApiError(error)).toBe('Dữ liệu đã thay đổi trên server. Đã tải lại bản mới nhất. (request_id: req-3)')
    expect(formatApiError(new Error('offline'))).toBe('offline')
    expect(formatApiError('offline')).toBe('Unknown API error')
  })
})
