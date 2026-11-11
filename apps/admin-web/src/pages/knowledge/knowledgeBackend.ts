import { inject, type InjectionKey } from 'vue'
import { jsonRequest, requestJson, withQuery } from '@/api/client'
import { listenKnowledgeChanges, type ListenKnowledgeChanges } from '@/api/knowledge-updates'
import { adminPath, VaultKnowledgeBackend, KNOWLEDGE_BACKEND_KEY as SHARED_KNOWLEDGE_BACKEND_KEY } from '@vela/knowledge-ui'
import type {
  AdminKnowledgeBackend, Change, DocPath, KnowledgeChangeEvent, KnowledgeGraph,
  KnowledgeGraphOptions, KnowledgeHit, KnowledgePage, KnowledgeRawFile,
  KnowledgeSearchOptions, KnowledgeSpace, KnowledgeStatus, KnowledgeTreeNode,
  Scope, SyncBackend, SyncConfiguration, SyncReport, SyncStatus,
} from '@vela/knowledge-ui'

export * from '@vela/knowledge-ui'

type HttpRequestJson = <T>(path: string, init?: RequestInit) => Promise<T>

class HttpAdminKnowledgeBackend implements AdminKnowledgeBackend {
  readonly capabilities = { chat: true } as const
  readonly sync: SyncBackend
  private readonly lastTrees = new Map<KnowledgeSpace, KnowledgeTreeNode[]>()
  private readonly changeListeners = new Set<(event: KnowledgeChangeEvent) => void>()
  private readonly syncListeners = new Set<(status: SyncStatus) => void>()
  private streamAbort?: AbortController
  constructor(
    private readonly request: HttpRequestJson = requestJson,
    private readonly listen: ListenKnowledgeChanges = listenKnowledgeChanges,
  ) { this.sync = new HttpSyncBackend(request, this) }

  addSyncListener(listener: (status: SyncStatus) => void) {
    this.syncListeners.add(listener)
    this.startEvents()
    return () => { this.syncListeners.delete(listener); this.stopEventsIfUnused() }
  }

  private startEvents() {
    if (this.streamAbort) return
    const abort = new AbortController()
    this.streamAbort = abort
    void this.listen({
      onChanged: paths => this.changeListeners.forEach(listener => listener({ type: 'external', origin: 'server', paths })),
      onResync: () => this.changeListeners.forEach(listener => listener({ type: 'external', origin: 'server', paths: [] })),
      onSyncStatus: status => this.syncListeners.forEach(listener => listener(status)),
    }, abort.signal)
  }

  private stopEventsIfUnused() {
    if (this.changeListeners.size || this.syncListeners.size) return
    this.streamAbort?.abort()
    this.streamAbort = undefined
  }

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

  /** The server pushes vault changes over SSE; every change reloads from REST. */
  onChange(listener: (event: KnowledgeChangeEvent) => void) {
    this.changeListeners.add(listener)
    this.startEvents()
    return () => { this.changeListeners.delete(listener); this.stopEventsIfUnused() }
  }
}

class HttpSyncBackend implements SyncBackend {
  readonly configurable = false
  constructor(
    private readonly request: HttpRequestJson,
    private readonly backend: HttpAdminKnowledgeBackend,
  ) {}

  async status(): Promise<SyncStatus> { return (await this.request<{ status: SyncStatus }>('/api/admin/knowledge/sync')).status }
  async configuration(): Promise<SyncConfiguration> {
    const { configuration } = await this.request<{ configuration: SyncConfiguration }>('/api/admin/knowledge/sync')
    return configuration
  }
  syncNow(): Promise<SyncReport> { return this.request('/api/admin/knowledge/sync', jsonRequest('POST', {})) }
  onStatus(listener: (status: SyncStatus) => void) { return this.backend.addSyncListener(listener) }
}


export class HttpKnowledgeBackend extends VaultKnowledgeBackend {
  readonly capabilities = { chat: true } as const
  constructor(request: HttpRequestJson = requestJson, listen: ListenKnowledgeChanges = listenKnowledgeChanges) {
    super(new HttpAdminKnowledgeBackend(request, listen))
  }
  protected override adminReadPath(path: DocPath) {
    const vaultPath = adminPath(path)
    return path.scope === 'wiki' ? this.documentIdsByVaultPath.get(vaultPath) ?? vaultPath : vaultPath
  }
}


export const KNOWLEDGE_BACKEND_KEY = SHARED_KNOWLEDGE_BACKEND_KEY as InjectionKey<import('@vela/knowledge-ui').KnowledgeBackend>
const defaultBackend = new HttpKnowledgeBackend()
export function useKnowledgeBackend() { return inject(KNOWLEDGE_BACKEND_KEY, defaultBackend) }
export const knowledgeBackend = new HttpKnowledgeBackend()
