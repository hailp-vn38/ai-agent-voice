import { inject, type InjectionKey } from 'vue'

import { jsonRequest, requestJson, withQuery } from '@/api/client'

export type KnowledgeSpace = 'raw' | 'wiki' | 'drafts'
export type KnowledgeTreeNode = {
  type: 'file' | 'folder'
  space: KnowledgeSpace
  path: string
  name: string
  editable: boolean
  id?: string
  revision?: number
  status?: string
  [key: string]: unknown
}
export type KnowledgePage = {
  id: string
  title: string
  body: string
  revision: number
  status?: string
  source_ref?: string | null
  path?: string
  [key: string]: unknown
}
export type KnowledgeRawFile = { path: string; body: string; sha256: string; [key: string]: unknown }
export type KnowledgeHit = {
  id: string
  title: string
  revision: number
  excerpt: string
  path?: string
  relative_path?: string
  space?: KnowledgeSpace
  [key: string]: unknown
}
export type KnowledgeGraph = { nodes: unknown[]; edges: unknown[]; unresolved?: unknown[]; [key: string]: unknown }
export type KnowledgeStatus = { health: string; document_count: number; last_complete_scan_at: number | null; [key: string]: unknown }

export type VaultErrorKind = 'not_found' | 'invalid_path' | 'conflict' | 'read_only' | 'io' | 'index'
export class VaultError extends Error {
  constructor(readonly kind: VaultErrorKind, message: string, readonly current?: string | number) {
    super(message)
    this.name = 'VaultError'
  }
}

export type Change =
  | { type: 'write'; path: string; body: string; expect?: string | number; title?: string; sourceRef?: string | null; folderPath?: string; fileName?: string }
  | { type: 'patch'; path: string; body: string; expect?: string | number }
  | { type: 'move'; from: string; to: string; expect?: string | number }
  | { type: 'delete'; path: string; expect?: string | number }
  | { type: 'import_raw'; folder: string; files: Array<{ path: string; body: string }> }
  | { type: 'ingest_source'; path: string; expect?: string; name?: string; provenance?: string }
  | { type: 'create_folder'; space: KnowledgeSpace; parentPath: string; name: string }
  | { type: 'create_source'; name: string; provenance: string; content: string }
  | { type: 'publish'; path: string; expect: string | number; sourceRef: string }
  | { type: 'reindex' }

export type KnowledgeChangeEvent = {
  type: 'changed' | 'external' | 'reindexed'
  paths?: string[]
  origin?: 'ui' | 'mcp' | 'server' | 'external'
  status?: KnowledgeStatus
}
export type KnowledgeSearchOptions = { space?: KnowledgeSpace; mode?: 'hybrid' | 'lex' | 'vec'; limit?: number; cursor?: string }
export type KnowledgeGraphOptions = { scope?: 'global' | 'local' | 'query'; centerId?: string; depth?: number; query?: string; limit?: number }

/** Public data seam shared by Admin Web and the desktop shell. */
export interface KnowledgeBackend {
  readonly capabilities: { chat: boolean }
  tree(space: KnowledgeSpace): Promise<{ items: KnowledgeTreeNode[]; [key: string]: unknown }>
  read(path: string, space?: KnowledgeSpace): Promise<KnowledgePage | KnowledgeRawFile>
  search(query: string, options?: KnowledgeSearchOptions): Promise<{ items: KnowledgeHit[]; next_cursor?: string | null; [key: string]: unknown }>
  graph(options?: KnowledgeGraphOptions): Promise<KnowledgeGraph>
  status(): Promise<KnowledgeStatus>
  apply(change: Change): Promise<unknown>
  sources(): Promise<Array<{ id: string; version: number; name: string; provenance: string; sha256: string }>>
  readSource(id: string): Promise<{ text: string }>
  chart(fileName: string): Promise<unknown>
  onChange(listener: (event: KnowledgeChangeEvent) => void): () => void
}

type RequestJson = <T>(path: string, init?: RequestInit) => Promise<T>

/** HTTP adapter for the existing Admin Knowledge routes. */
export class HttpKnowledgeBackend implements KnowledgeBackend {
  readonly capabilities = { chat: true } as const
  constructor(private readonly request: RequestJson = requestJson, private readonly pollIntervalMs = 2_000) {}

  tree(space: KnowledgeSpace) {
    return this.request<{ items: KnowledgeTreeNode[] }>(withQuery('/api/admin/knowledge/tree', { space }))
  }

  read(path: string, space: KnowledgeSpace = path.startsWith('raw/') ? 'raw' : 'wiki') {
    return space === 'raw'
      ? this.request<KnowledgeRawFile>(withQuery('/api/admin/knowledge/raw/files', { path }))
      : this.request<KnowledgePage>(`/api/admin/knowledge/pages/${encodeURIComponent(path)}`)
  }

