import { apiUrl } from './client'
import { consumeSse, createSseParserState } from './sse'
import { readAdminToken } from '@/stores/auth'

export interface WikiChatInputMessage {
  role: 'user' | 'assistant'
  content: string
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

export interface WikiChatHandlers {
  onTool: (tool: WikiChatTool) => void
  onGraph: (graph: AnswerGraph) => void
  onDelta: (text: string) => void
  onDone: (result: { citations: string[]; stop_reason: string }) => void
  onError: (code: string) => void
}

export class WikiChatRequestError extends Error {
  constructor(readonly code: string) {
    super(code)
    this.name = 'WikiChatRequestError'
  }
}

function eventObject(data: string): Record<string, unknown> {
  let parsed: unknown
  try {
    parsed = JSON.parse(data)
  } catch {
    throw new WikiChatRequestError('wiki_chat_invalid_event')
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new WikiChatRequestError('wiki_chat_invalid_event')
  }
  return parsed as Record<string, unknown>
}

function apiErrorCode(body: unknown, fallback: string): string {
  if (!body || typeof body !== 'object') return fallback
  const error = (body as { error?: unknown }).error
  if (error && typeof error === 'object' && typeof (error as { code?: unknown }).code === 'string') {
    return (error as { code: string }).code
  }
  return fallback
}

/** POST one chat turn and consume its event stream. A failed POST is never retried automatically. */
export async function askWiki(
  messages: WikiChatInputMessage[],
  handlers: WikiChatHandlers,
  signal: AbortSignal,
): Promise<void> {
  const token = readAdminToken()
  const response = await fetch(apiUrl('/api/admin/knowledge/ask'), {
    method: 'POST',
    headers: {
      Accept: 'text/event-stream',
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    cache: 'no-store',
    signal,
    body: JSON.stringify({ messages }),
  })
  if (!response.ok) {
    let body: unknown
    try { body = await response.json() } catch { /* keep status fallback */ }
    throw new WikiChatRequestError(apiErrorCode(body, `http_${response.status}`))
  }
  if (!response.body || !response.headers.get('content-type')?.startsWith('text/event-stream')) {
    throw new WikiChatRequestError('wiki_chat_stream_unavailable')
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  const state = createSseParserState()
  let completed = false
  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read()
      if (done) break
      const frames = consumeSse(decoder.decode(value, { stream: true }), state, 512 * 1024)
      for (const frame of frames) {
        if (signal.aborted) return
        const payload = eventObject(frame.data)
        if (frame.event === 'tool' && typeof payload.call_id === 'string' && typeof payload.name === 'string') {
          if (payload.state === 'running' || payload.state === 'completed' || payload.state === 'failed') {
            const pages = Array.isArray(payload.pages)
              ? [...new Set(payload.pages.filter((path): path is string =>
                typeof path === 'string' && path.length <= 512 &&
                (path.startsWith('wiki/') || path.startsWith('sources/')) &&
                !path.includes('\\') && !path.split('/').some(segment => segment === '..' || segment === ''),
              ))].slice(0, 20)
              : []
            handlers.onTool({
              call_id: payload.call_id,
              name: payload.name,
              state: payload.state,
              ...(typeof payload.query === 'string' ? { query: payload.query } : {}),
              pages,
            })
          }
        } else if (frame.event === 'graph') {
          handlers.onGraph(payload as unknown as AnswerGraph)
        } else if (frame.event === 'delta' && typeof payload.text === 'string') {
          handlers.onDelta(payload.text)
        } else if (frame.event === 'done') {
          const citations = Array.isArray(payload.citations)
            ? payload.citations.filter((citation): citation is string => typeof citation === 'string')
            : []
          handlers.onDone({
            citations,
            stop_reason: typeof payload.stop_reason === 'string' ? payload.stop_reason : 'finished',
          })
          completed = true
        } else if (frame.event === 'error') {
          handlers.onError(typeof payload.code === 'string' ? payload.code : 'provider_failed')
        }
      }
    }
  } finally {
    if (signal.aborted) void reader.cancel().catch(() => undefined)
    reader.releaseLock()
  }
  if (!signal.aborted && !completed) throw new WikiChatRequestError('wiki_chat_connection_lost')
}
