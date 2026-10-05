import type { DbId, Page, PageQuery } from './common'

export type HistoryRole = 'user' | 'assistant'

export interface HistoryEntry {
  id: DbId
  session_id: string
  device_id: DbId | null
  agent_id: DbId | null
  template_id: DbId | null
  role: HistoryRole
  sequence: number
  created_at: string
  [field: string]: unknown
}

export interface HistoryListQuery extends PageQuery {
  session_id?: string
  /** Internal database ID, not the Device's external `device_id`. */
  device_id?: DbId
  /** Internal database ID, not the Agent public key. */
  agent_id?: DbId
  /** Internal database ID, not the Template public key. */
  template_id?: DbId
  role?: HistoryRole
  sort?: 'created_at' | '-created_at' | 'sequence' | '-sequence'
}

export type HistoryPurgeInput =
  | { session_id: string }
  | { device_id: DbId }
  | { all: 'all'; confirm: 'PURGE_ALL_HISTORY' }

export type HistoryPage = Page<HistoryEntry>
