import { afterEach, describe, expect, it, vi } from 'vitest'

import { ApiError } from './errors'
import { listenSystemLogStream, systemLogsApi } from './system-logs'
import type {
  LogStreamLaggedEvent,
  LogStreamReadyEvent,
  StructuredLogEntry,
  SystemLogPage,
} from './types/system-logs'

describe('systemLogsApi & listenSystemLogStream', () => {
  afterEach(() => {
    sessionStorage.clear()
    vi.unstubAllGlobals()
  })

  it('requests filtered system logs and audit logs with query parameters matching backend deny_unknown_fields', async () => {
    const samplePage: SystemLogPage = {
      items: [
        {
          seq: 12,
          timestamp_ms: 1793000000000,
          level: 'INFO',
          subsystem: 'voice.llm',
          event: 'llm_stream_completed',
          message: 'LLM stream completed',
          target: 'voice_agent_server::workers::llm',
          session_id: 'sess-99',
          turn_id: 2,
          generation: 1,
          duration_ms: 480,
          fields: { ttft_ms: 180 },
        },
      ],
      oldest_seq: 1,
      latest_seq: 12,
      buffered_count: 12,
      capacity: 4000,
      dropped_evicted: 0,
      counts: { error: 0, warn: 1, info: 11, debug: 0, trace: 0 },
    }
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(samplePage), { status: 200 }))
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            items: [
              {
                id: 1,
                created_at: 1793000000,
                request_id: 'req-audit-1',
                resource_type: 'provider',
                resource_id: 1,
                action: 'update',
                prior_revision: 2,
                new_revision: 3,
                outcome: 'success',
                error_kind: null,
                affected_rows: 1,
              },
            ],
            total: 1,
            page: 1,
            page_size: 20,
          }),
          { status: 200 },
        ),
      )
    vi.stubGlobal('fetch', fetchMock)

    const logs = await systemLogsApi.list({
      min_level: 'INFO',
      subsystem: 'voice',
      session_id: 'sess-99',
      limit: 50,
    })
    expect(logs.items).toHaveLength(1)
    expect(logs.items[0]?.session_id).toBe('sess-99')
    expect(fetchMock.mock.calls[0]?.[0]).toContain(
      '/api/admin/system/logs?min_level=info&subsystem=voice&session_id=sess-99&limit=50',
    )

    const audit = await systemLogsApi.auditLogs({
      resource_type: 'provider',
      outcome: 'success',
      page: 1,
      page_size: 20,
    })
    expect(audit.total).toBe(1)
    expect(audit.items[0]?.created_at).toBe(1793000000)
    expect(fetchMock.mock.calls[1]?.[0]).toContain(
      '/api/admin/system/audit-logs?page=1&page_size=20&resource_type=provider&outcome=success',
    )
  })

  it('parses chunked SSE ready, log_entry, and lagged frames with bearer token', async () => {
    sessionStorage.setItem('voice-agent-admin-token', 'secret-admin-jwt')

    const encoder = new TextEncoder()
    const chunks = [
      'event: ready\ndata: {"latest_seq":10,"capacity":4000}\n\n',
      'event: log_entry\ndata: {"seq":11,"timestamp_ms":1793000001000,"level":"WARN",',
      '"subsystem":"voice.tts","event":"tts_synthesis_timed_out","message":"TTS synthesis timed out","target":"tts","session_id":"sess-42","turn_id":3,"fields":{"segment_id":2}}\r\n\r\n',
      'event: lagged\ndata: {"missed":18,"latest_seq":30}\n\n',
    ]

    const controller = new AbortController()
    const stream = new ReadableStream<Uint8Array>({
      start(streamController) {
        for (const chunk of chunks) {
          streamController.enqueue(encoder.encode(chunk))
        }
        streamController.close()
      },
    })

    const fetchMock = vi.fn().mockResolvedValue(
      new Response(stream, {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)

    const readyEvents: LogStreamReadyEvent[] = []
    const entries: StructuredLogEntry[] = []
    const laggedEvents: LogStreamLaggedEvent[] = []

    await listenSystemLogStream(
      { min_level: 'WARN', subsystem: 'voice' },
      {
        onReady: (ev) => readyEvents.push(ev),
        onEntry: (entry) => entries.push(entry),
        onLagged: (ev) => laggedEvents.push(ev),
      },
      controller.signal,
    )

    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toContain('/api/admin/system/logs/stream?min_level=warn&subsystem=voice')
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer secret-admin-jwt')

    expect(readyEvents).toEqual([{ latest_seq: 10, capacity: 4000 }])
    expect(entries).toHaveLength(1)
    expect(entries[0]?.seq).toBe(11)
    expect(entries[0]?.event).toBe('tts_synthesis_timed_out')
    expect(entries[0]?.session_id).toBe('sess-42')
    expect(laggedEvents).toEqual([{ missed: 18, latest_seq: 30 }])
  })

  it('stops SSE stream and reports ApiError on 401 unauthorized instead of demo fallback', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('unauthorized', { status: 401 })))

    const errors: unknown[] = []
    const readyEvents: LogStreamReadyEvent[] = []
    const controller = new AbortController()

    await listenSystemLogStream(
      { min_level: 'INFO' },
      {
        onReady: (ev) => readyEvents.push(ev),
        onEntry: () => {},
        onError: (err) => errors.push(err),
      },
      controller.signal,
    )

    expect(readyEvents).toHaveLength(0)
    expect(errors).toHaveLength(1)
    expect(errors[0]).toBeInstanceOf(ApiError)
    expect((errors[0] as ApiError).status).toBe(401)
  })

  it('falls back to stateful demo store and simulates a 7-step voice turn trace when backend returns 404', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('not found', { status: 404 })))

    const initial = await systemLogsApi.list({ min_level: 'INFO' })
    expect(initial.demo_mode).toBe(true)
    expect(initial.items.length).toBeGreaterThan(0)

    const simulated = await systemLogsApi.simulateDemoTurnLogs()
    expect(simulated).toHaveLength(7)
    const sessionId = simulated[0]?.session_id
    expect(sessionId).toBeTruthy()

    const bySession = await systemLogsApi.list({ session_id: sessionId ?? undefined })
    expect(bySession.items).toHaveLength(7)
  })
})
