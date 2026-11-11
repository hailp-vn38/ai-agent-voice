import type { Component } from 'vue'

export interface WikiConversationSummary {
  id: string
  title: string
  updated_at_ms: number
  message_count: number
  running: boolean
}

export interface WikiChatTool {
  call_id: string
  name: string
  state: 'running' | 'completed' | 'failed'
  query?: string
  pages: string[]
}

export interface AnswerGraph {
  revision: number
  nodes: Array<{
    id: string
    path: string
    title: string
    status: string
    revision: number
    category?: string
    tags?: string[]
    tier?: 'cited' | 'read' | 'retrieved' | 'context'
    score?: number | null
  }>
  edges: Array<{
    source_id: string
    target_id: string
    kind: string
    lines: number[]
    references: Array<{ line: number; raw_target: string; alias?: string; anchor?: string }>
  }>
  unresolved: Array<{ source_id: string; source_line: number; raw_target: string; reason: string }>
  truncated: boolean
}

/** A vault file the user attached to the conversation; `path` is a `wiki/` or `sources/` path. */
export interface WikiChatFile {
  path: string
  title: string
}

export interface WikiChatMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  status: 'streaming' | 'done' | 'stopped' | 'failed' | 'interrupted'
  tools: WikiChatTool[]
  graph: AnswerGraph | null
  citations: string[]
  errorCode: string | null
  saved?: boolean
  contentTruncated?: boolean
  files?: WikiChatFile[]
}

export const MAX_PINNED_FILES = 4

export interface WikiChatPanelProps {
  messages: WikiChatMessage[]
  busy: boolean
  readOnly?: boolean
  loadFailed?: boolean
  drawerOpen?: boolean
  suggestions?: Array<{ label: string; prompt: string }>
  highlightedNode?: string | null
  pinnedFiles?: WikiChatFile[]
  openFile?: WikiChatFile | null
  loadingConversation?: boolean
  conversations?: WikiConversationSummary[]
  conversationId?: string | null
  loadingConversations?: boolean
  conversationListError?: string | null
  conversationHistory?: boolean
  allowRetry?: boolean
  confirmDialog?: Component
}

export type WikiChatPanelEvents = {
  send: [text: string]
  stop: []
  retry: [id: string]
  refreshConversation: []
  showGraph: [id: string]
  focusCitation: [path: string]
  highlightCitation: [path: string | null]
  pinFile: [file: WikiChatFile]
  unpinFile: [path: string]
  loadConversations: []
  selectConversation: [id: string]
  newConversation: []
  renameConversation: [id: string, title: string]
  deleteConversation: [id: string]
}
