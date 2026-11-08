import { inject, type InjectionKey } from 'vue'

import { jsonRequest, requestJson, withQuery } from '@/api/client'

export type KnowledgeSpace = 'raw' | 'wiki' | 'drafts'
export type Scope = 'raw' | 'wiki' | 'all'
export type DocumentScope = Exclude<Scope, 'all'>
export type DocPath = { scope: DocumentScope; relative: string }
export type ReadWindow = { from_line?: number; max_lines?: number }
export type VaultSearchOptions = { scope?: Scope; mode?: 'hybrid' | 'lex' | 'vec'; limit?: number }
export type VaultTreeNode = { name: string; path: string; is_directory: boolean; children: VaultTreeNode[] }
export type VaultPage = { path: string; title: string; content: string; revision: string }
export type VaultHit = { path: string; title: string; score: number; line: number; snippet: string }
export type VaultGraph = {
  nodes: Array<{ path: string; label: string; title: string; inbound: number; outbound: number }>
  edges: Array<{ source: string; target: string | null; raw_target: string; line: number }>
  unresolved: string[]
}
export type VaultStatus = { raw_documents: number; wiki_pages: number; graph_links: number; resolved_links: number; unresolved_links: number }
export type TextEdit = { start_line: number; delete_lines: number; insert: string }
export type VaultChange =
  | { type: 'write'; path: DocPath; content: string; expect?: string | null }
  | { type: 'patch'; path: DocPath; edits: TextEdit[]; expect?: string | null }
  | { type: 'move'; from: DocPath; to: DocPath }
  | { type: 'delete'; path: DocPath }
  | { type: 'import_raw'; files: Array<{ path: DocPath; content: string }> }
  | { type: 'ingest_source'; source: DocPath }
  | { type: 'reindex'; force: boolean }
export type Applied = { paths: string[]; revision: string | null }
export type VaultEvent =
  | { type: 'changed'; paths: string[]; origin: 'ui' | 'mcp' | 'server' }
  | { type: 'external'; paths: string[] }
  | { type: 'reindexed'; stats: { raw_documents: number; wiki_pages: number; total_chunks: number } }
  | { type: 'lock'; writable: boolean }

export function docPath(scope: DocumentScope, relative: string): DocPath {
  const pieces = relative.split('/')
  if (!relative || relative.startsWith('/') || relative.includes('\\') || relative.includes(':')
    || relative.includes('\0') || !relative.endsWith('.md') || pieces.some(piece => !piece || piece === '.' || piece === '..')) {
    throw new VaultError('invalid_path', `Invalid ${scope} document path: ${relative}`)
  }
  return { scope, relative }
}

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
  tree(scope: Scope): Promise<VaultTreeNode>
  read(path: DocPath, window?: ReadWindow): Promise<VaultPage>
  search(query: string, options?: VaultSearchOptions): Promise<VaultHit[]>
  graph(focus?: DocPath): Promise<VaultGraph>
  status(): Promise<VaultStatus>
  apply(change: VaultChange): Promise<Applied>
  onChange(listener: (event: VaultEvent) => void): () => void
}

/** Admin-only routes and DTOs that have no equivalent in wiki-core::Vault. */
export interface AdminKnowledgeBackend {
  readonly capabilities: { chat: boolean }
  tree(space: KnowledgeSpace): Promise<{ items: KnowledgeTreeNode[]; [key: string]: unknown }>
  treeMetadata?(space: KnowledgeSpace): Promise<KnowledgeTreeNode[]>
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

export interface AdminKnowledgeExtension {
  readonly admin: AdminKnowledgeBackend
}

type RequestJson = <T>(path: string, init?: RequestInit) => Promise<T>

/** HTTP adapter for the existing Admin Knowledge routes. */
class HttpAdminKnowledgeBackend implements AdminKnowledgeBackend {
  readonly capabilities = { chat: true } as const
  private readonly lastTrees = new Map<KnowledgeSpace, KnowledgeTreeNode[]>()
  constructor(private readonly request: RequestJson = requestJson, private readonly pollIntervalMs = 2_000) {}

