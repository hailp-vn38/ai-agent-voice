import { jsonRequest, requestJson, withQuery } from './client'
import type { MemoryMode } from './types/devices'

export type FactKind = 'user' | 'preference' | 'business' | 'decision' | 'reference'

export const FACT_KINDS: readonly FactKind[] = ['user', 'preference', 'business', 'decision', 'reference']

export interface MemoryFact {
  id: string
  revision: number
  title: string
  hook: string
  body: string
  kind: FactKind
  origin: string
  status: string
  pinned: boolean
  created_at: string
  updated_at: string
}

export interface MemoryReceipt {
  action: string
  fact_id: string | null
  title: string | null
  revision: number | null
  at: string
  /** Committed, but some cleanup did not finish; a rebuild completes it. */
  cleanup_pending: boolean
}

export interface MemorySummary {
  summary: string
  hidden: number
}

export interface MemoryIndex {
  facts: number
  quarantined: number
  quarantined_files: string[]
  memory_mode: MemoryMode
}

export interface MemoryRebuildReport {
  facts: number
  quarantined: number
  tombstones_applied: number
  owner_edited: number
  temp_files_removed: number
}

export interface MemoryProfile {
  body: string
  revision: number
  updated_at: string | null
}

export interface CreateFactInput {
  title: string
  hook: string
  kind: FactKind
  body: string
}

export interface UpdateFactInput {
  title: string
  hook: string
  body: string
  expected_revision: number
}

const base = '/api/admin/memory'

function factPath(device: string, id: string, action?: string) {
  const path = `${base}/facts/${encodeURIComponent(id)}${action ? `/${action}` : ''}`
  return withQuery(path, { device })
}

export const memoryApi = {
  facts(device: string, q?: string) {
    return requestJson<{ items: MemoryFact[] }>(withQuery(`${base}/facts`, { device, q }))
  },
  fact(device: string, id: string) {
    return requestJson<MemoryFact>(factPath(device, id))
  },
  create(device: string, input: CreateFactInput) {
    return requestJson<MemoryReceipt>(withQuery(`${base}/facts`, { device }), jsonRequest('POST', input))
  },
  update(device: string, id: string, input: UpdateFactInput) {
    return requestJson<MemoryReceipt>(factPath(device, id), jsonRequest('PUT', input))
  },
  forget(device: string, id: string, expectedRevision: number) {
    return requestJson<MemoryReceipt>(factPath(device, id, 'forget'), jsonRequest('POST', { expected_revision: expectedRevision }))
  },
  discard(device: string, id: string, expectedRevision: number) {
    return requestJson<MemoryReceipt>(factPath(device, id, 'discard'), jsonRequest('POST', { expected_revision: expectedRevision }))
  },
  pin(device: string, id: string, pinned: boolean) {
    return requestJson<MemoryReceipt>(factPath(device, id, 'pin'), jsonRequest('POST', { pinned }))
  },
  summary(device: string) {
    return requestJson<MemorySummary>(withQuery(`${base}/summary`, { device }))
  },
  index(device: string) {
    return requestJson<MemoryIndex>(withQuery(`${base}/index`, { device }))
  },
  rebuild(device: string) {
    return requestJson<MemoryRebuildReport>(withQuery(`${base}/index/rebuild`, { device }), { method: 'POST' })
  },
  profile() {
    return requestJson<MemoryProfile>(`${base}/profile`)
  },
  saveProfile(body: string, expectedRevision: number) {
    return requestJson<MemoryReceipt>(`${base}/profile`, jsonRequest('POST', { body, expected_revision: expectedRevision }))
  },
}
