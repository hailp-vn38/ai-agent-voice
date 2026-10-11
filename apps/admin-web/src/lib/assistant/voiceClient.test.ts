import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WebAssistantClient } from './voiceClient'
import { parseServerEvent } from './protocol'

vi.mock('@/api/assistant', () => ({ assistantApi: {
  settings: vi.fn(async () => ({ configured: true, enabled: true, device_id: 'managed', agent_key: 'studio', template_key: null, revision: 1, active_connections: 0 })),
  ticket: vi.fn(async () => ({ device_id: 'managed', client_id: 'browser', ticket: 'secret', expires_in_seconds: 30 })),
} }))
import { assistantApi } from '@/api/assistant'

class Socket {
  static OPEN = 1
  static sockets: Socket[] = []
  readyState = 1
  bufferedAmount = 0
  binaryType = ''
  sent: string[] = []
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string | ArrayBuffer }) => void) | null = null
  onerror: (() => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null
  constructor(readonly url: URL) { Socket.sockets.push(this) }
  send(text: string) { this.sent.push(text) }
  close() { this.readyState = 3 }
  event(event: object) { this.onmessage?.({ data: JSON.stringify({ session_id: 'session-1', ...event }) }) }
}
const hello = { type: 'hello', transport: 'websocket', audio_params: { format: 'opus', sample_rate: 24000, channels: 1, frame_duration: 60 } }
let client: WebAssistantClient
let states: string[]
let errors: string[]
let events: object[]
async function open() {
  const connected = client.connect()
  await vi.waitFor(() => expect(Socket.sockets).toHaveLength(1))
  const socket = Socket.sockets[0]!
  socket.onopen?.()
  expect(states.at(-1)).toBe('handshaking')
  socket.event(hello)
  await connected
  return socket
}
beforeEach(() => {
  Socket.sockets = []
  states = []; errors = []; events = []
  vi.stubGlobal('WebSocket', Socket)
  client = new WebAssistantClient({ state: s => states.push(s), error: e => errors.push(e), event: e => events.push(e), settings: () => {} })
})
afterEach(() => { client.disconnect(); vi.unstubAllGlobals(); vi.clearAllMocks() })