  tree(space: KnowledgeSpace) {
    return this.request<{ items: KnowledgeTreeNode[] }>(withQuery('/api/admin/knowledge/tree', { space }))
      .then(result => { this.lastTrees.set(space, result.items); return result })
  }

  async treeMetadata(space: KnowledgeSpace) {
    return this.lastTrees.get(space) ?? (await this.tree(space)).items
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
        if (raw && change.expect == null) {
          const parts = change.path.split('/')
          const fileName = parts.pop() ?? ''
          return this.request('/api/admin/knowledge/raw/files', jsonRequest('POST', {
            folder_path: parts.join('/') || 'raw', file_name: fileName, body: change.body,
          }))
        }
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

export type MemoryKnowledgeSeed = {
  pages?: KnowledgePage[]
  rawFiles?: KnowledgeRawFile[]
  status?: Partial<KnowledgeStatus>
  graph?: KnowledgeGraph
  graphError?: unknown
}

/** Deterministic in-memory adapter used by Knowledge UI tests. */
class MemoryAdminKnowledgeBackend implements AdminKnowledgeBackend {
  readonly capabilities = { chat: false } as const
  private readonly pages = new Map<string, KnowledgePage>()
  private readonly rawFiles = new Map<string, KnowledgeRawFile>()
  private readonly folders = new Map<KnowledgeSpace, Set<string>>([
    ['wiki', new Set(['wiki'])], ['raw', new Set(['raw'])], ['drafts', new Set(['drafts'])],
  ])
  private readonly listeners = new Set<(event: KnowledgeChangeEvent) => void>()
  private currentStatus: KnowledgeStatus
  private readonly graphSnapshot: KnowledgeGraph
  private readonly graphFailure: unknown
  private readonly lastTrees = new Map<KnowledgeSpace, KnowledgeTreeNode[]>()

  constructor(seed: MemoryKnowledgeSeed = {}) {
    for (const page of seed.pages ?? []) {
      const path = page.path ?? page.id
      this.pages.set(path, { ...page, path })
      this.addParentFolders('wiki', path)
    }
    for (const file of seed.rawFiles ?? []) {
      this.rawFiles.set(file.path, { ...file })
      this.addParentFolders('raw', file.path)
    }
    this.currentStatus = { health: 'healthy', document_count: this.pages.size + this.rawFiles.size, last_complete_scan_at: null, ...seed.status }
    this.graphSnapshot = seed.graph ?? { nodes: [], edges: [], unresolved: [], truncated: false }
    this.graphFailure = seed.graphError
  }

  async tree(space: KnowledgeSpace) {
    const files = space === 'wiki' ? this.pages : this.rawFiles
    const folders = [...(this.folders.get(space) ?? [])].map(path => ({
      type: 'folder' as const, space, path, name: path.split('/').at(-1) ?? path, editable: true,
    }))
    const notes = [...files.values()].map(file => ({
      type: 'file' as const, space, path: file.path ?? '', name: (file.path ?? '').split('/').at(-1) ?? '', editable: true,
      ...(space === 'wiki' ? { id: (file as KnowledgePage).id, revision: (file as KnowledgePage).revision } : {}),
    }))
    const items = [...folders, ...notes]
    this.lastTrees.set(space, items)
    return { items }
  }

  async treeMetadata(space: KnowledgeSpace) { return this.lastTrees.get(space) ?? (await this.tree(space)).items }

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
      const pathTitle = path.split('/').at(-1)?.replace(/\.md$/i, '').replace(/[-_]+/g, ' ')
        .replace(/\b\p{L}/gu, value => value.toLocaleUpperCase()) ?? path
      const line = file.body.split('\n').findIndex(value => value.normalize('NFD')
        .replace(/[\u0300-\u036f]/g, '').toLocaleLowerCase().includes(normalized))
      return {
        id: typeof page.id === 'string' ? page.id : path,
        title: typeof page.title === 'string' ? page.title : pathTitle,
        revision: typeof page.revision === 'number' ? page.revision : 1,
        excerpt: file.body.slice(0, 180), path, space: options.space ?? 'wiki',
        match_line: line < 0 ? 0 : line + 1,
        score: 1,
      } satisfies KnowledgeHit
    })
    return { items, next_cursor: null }
  }

