export interface KnowledgeDraftState {
  wiki_id: string
  draft: null | {
    revision: number
    base_revision: number | null
    content_sha256: string
    owner: null | { job_id: string; operation_key: string }
  }
  resolution: null | { resolution_id: string; action: string; phase: string }
}
export interface KnowledgeCurrentState {
  drafts?: KnowledgeDraftState[]
  unavailable?: boolean
  code?: string
  source_ref?: string
}
export interface KnowledgeOutcome {
  wiki_id: string
  path: string
  accepted_revision: number | null
  draft_revision: number | null
  base_revision: number | null
  content_sha256: string
  index_health: string
}
export interface KnowledgeJobResult extends Record<string, unknown> {
  execution_version: 2
  source_ref: string
  source_coverage: { read_bytes: number; total_bytes: number; complete: boolean; missing_ranges: { start: number; end: number }[] }
  published_pages: KnowledgeOutcome[]
  remaining_drafts: KnowledgeOutcome[]
  blockers: { code: string; wiki_id?: string }[]
  outcome: 'published' | 'no_change' | 'incomplete'
  markdown: string
  index_health: string | null
}
