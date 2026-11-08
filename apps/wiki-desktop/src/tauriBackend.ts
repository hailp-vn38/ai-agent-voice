import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { KnowledgeBackend, DocPath, ReadWindow, Scope, VaultChange, VaultTreeNode, VaultPage, VaultHit, VaultGraph, VaultStatus, Applied, VaultEvent, VaultErrorKind } from '@vela/knowledge-ui'
import { VaultError } from '@vela/knowledge-ui'

type TypedError = { kind?: VaultErrorKind; message?: string; current?: string }

function normalizeError(error: unknown): never {
  const value = error && typeof error === 'object' ? error as TypedError : {}
  const kinds: VaultErrorKind[] = ['not_found', 'invalid_path', 'conflict', 'read_only', 'io', 'index']
  const kind = kinds.includes(value.kind as VaultErrorKind) ? value.kind as VaultErrorKind : 'io'
  throw new VaultError(kind, value.message ?? String(error), value.current)
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try { return await invoke<T>(command, args) } catch (error) { return normalizeError(error) }
}

export class TauriKnowledgeBackend implements KnowledgeBackend {
  readonly capabilities = { chat: false, write: true } as const
  async tree(scope: Scope): Promise<VaultTreeNode> { return call('vault_tree', { scope }) }
  async read(path: DocPath, window?: ReadWindow): Promise<VaultPage> { return call('vault_read', { path, window }) }
  async search(query: string): Promise<VaultHit[]> { return call('vault_search', { query }) }
  async graph(focus?: DocPath): Promise<VaultGraph> { return call('vault_graph', { focus }) }
  async status(): Promise<VaultStatus> { return call('vault_status') }
  async apply(change: VaultChange): Promise<Applied> { return call('vault_apply', { change }) }
  onChange(listener: (event: VaultEvent) => void): () => void {
    let unlisten: (() => void) | undefined
    void listen<VaultEvent>('vault://changed', event => listener(event.payload)).then(stop => { unlisten = stop })
    return () => unlisten?.()
  }
}

export const tauriKnowledgeBackend = new TauriKnowledgeBackend()