  async graph(): Promise<KnowledgeGraph> {
    if (this.graphFailure) throw this.graphFailure
    return structuredClone(this.graphSnapshot)
  }
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
        const updated = { path: `${change.folderPath}/${change.fileName}`, body: change.body, sha256: 'memory-rev-1' }
        this.rawFiles.set(updated.path, updated)
        this.addParentFolders('raw', updated.path)
        this.publish({ type: 'changed', paths: [updated.path], origin: 'ui' })
        return updated
      }
      const current = raw ? this.rawFiles.get(change.path) : this.pages.get(change.path)
      const revision: string | number | undefined = raw
        ? (current as KnowledgeRawFile | undefined)?.sha256
        : (current as KnowledgePage | undefined)?.revision
      if (change.expect !== undefined && String(change.expect) !== String(revision ?? 0)) {
        throw new VaultError('conflict', 'Knowledge file changed since it was read', revision === undefined ? undefined : String(revision))
      }
      const updated: KnowledgePage | KnowledgeRawFile = raw
        ? { path: change.path, body: change.body, sha256: `memory-rev-${(Number(String(revision).match(/\d+$/)?.[0]) || 0) + 1}` }
        : { id: (current as KnowledgePage | undefined)?.id ?? change.path, path: change.path,
          title: (current as KnowledgePage | undefined)?.title ?? change.path.split('/').at(-1) ?? change.path,
          body: change.body, revision: Number(revision ?? 0) + 1 }
      if (raw) this.rawFiles.set(change.path, updated as KnowledgeRawFile)
      else this.pages.set(change.path, updated as KnowledgePage)
      this.addParentFolders(raw ? 'raw' : 'wiki', change.path)
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
    if (change.type === 'create_folder') {
      const path = `${change.parentPath}/${change.name}`
      this.addParentFolders(change.space, path + '/.folder')
      return { path }
    }
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
  private addParentFolders(space: KnowledgeSpace, path: string) {
    const folders = this.folders.get(space) ?? new Set<string>([space])
    let parent = ''
    for (const segment of path.split('/').slice(0, -1)) {
      parent = parent ? `${parent}/${segment}` : segment
      folders.add(parent)
    }
    this.folders.set(space, folders)
  }
}

function adminPath(path: DocPath): string {
  return `${path.scope === 'raw' ? 'raw' : 'wiki'}/${path.relative}`
}

function vaultPath(path: DocPath): string {
  return `${path.scope === 'raw' ? 'sources' : 'wiki'}/${path.relative}`
}