  search(query: string, options: KnowledgeSearchOptions = {}) {
    const params: Record<string, string | number | undefined> = {}
    if (query) params.q = query
    if (options.space) params.space = options.space
    if (options.mode) params.mode = options.mode
    params.limit = options.limit ?? 20
    if (options.cursor) params.after = options.cursor
    return this.request<{ items: KnowledgeHit[]; next_cursor?: string | null }>(withQuery('/api/admin/knowledge/pages', params))
  }

  graph(options: KnowledgeGraphOptions = {}) {
    const params: Record<string, string | number> = { scope: options.scope ?? 'global', limit: options.limit ?? 500 }
    if (options.centerId !== undefined) params.center_id = options.centerId
    if (options.depth !== undefined) params.depth = options.depth
    if (options.query !== undefined) params.q = options.query
    return this.request<KnowledgeGraph>(withQuery('/api/admin/knowledge/graph', params))
  }

  status() {
    return this.request<KnowledgeStatus>('/api/admin/knowledge/index')
  }

  async apply(change: Change): Promise<unknown> {
    switch (change.type) {
      case 'write': {
        const raw = change.path.startsWith('raw/')
        if (raw && change.folderPath && change.fileName) {
          return this.request('/api/admin/knowledge/raw/files', jsonRequest('POST', {
            folder_path: change.folderPath, file_name: change.fileName, body: change.body,
          }))
        }
        const path = raw ? '/api/admin/knowledge/raw/files' : change.expect === 0
          ? '/api/admin/knowledge/pages' : `/api/admin/knowledge/pages/${encodeURIComponent(change.path)}`
        return this.request(path, jsonRequest(raw ? 'PUT' : change.expect === 0 ? 'POST' : 'PUT', raw
          ? { path: change.path, body: change.body, expected_sha256: change.expect }
          : { ...(change.expect === 0 ? { folder_path: change.folderPath, file_name: change.fileName } : { id: change.path }),
            title: change.title, body: change.body, source_ref: change.sourceRef ?? null, expected_revision: change.expect }))
      }
      case 'patch':
        return this.request(`/api/admin/knowledge/pages/${encodeURIComponent(change.path)}`, jsonRequest('PATCH', {
          body: change.body, expected_revision: change.expect,
        }))
      case 'move':
        {
          const pieces = change.to.split('/')
          const fileName = pieces.pop() ?? ''
          const targetFolder = pieces.join('/')
          const raw = change.from.startsWith('raw/')
          return this.request(raw ? '/api/admin/knowledge/raw/move'
            : `/api/admin/knowledge/pages/${encodeURIComponent(change.from)}/move`, jsonRequest('POST', raw
            ? { path: change.from, target_folder_path: targetFolder, file_name: fileName, expected_sha256: change.expect }
            : { target_folder_path: targetFolder, file_name: fileName, expected_revision: change.expect }))
        }
      case 'delete':
        return this.request(change.path.startsWith('raw/')
          ? withQuery('/api/admin/knowledge/raw/files', { path: change.path })
          : `/api/admin/knowledge/pages/${encodeURIComponent(change.path)}`, { method: 'DELETE' })
      case 'import_raw':
        return this.request('/api/admin/knowledge/raw/batch', jsonRequest('POST', {
          folder_path: change.folder, files: change.files,
        }))
      case 'ingest_source':
        return this.request('/api/admin/knowledge/raw/import-source', jsonRequest('POST', {
          path: change.path, expected_sha256: change.expect,
          name: change.name ?? change.path.split('/').pop(), provenance: change.provenance ?? 'admin:manual',
        }))
      case 'create_folder':
        return this.request('/api/admin/knowledge/folders', jsonRequest('POST', {
          space: change.space, parent_path: change.parentPath, name: change.name,
        }))
      case 'create_source':
        return this.request('/api/admin/knowledge/sources', jsonRequest('POST', {
          name: change.name, provenance: change.provenance, content: change.content,
        }))
      case 'publish':
        return this.request(`/api/admin/knowledge/pages/${encodeURIComponent(change.path)}/publish`, jsonRequest('POST', {
          expected_revision: change.expect, source_ref: change.sourceRef,
        }))
      case 'reindex':
        return this.request('/api/admin/knowledge/index/rebuild', { method: 'POST' })
    }
  }

  async sources() {
    const response = await this.request<{ items: Array<{ id: string; version: number; name: string; provenance: string; sha256: string }> }>('/api/admin/knowledge/sources')
    return response.items
  }

