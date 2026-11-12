import { readAdminWebToken } from '@/stores/auth'

import { toApiError } from './errors'
import { applySessionFailure } from './session'
import { reportConnectionLost } from '@/composables/useNotify'

const rawBaseUrl = import.meta.env.VITE_API_BASE_URL?.trim() ?? ''
const apiBaseUrl = rawBaseUrl.replace(/\/+$/, '')

export interface RequestOptions {
  revision?: number
  signal?: AbortSignal
  headers?: HeadersInit
  body?: BodyInit | null
  /** Login and status calls carry no session, so a 401 from them must not sign the Admin out. */
  anonymous?: boolean
}

export type QueryValue = string | number | boolean | null | undefined

export function withQuery(path: string, query: Record<string, QueryValue>): string {
  const parameters = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== null) parameters.set(key, String(value))
  }
  const serialized = parameters.toString()
  return serialized ? `${path}?${serialized}` : path
}

export function jsonRequest(method: string, body: unknown): RequestInit {
  return {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }
}

export function apiUrl(path: string): string {
  const normalizedPath = path.startsWith('/') ? path : `/${path}`
  return `${apiBaseUrl}${normalizedPath}`
}

function requestHeaders(initHeaders: HeadersInit | undefined, optionHeaders: HeadersInit | undefined, revision: number | undefined, optionsAnonymous: boolean | undefined): Headers {
  const headers = new Headers(initHeaders)
  new Headers(optionHeaders).forEach((value, key) => headers.set(key, value))
  const token = optionsAnonymous ? null : readAdminWebToken()
  if (token) headers.set('Authorization', `Bearer ${token}`)
  if (revision !== undefined) headers.set('If-Match', `"${revision}"`)
  return headers
}

/** Statuses a reverse proxy returns when the server behind it is down. */
const GATEWAY_FAILURES = new Set([502, 503, 504])

export async function request(path: string, init: RequestInit = {}, options: RequestOptions = {}) {
  let response: Response
  try {
    response = await fetch(apiUrl(path), {
      ...init,
      body: options.body ?? init.body,
      headers: requestHeaders(init.headers, options.headers, options.revision, options.anonymous),
      signal: options.signal,
    })
  } catch (error) {
    // fetch rejects with a TypeError when the network or server is unreachable.
    if (error instanceof TypeError && !options.signal?.aborted) reportConnectionLost()
    throw error
  }
  if (!response.ok) {
    const error = await toApiError(response)
    if (!options.anonymous) applySessionFailure(error)
    // Only a bare gateway failure means the server is gone; a JSON 503 is the server speaking.
    if (GATEWAY_FAILURES.has(error.status) && error.code === `http_${error.status}`) reportConnectionLost()
    throw error
  }
  return response
}

export async function requestJson<T>(path: string, init: RequestInit = {}, options: RequestOptions = {}): Promise<T> {
  const response = await request(path, init, options)
  if (response.status === 204) return undefined as T
  return (await response.json()) as T
}

export async function requestText(path: string, init: RequestInit = {}, options: RequestOptions = {}) {
  return (await request(path, init, options)).text()
}

export async function requestBlob(path: string, init: RequestInit = {}, options: RequestOptions = {}) {
  return (await request(path, init, options)).blob()
}

export async function requestAudio(path: string, init: RequestInit, options: RequestOptions = {}) {
  const response = await request(path, init, options)
  if (response.headers.get('content-type')?.split(';')[0] !== 'audio/wav') throw new Error('invalid_audio_response')
  return { audio: await response.blob(), elapsedMs: Number(response.headers.get('x-provider-test-elapsed-ms') ?? 0) }
}
