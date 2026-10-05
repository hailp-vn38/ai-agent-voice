import { readAdminToken } from '@/stores/auth'

import { toApiError } from './errors'

const rawBaseUrl = import.meta.env.VITE_API_BASE_URL?.trim() ?? ''
const apiBaseUrl = rawBaseUrl.replace(/\/+$/, '')

export interface RequestOptions {
  revision?: number
  signal?: AbortSignal
  headers?: HeadersInit
  body?: BodyInit | null
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

function requestHeaders(initHeaders: HeadersInit | undefined, optionHeaders: HeadersInit | undefined, revision: number | undefined): Headers {
  const headers = new Headers(initHeaders)
  new Headers(optionHeaders).forEach((value, key) => headers.set(key, value))
  const token = readAdminToken()
  if (token) headers.set('Authorization', `Bearer ${token}`)
  if (revision !== undefined) headers.set('If-Match', `"${revision}"`)
  return headers
}

export async function request(path: string, init: RequestInit = {}, options: RequestOptions = {}) {
  const response = await fetch(apiUrl(path), {
    ...init,
    body: options.body ?? init.body,
    headers: requestHeaders(init.headers, options.headers, options.revision),
    signal: options.signal,
  })
  if (!response.ok) throw await toApiError(response)
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