function treeFromAdmin(items: KnowledgeTreeNode[], scope: Scope): VaultTreeNode {
  const rootPath = scope === 'all' ? 'all' : scope === 'raw' ? 'sources' : 'wiki'
  const root: VaultTreeNode = { name: rootPath, path: rootPath, is_directory: true, children: [] }
  const byPath = new Map<string, VaultTreeNode>([[rootPath, root]])
  for (const item of items) {
    const path = item.space === 'raw' ? item.path.replace(/^raw\//, 'sources/') : item.path
    if (path === rootPath) continue
    byPath.set(path, { name: item.name, path, is_directory: item.type === 'folder', children: [] })
  }
  for (const [path, node] of byPath) {
    if (path === rootPath) continue
    const parentPath = path.slice(0, Math.max(0, path.lastIndexOf('/'))) || rootPath
    const parent = byPath.get(parentPath) ?? root
    if (!parent.children.some(child => child.path === path)) parent.children.push(node)
  }
  return root
}

function translateEvent(event: KnowledgeChangeEvent): VaultEvent {
  if (event.type === 'external') return { type: 'external', paths: event.paths ?? [] }
  if (event.type === 'reindexed') return { type: 'reindexed', stats: {
    raw_documents: Number(event.status?.raw_documents ?? 0), wiki_pages: Number(event.status?.wiki_pages ?? event.status?.document_count ?? 0), total_chunks: 0,
  } }
  return { type: 'changed', paths: event.paths ?? [], origin: event.origin === 'mcp' || event.origin === 'server' ? event.origin : 'ui' }
}

function applied(result: unknown, fallbackPath?: string): Applied {
  const value = result && typeof result === 'object' ? result as Record<string, unknown> : {}
  const paths = Array.isArray(value.paths) ? value.paths.filter((path): path is string => typeof path === 'string')
    : typeof value.path === 'string' ? [value.path] : fallbackPath ? [fallbackPath] : []
  const revision = typeof value.revision === 'string' || typeof value.revision === 'number' ? String(value.revision) : null
  return { paths, revision }
}

async function editContent(content: string, edits: TextEdit[]) {
  const lines = content.split('\n')
  for (const edit of [...edits].sort((a, b) => b.start_line - a.start_line)) lines.splice(edit.start_line, edit.delete_lines, ...edit.insert.split('\n'))
  return lines.join('\n')
}

abstract class VaultKnowledgeBackend implements KnowledgeBackend, AdminKnowledgeExtension {
  abstract readonly capabilities: { chat: boolean }
  readonly admin: AdminKnowledgeBackend
  protected readonly documentIdsByVaultPath = new Map<string, string>()
  protected constructor(admin: AdminKnowledgeBackend) { this.admin = admin }

  async tree(scope: Scope) {
    const spaces: KnowledgeSpace[] = scope === 'all' ? ['raw', 'wiki'] : [scope]
    const items = (await Promise.all(spaces.map(async space => (await this.admin.tree(space)).items))).flat()
    for (const item of items) {
      if (item.space === 'wiki' && item.id) this.documentIdsByVaultPath.set(item.path, item.id)
    }
    return treeFromAdmin(items, scope)
  }
  async read(path: DocPath, window: ReadWindow = {}) {
    const page = await this.admin.read(this.adminReadPath(path), path.scope)
    const body = 'body' in page ? page.body : ''
    const start = Math.max(0, window.from_line ?? 0)
    const content = body.split('\n').slice(start, window.max_lines === undefined ? undefined : start + window.max_lines).join('\n')
    return { path: vaultPath(path), title: 'title' in page ? String(page.title) : path.relative.split('/').at(-1) ?? path.relative,
      content, revision: String('revision' in page ? page.revision : page.sha256) }
  }
  protected adminReadPath(path: DocPath) { return adminPath(path) }
  async search(query: string, options: VaultSearchOptions = {}) {
    const result = await this.admin.search(query, {
      ...(options.scope === 'raw' || options.scope === 'wiki' ? { space: options.scope } : {}),
      ...(options.mode ? { mode: options.mode } : {}),
      ...(options.limit ? { limit: options.limit } : {}),
    })
    return result.items.map(hit => ({ path: (hit.path ?? hit.id).replace(/^raw\//, 'sources/'), title: hit.title, score: Number(hit.score ?? 0),
      line: Number(hit.match_line ?? 0), snippet: hit.excerpt ?? '' }))
  }
  async graph(focus?: DocPath) {
    const graph = await this.admin.graph(focus ? { scope: 'local', centerId: adminPath(focus), depth: 1 } : { scope: 'global' })
    const nodes = graph.nodes.map(value => { const node = value as Record<string, unknown>; return {
      path: String(node.path ?? node.id ?? ''), label: String(node.label ?? node.title ?? ''), title: String(node.title ?? ''),
      inbound: Number(node.inbound ?? 0), outbound: Number(node.outbound ?? 0) } })
    const idToPath = new Map(graph.nodes.map(value => { const node = value as Record<string, unknown>; return [String(node.id ?? node.path ?? ''), String(node.path ?? node.id ?? '')] }))
    return { nodes,
      edges: graph.edges.map(value => { const edge = value as Record<string, unknown>
        const reference = Array.isArray(edge.references) ? edge.references[0] as Record<string, unknown> | undefined : undefined
        const lines = Array.isArray(edge.lines) ? edge.lines : []
        return {
        source: idToPath.get(String(edge.source ?? edge.source_id ?? '')) ?? String(edge.source ?? edge.source_id ?? ''),
        target: (edge.target ?? edge.target_id) == null ? null : idToPath.get(String(edge.target ?? edge.target_id)) ?? String(edge.target ?? edge.target_id),
        raw_target: String(edge.raw_target ?? reference?.raw_target ?? ''), line: Number(edge.line ?? edge.line_number ?? lines[0] ?? reference?.line ?? 0) } }),
      unresolved: (graph.unresolved ?? []).map(value => typeof value === 'string' ? value : String((value as Record<string, unknown>).raw_target ?? '')) }
  }
  async status(): Promise<VaultStatus> {
    const status = await this.admin.status(); const count = Number(status.document_count ?? 0)
    return { raw_documents: Number(status.raw_documents ?? status.raw_document_count ?? status.sources_count ?? 0), wiki_pages: Number(status.wiki_pages ?? status.wiki_page_count ?? status.wiki_count ?? count),
      graph_links: Number(status.graph_links ?? 0), resolved_links: Number(status.resolved_links ?? 0), unresolved_links: Number(status.unresolved_links ?? 0) }
  }
  async apply(change: VaultChange): Promise<Applied> {
    const path = 'path' in change ? adminPath(change.path) : undefined
    let result: unknown
    switch (change.type) {
      case 'write': result = await this.admin.apply({ type: 'write', path: path!, body: change.content,
        expect: change.expect == null ? undefined : change.path.scope === 'raw' ? change.expect : Number(change.expect) }); break
      case 'patch': {
        const current = await this.admin.read(path!, change.path.scope)
        const currentRevision = 'revision' in current ? current.revision : current.sha256
        const expect = change.expect ?? String(currentRevision)
        result = await this.admin.apply({ type: 'write', path: path!, body: await editContent('body' in current ? current.body : '', change.edits),
          expect: change.path.scope === 'raw' ? expect : Number(expect) }); break
      }
      case 'move': result = await this.admin.apply({ type: 'move', from: adminPath(change.from), to: adminPath(change.to) }); break
      case 'delete': result = await this.admin.apply({ type: 'delete', path: path! }); break
      case 'import_raw': {
        const grouped = new Map<string, Array<{ path: string; body: string }>>()
        for (const file of change.files) {
          const parts = file.path.relative.split('/')
          const fileName = parts.pop() ?? ''
          const folder = ['raw', ...parts].join('/')
          const files = grouped.get(folder) ?? []
          files.push({ path: fileName, body: file.content })
          grouped.set(folder, files)
        }
        const results = await Promise.all([...grouped].map(([folder, files]) => this.admin.apply({ type: 'import_raw', folder, files })))
        const paths = results.flatMap(value => {
          if (!value || typeof value !== 'object') return []
          const created = (value as { created?: unknown }).created
          return Array.isArray(created) ? created.filter((path): path is string => typeof path === 'string') : []
        }).map(path => path.replace(/^raw\//, 'sources/'))
        return { paths, revision: null }
      }
      case 'ingest_source': result = await this.admin.apply({ type: 'ingest_source', path: adminPath(change.source) }); break
      case 'reindex': result = await this.admin.apply({ type: 'reindex' }); break
    }
    return applied(result, path)
  }
  onChange(listener: (event: VaultEvent) => void) { return this.admin.onChange(event => listener(translateEvent(event))) }
}

export class HttpKnowledgeBackend extends VaultKnowledgeBackend {
  readonly capabilities = { chat: true } as const
  constructor(request: RequestJson = requestJson, pollIntervalMs = 2_000) { super(new HttpAdminKnowledgeBackend(request, pollIntervalMs)) }
  protected override adminReadPath(path: DocPath) {
    const vaultPath = adminPath(path)
    return path.scope === 'wiki' ? this.documentIdsByVaultPath.get(vaultPath) ?? vaultPath : vaultPath
  }
}

export class MemoryKnowledgeBackend extends VaultKnowledgeBackend {
  readonly capabilities = { chat: false } as const
  constructor(seed: MemoryKnowledgeSeed = {}) { super(new MemoryAdminKnowledgeBackend(seed)) }
}

export function useAdminKnowledgeBackend() {
  const backend = useKnowledgeBackend()
  const admin = (backend as KnowledgeBackend & Partial<AdminKnowledgeExtension>).admin
  if (!admin) throw new Error('The active KnowledgeBackend does not support Admin Web operations')
  return admin
}

export function useOptionalAdminKnowledgeBackend() {
  const backend = useKnowledgeBackend()
  return (backend as KnowledgeBackend & Partial<AdminKnowledgeExtension>).admin ?? null
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