  readSource(id: string) {
    return this.request<{ text: string }>(`/api/admin/knowledge/sources/${encodeURIComponent(id)}`)
  }

  chart(fileName: string) {
    return this.request<unknown>(`/api/admin/knowledge/charts/${encodeURIComponent(fileName)}`)
  }

  onChange(listener: (event: KnowledgeChangeEvent) => void) {
    let stopped = false
    let previous: string | null = null
    let timer: ReturnType<typeof setInterval> | undefined
    const poll = async () => {
      try {
        const status = await this.status()
        const next = JSON.stringify(status)
        if (previous !== null && previous !== next) listener({ type: 'external', origin: 'server', status })
        previous = next
      } catch { /* A transient network error is retried on the next poll. */ }
    }
    timer = setInterval(() => { if (!stopped) void poll() }, this.pollIntervalMs)
    return () => {
      stopped = true
      if (timer) clearInterval(timer)
    }
  }
}

export type MemoryKnowledgeSeed = { pages?: KnowledgePage[]; rawFiles?: KnowledgeRawFile[]; status?: Partial<KnowledgeStatus> }

/** Deterministic in-memory adapter used by Knowledge UI tests. */
export class MemoryKnowledgeBackend implements KnowledgeBackend {
  readonly capabilities = { chat: false } as const
  private readonly pages = new Map<string, KnowledgePage>()
  private readonly rawFiles = new Map<string, KnowledgeRawFile>()
  private readonly listeners = new Set<(event: KnowledgeChangeEvent) => void>()
  private currentStatus: KnowledgeStatus

  constructor(seed: MemoryKnowledgeSeed = {}) {
    for (const page of seed.pages ?? []) this.pages.set(page.path ?? page.id, { ...page, path: page.path ?? page.id })
    for (const file of seed.rawFiles ?? []) this.rawFiles.set(file.path, { ...file })
    this.currentStatus = { health: 'healthy', document_count: this.pages.size + this.rawFiles.size, last_complete_scan_at: null, ...seed.status }
  }

  async tree(space: KnowledgeSpace) {
    const files = space === 'wiki' ? this.pages : this.rawFiles
    return { items: [...files.values()].map(file => ({
      type: 'file' as const, space, path: file.path ?? '', name: (file.path ?? '').split('/').at(-1) ?? '', editable: true,
      ...(space === 'wiki' ? { id: (file as KnowledgePage).id, revision: (file as KnowledgePage).revision } : {}),
    })) }
  }

  async read(path: string, space: KnowledgeSpace = path.startsWith('raw/') ? 'raw' : 'wiki') {
    const file = (space === 'raw' ? this.rawFiles : this.pages).get(path)
    if (!file) throw new VaultError('not_found', `Knowledge file not found: ${path}`)
    return { ...file } as KnowledgePage | KnowledgeRawFile
  }

  async search(query: string, options: KnowledgeSearchOptions = {}) {
    const normalized = query.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLocaleLowerCase()
    const source: Array<KnowledgePage | KnowledgeRawFile> = options.space === 'raw'
      ? [...this.rawFiles.values()] : [...this.pages.values()]
    const items = [...source.values()].filter(file => {
      const text = `${file.path} ${'title' in file ? file.title : ''} ${file.body}`
        .normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLocaleLowerCase()
      return text.includes(normalized)
    }).slice(0, options.limit ?? 20).map(file => {
      const page = file as KnowledgePage
      const path = typeof file.path === 'string' ? file.path : page.id
      return {
        id: typeof page.id === 'string' ? page.id : path,
        title: typeof page.title === 'string' ? page.title : path,
        revision: typeof page.revision === 'number' ? page.revision : 1,
        excerpt: file.body.slice(0, 180), path, space: options.space ?? 'wiki',
      } satisfies KnowledgeHit
    })
    return { items, next_cursor: null }
  }

  async graph(): Promise<KnowledgeGraph> { return { nodes: [], edges: [], unresolved: [], truncated: false } }
  async status() { return { ...this.currentStatus } }