describe('shared Voice WebSocket client', () => {
  it('waits for ServerHello and listening_ready before detect, then waits for terminal status', async () => {
    const socket = await open()
    const sending = client.sendText('Xin chào')
    expect(JSON.parse(socket.sent.at(-1)!)).toMatchObject({ type: 'listen', state: 'start', mode: 'manual', session_id: 'session-1' })
    expect(socket.sent.some(s => JSON.parse(s).state === 'detect')).toBe(false)
    await expect(client.sendText('duplicate')).rejects.toThrow('turn_busy')
    socket.event({ type: 'turn_status', state: 'listening_ready' })
    await sending
    expect(JSON.parse(socket.sent.at(-1)!)).toMatchObject({ state: 'detect', text: 'Xin chào' })
    socket.event({ type: 'stt', text: 'Xin chào' })
    socket.event({ type: 'llm', text: 'Chào bạn.' })
    socket.event({ type: 'tts', state: 'start' })
    socket.onmessage?.({ data: new Uint8Array([24, 0]).buffer }) // muted still drains binary
    socket.event({ type: 'tts', state: 'stop' })
    expect(states.at(-1)).toBe('speaking')
    socket.event({ type: 'turn_status', state: 'completed', turn_id: 1 })
    expect(states.at(-1)).toBe('ready')
    expect(events).toHaveLength(6)
  })
  it('never sends detect when manual listening is rejected', async () => {
    const socket = await open()
    const sending = client.sendText('hello')
    socket.event({ type: 'turn_status', state: 'rejected', reason: 'asr_capacity' })
    await expect(sending).rejects.toThrow('asr_capacity')
    expect(socket.sent.some(s => JSON.parse(s).state === 'detect')).toBe(false)
    expect(states.at(-1)).toBe('ready')
  })
  it('disconnect invalidates old callbacks and reconnect requests a fresh ticket without replay', async () => {
    const socket = await open()
    const oldMessage = socket.onmessage
    client.disconnect()
    Socket.sockets = []
    await open()
    oldMessage?.({ data: JSON.stringify({ type: 'llm', session_id: 'session-1', text: 'stale' }) })
    expect(events).toHaveLength(0)
    expect(assistantApi.ticket).toHaveBeenCalledTimes(2)
    expect(Socket.sockets[0]!.sent).toHaveLength(1)
  })
  it('rejects noncanonical hello and never marks connected on transport open', async () => {
    const connected = client.connect()
    await vi.waitFor(() => expect(Socket.sockets).toHaveLength(1))
    const socket = Socket.sockets[0]!
    socket.onopen?.()
    expect(states).not.toContain('ready')
    socket.event({ ...hello, audio_params: { format: 'opus', sample_rate: 16000, channels: 1, frame_duration: 60 } })
    await connected
    expect(states.at(-1)).toBe('error')
    expect(errors).toContain('protocol_error')
  })
  it('discards in-flight audio and tts:start after abort until terminal cancellation', async () => {
    const socket = await open()
    const sending = client.sendText('hello')
    socket.event({ type: 'turn_status', state: 'listening_ready' })
    await sending
    socket.event({ type: 'tts', state: 'start' })
    client.interrupt()
    socket.event({ type: 'tts', state: 'start' })
    socket.onmessage?.({ data: new Uint8Array([24, 0]).buffer })
    expect(states.at(-1)).toBe('aborting')
    expect(socket.readyState).toBe(Socket.OPEN)
    socket.event({ type: 'turn_status', state: 'cancelled' })
    expect(states.at(-1)).toBe('ready')
    expect(errors).toHaveLength(0)
  })
  it('a cancelled microphone permission request cannot abort the next turn', async () => {
    const socket = await open()
    let rejectCapture!: (error: Error) => void
    vi.spyOn(client.audio, 'startMicrophone').mockImplementation(() => new Promise((_resolve, reject) => { rejectCapture = reject }))
    const capture = client.startMicrophone()
    socket.event({ type: 'turn_status', state: 'listening_ready' })
    await vi.waitFor(() => expect(rejectCapture).toBeDefined())
    client.interrupt()
    socket.event({ type: 'turn_status', state: 'cancelled' })
    const sending = client.sendText('next')
    socket.event({ type: 'turn_status', state: 'listening_ready' })
    await sending
    rejectCapture(new Error('capture_cancelled'))
    await expect(capture).rejects.toThrow('capture_cancelled')
    expect(states.at(-1)).toBe('processing')
    expect(socket.sent.filter(s => JSON.parse(s).type === 'abort')).toHaveLength(1)
  })
  it('ignores late listening ACK while aborting and accepts terminal cancellation', async () => {
    const socket = await open()
    const sending = client.sendText('hello')
    client.interrupt()
    await expect(sending).rejects.toThrow('cancelled')
    socket.event({ type: 'turn_status', state: 'listening_ready' })
    socket.event({ type: 'turn_status', state: 'cancelled' })
    expect(states.at(-1)).toBe('ready')
    expect(errors).not.toContain('protocol_error')
  })
  it('streams Opus frames across multiple Auto VAD turns without another microphone click', async () => {
    const socket = await open()
    let emitFrame!: (frame: ArrayBuffer) => void
    vi.spyOn(client.audio, 'startMicrophone').mockImplementation(async send => { emitFrame = send })
    vi.spyOn(client.audio, 'stopMicrophone').mockResolvedValue(undefined)
    await client.startMicrophone()
    expect(JSON.parse(socket.sent.at(-1)!)).toMatchObject({ type: 'listen', state: 'start', mode: 'auto' })
    expect(states.at(-1)).toBe('listening')

    const frame = new Uint8Array([24, 0]).buffer
    emitFrame(frame)
    expect(socket.sent.at(-1)).toBe(frame)

    socket.event({ type: 'stt', text: 'First question' })
    expect(states.at(-1)).toBe('processing')
    socket.event({ type: 'tts', state: 'start' })
    expect(states.at(-1)).toBe('speaking')
    emitFrame(frame)
    expect(socket.sent.at(-1)).toBe(frame)
    socket.event({ type: 'tts', state: 'stop' })
    socket.event({ type: 'turn_status', state: 'completed', turn_id: 1 })
    expect(states.at(-1)).toBe('listening')

    // The second speech segment reuses the same WebSocket and capture source.
    emitFrame(frame)
    expect(socket.sent.at(-1)).toBe(frame)
    socket.event({ type: 'stt', text: 'Second question' })
    expect(states.at(-1)).toBe('processing')
    socket.event({ type: 'llm', text: 'Second answer' })
    socket.event({ type: 'turn_status', state: 'completed', turn_id: 2 })
    expect(states.at(-1)).toBe('listening')
    expect(Socket.sockets).toHaveLength(1)
  })

  it('turns Auto microphone off during TTS without aborting playback or disconnecting', async () => {
    const socket = await open()
    let emitFrame!: (frame: ArrayBuffer) => void
    vi.spyOn(client.audio, 'startMicrophone').mockImplementation(async send => { emitFrame = send })
    vi.spyOn(client.audio, 'stopMicrophone').mockResolvedValue(undefined)
    await client.startMicrophone()
    socket.event({ type: 'stt', text: 'Hello' })
    socket.event({ type: 'tts', state: 'start' })
    await client.stopMicrophone()
    expect(JSON.parse(socket.sent.at(-1)!)).toMatchObject({ type: 'listen', state: 'stop' })
    expect(states.at(-1)).toBe('speaking')
    const count = socket.sent.length
    emitFrame(new Uint8Array([24, 0]).buffer)
    expect(socket.sent).toHaveLength(count)
    socket.event({ type: 'turn_status', state: 'completed', turn_id: 1 })
    expect(states.at(-1)).toBe('ready')
    expect(socket.readyState).toBe(Socket.OPEN)
    expect(socket.sent.some(s => typeof s === 'string' && s.includes('"type":"abort"'))).toBe(false)
  })

  it('cancels microphone permission safely if the user switches capture off before it resolves', async () => {
    const socket = await open()
    let rejectCapture!: (error: Error) => void
    vi.spyOn(client.audio, 'startMicrophone').mockImplementation(() => new Promise((_resolve, reject) => { rejectCapture = reject }))
    vi.spyOn(client.audio, 'stopMicrophone').mockResolvedValue(undefined)
    const start = client.startMicrophone()
    await vi.waitFor(() => expect(rejectCapture).toBeDefined())
    await client.stopMicrophone()
    rejectCapture(new Error('capture_cancelled'))
    await expect(start).resolves.toBeUndefined()
    expect(states.at(-1)).toBe('ready')
    expect(JSON.parse(socket.sent.at(-1)!)).toMatchObject({ type: 'listen', state: 'stop' })
    expect(errors).toHaveLength(0)
  })

})

it('validates session and typed events without exposing raw frames', () => {
  expect(() => parseServerEvent('{"type":"stt","text":"x","session_id":"other"}', 'session-1')).toThrow('invalid_session')
  expect(() => parseServerEvent('{"type":"llm","text":4,"session_id":"session-1"}')).toThrow('invalid_frame')
  expect(() => parseServerEvent('[]')).toThrow('invalid_frame')
  expect(() => parseServerEvent('x'.repeat(65537))).toThrow('invalid_frame')
})