  async apply(change: Change) {
    if (change.type === 'reindex') {
      this.currentStatus = { ...this.currentStatus, document_count: this.pages.size + this.rawFiles.size, last_complete_scan_at: Date.now() }
      this.publish({ type: 'reindexed', status: this.currentStatus })
      return this.status()
    }
    if (change.type === 'write' || change.type === 'patch') {
      const raw = change.path.startsWith('raw/')
      if (change.type === 'write' && raw && change.folderPath && change.fileName) {
        const updated = { path: `${change.folderPath}/${change.fileName}`, body: change.body, sha256: 'memory-sha' }
        this.rawFiles.set(updated.path, updated)
        this.publish({ type: 'changed', paths: [updated.path], origin: 'ui' })
        return updated
      }
      const current = raw ? this.rawFiles.get(change.path) : this.pages.get(change.path)
      const revision = raw ? (current ? 1 : 0) : (current as KnowledgePage | undefined)?.revision ?? 0
      if (change.expect !== undefined && String(change.expect) !== String(revision)) {
        throw new VaultError('conflict', 'Knowledge file changed since it was read', revision ? String(revision) : undefined)
      }
      const updated: KnowledgePage | KnowledgeRawFile = raw
        ? { path: change.path, body: change.body, sha256: String(revision + 1) }
        : { id: (current as KnowledgePage | undefined)?.id ?? change.path, path: change.path,
          title: (current as KnowledgePage | undefined)?.title ?? change.path.split('/').at(-1) ?? change.path,
          body: change.body, revision: revision + 1 }
      if (raw) this.rawFiles.set(change.path, updated as KnowledgeRawFile)
      else this.pages.set(change.path, updated as KnowledgePage)
      this.currentStatus = { ...this.currentStatus, document_count: this.pages.size + this.rawFiles.size }
      this.publish({ type: 'changed', paths: [change.path], origin: 'ui' })
      return updated
    }
    if (change.type === 'delete') {
      const removed = change.path.startsWith('raw/')
        ? this.rawFiles.delete(change.path) : this.pages.delete(change.path)
      if (!removed) throw new VaultError('not_found', `Knowledge file not found: ${change.path}`)
      this.publish({ type: 'changed', paths: [change.path], origin: 'ui' })
      return
    }
    if (change.type === 'move') {
      const raw = change.from.startsWith('raw/')
      const file = raw ? this.rawFiles.get(change.from) : this.pages.get(change.from)
      if (!file) throw new VaultError('not_found', `Knowledge file not found: ${change.from}`)
      if (raw) {
        this.rawFiles.delete(change.from)
        this.rawFiles.set(change.to, { ...(file as KnowledgeRawFile), path: change.to })
      } else {
        this.pages.delete(change.from)
        this.pages.set(change.to, { ...(file as KnowledgePage), path: change.to })
      }
      this.publish({ type: 'changed', paths: [change.from, change.to], origin: 'ui' })
      return { path: change.to }
    }
    if (change.type === 'import_raw') {
      const paths = change.files.map(file => `${change.folder.replace(/\/$/, '')}/${file.path}`)
      for (const [index, file] of change.files.entries()) {
        const path = paths[index]!
        this.rawFiles.set(path, { path, body: file.body, sha256: String(Date.now()) })
      }
      this.currentStatus = { ...this.currentStatus, document_count: this.pages.size + this.rawFiles.size }
      this.publish({ type: 'changed', paths, origin: 'ui' })
      return { created: paths, skipped: [], failed: [] }
    }
    if (change.type === 'ingest_source') {
      this.publish({ type: 'changed', paths: [change.path], origin: 'ui' })
      return { path: change.path }
    }
    if (change.type === 'create_folder') return { path: `${change.parentPath}/${change.name}` }
    if (change.type === 'create_source') return { id: 'memory-source', version: 1, name: change.name, provenance: change.provenance, sha256: '' }
    if (change.type === 'publish') {
      const page = this.pages.get(change.path)
      if (!page) throw new VaultError('not_found', `Knowledge page not found: ${change.path}`)
      const updated = { ...page, status: 'verified', source_ref: change.sourceRef }
      this.pages.set(change.path, updated)
      this.publish({ type: 'changed', paths: [change.path], origin: 'ui' })
      return updated
    }
  }

  onChange(listener: (event: KnowledgeChangeEvent) => void) {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  async sources() { return [] }
  async readSource(id: string): Promise<{ text: string }> { throw new VaultError('not_found', `Knowledge source not found: ${id}`) }
  async chart(fileName: string) { throw new VaultError('not_found', `Knowledge chart not found: ${fileName}`) }

  private publish(event: KnowledgeChangeEvent) { for (const listener of this.listeners) listener(event) }
}

export const KNOWLEDGE_BACKEND_KEY: InjectionKey<KnowledgeBackend> = Symbol('KnowledgeBackend')
const defaultBackend = new HttpKnowledgeBackend()
export function useKnowledgeBackend() {
  return inject(KNOWLEDGE_BACKEND_KEY, defaultBackend)
}

export function provideKnowledgeBackend(backend: KnowledgeBackend) {
  return { key: KNOWLEDGE_BACKEND_KEY, backend }
}

export const knowledgeBackend = new HttpKnowledgeBackend()
