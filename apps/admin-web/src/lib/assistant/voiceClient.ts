import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { apiUrl } from '@/api/client'
import { isApiError } from '@/api/errors'
import { AssistantAudio, isCanonicalOpusPacket } from './audio'
import { clientHello, parseServerEvent, type ServerEvent } from './protocol'

export type AssistantState = 'disconnected' | 'unconfigured' | 'disabled' | 'auth_error' | 'connecting' | 'handshaking' | 'ready' | 'listening' | 'processing' | 'speaking' | 'aborting' | 'reconnecting' | 'uncertain' | 'error'
interface Callbacks {
  state: (state: AssistantState) => void
  event: (event: ServerEvent) => void
  error: (code: string) => void
  settings: (settings: AssistantSettings) => void
  level?: (value: number) => void
}
interface Waiter { resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }

export class WebAssistantClient {
  private socket?: WebSocket
  private session?: string
  private generation = 0
  private turnGeneration = 0
  private state: AssistantState = 'disconnected'
  private waiter?: Waiter
  private connecting?: Promise<void>
  private request?: AbortController
  private retry?: ReturnType<typeof setTimeout>
  private retries = 0
  private turnTimeout?: ReturnType<typeof setTimeout>
  readonly audio: AssistantAudio
  constructor(private readonly callbacks: Callbacks) {
    this.audio = new AssistantAudio(code => {
      this.callbacks.error(code)
      if (this.state === 'listening') this.interrupt()
    }, callbacks.level)
  }
  private setState(state: AssistantState) { this.state = state; this.callbacks.state(state) }
  private wait(milliseconds: number) {
    return new Promise<void>((resolve, reject) => {
      this.waiter = { resolve, reject, timer: setTimeout(() => this.settle(new Error('server_ack_timeout')), milliseconds) }
    })
  }
  private settle(error?: Error) {
    if (!this.waiter) return
    const waiter = this.waiter
    this.waiter = undefined
    clearTimeout(waiter.timer)
    if (error) waiter.reject(error); else waiter.resolve()
  }
  connect(): Promise<void> {
    if (this.connecting) return this.connecting
    if (this.session && this.socket?.readyState === WebSocket.OPEN) return Promise.resolve()
    this.cleanup()
    const generation = this.generation
    const controller = new AbortController()
    this.request = controller
    this.connecting = this.open(generation, controller).finally(() => {
      if (generation === this.generation) this.connecting = undefined
    })
    return this.connecting
  }
  private async open(generation: number, controller: AbortController) {
    this.setState('connecting')
    try {
      const settings = await assistantApi.settings(controller.signal)
      if (generation !== this.generation) return
      this.callbacks.settings(settings)
      if (!settings.configured || !settings.enabled) {
        this.setState(settings.configured ? 'disabled' : 'unconfigured')
        return
      }
      let lease = await assistantApi.ticket(controller.signal)
      if (generation !== this.generation) return
      const url = new URL(apiUrl('/voice/v1/'), window.location.origin)
      if (!['http:', 'https:'].includes(url.protocol)) throw new Error('invalid_api_origin')
      url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
      if (url.protocol === 'ws:' && !['localhost', '127.0.0.1', '[::1]'].includes(url.hostname) && window.location.protocol === 'https:') throw new Error('insecure_websocket')
      url.search = new URLSearchParams({ 'protocol-version': '1', 'device-id': lease.device_id, 'client-id': lease.client_id, ticket: lease.ticket }).toString()
      const socket = new WebSocket(url)
      // The socket owns the upgrade URL; no ticket is retained in UI state or errors.
      lease = { ...lease, ticket: '' }
      url.search = ''
      this.socket = socket
      socket.binaryType = 'arraybuffer'
      const handshake = this.wait(10000)
      socket.onopen = () => {
        if (generation !== this.generation) return
        this.setState('handshaking')
        socket.send(JSON.stringify(clientHello))
      }
      socket.onmessage = (event: MessageEvent) => {
        if (generation !== this.generation) return
        try {
          if (event.data instanceof ArrayBuffer) {
            if (!this.session || !['speaking', 'aborting'].includes(this.state)) throw new Error('unexpected_audio')
            if (this.state === 'aborting') {
              if (!isCanonicalOpusPacket(event.data)) throw new Error('invalid_audio_frame')
              return
            }
            this.audio.receive(event.data)
            return
          }
          if (typeof event.data !== 'string') throw new Error('invalid_frame')
          const message = parseServerEvent(event.data, this.session)
          if (!message) return
          if (message.type === 'hello') {
            if (this.state !== 'handshaking') throw new Error('invalid_hello')
            this.session = message.session_id
            this.retries = 0
            this.setState('ready')
            this.settle()
            return
          }
          if (!this.session) throw new Error('hello_required')
          this.receive(message)
        } catch { this.fail('protocol_error') }
      }
      socket.onerror = () => { if (generation === this.generation) this.callbacks.error('websocket_failed') }
      socket.onclose = event => {
        if (generation !== this.generation) return
        const active = !['ready', 'connecting', 'handshaking'].includes(this.state)
        this.callbacks.error(event.code === 1008 ? 'configuration_changed' : active ? 'delivery_unknown' : 'connection_closed')
        this.cleanup()
        this.setState(active ? 'uncertain' : 'disconnected')
        if (event.code !== 1008 && this.retries < 3) {
          const delay = 1000 * 2 ** this.retries++
          this.retry = setTimeout(() => { this.setState('reconnecting'); void this.connect().catch(() => {}) }, delay)
        }
      }
      await handshake
    } catch (error) {
      if (generation !== this.generation) return
      this.cleanup()
      const code = isApiError(error) ? error.code : error instanceof Error ? error.message : 'connection_failed'
      this.callbacks.error(code)
      this.setState(isApiError(error) && error.status === 401 ? 'auth_error' : 'error')
      throw new Error(code)
    }
  }
  private receive(event: ServerEvent) {
    if (event.type === 'turn_status') {
      if (event.state === 'listening_ready') {
        if (this.state === 'aborting') return
        if (this.state !== 'listening' || !this.waiter) throw new Error('unexpected_ack')
        this.settle()
      } else {
        this.settle(new Error(event.reason ?? event.state))
        clearTimeout(this.turnTimeout)
        this.audio.cancelMicrophone()
        if (event.state !== 'completed') this.audio.cancelPlayback()
        this.setState('ready')
      }
    } else if (event.type === 'tts') {
      if (this.state === 'aborting') return
      if (event.state === 'start') { this.audio.beginPlayback(); this.setState('speaking') }
      // Writer terminal status, not a timer or tts:stop, releases the next turn.
    }
    this.callbacks.event(event)
  }
  private send(command: Record<string, unknown>) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN || !this.session) throw new Error('not_connected')
    this.socket.send(JSON.stringify({ ...command, session_id: this.session }))
  }
  private async listen() {
    if (this.state !== 'ready') throw new Error('turn_busy')
    this.audio.cancelPlayback()
    const turn = ++this.turnGeneration
    const generation = this.generation
    this.setState('listening')
    const ready = this.wait(10000)
    try {
      this.send({ type: 'listen', state: 'start', mode: 'manual' })
      await ready
      if (generation !== this.generation || turn !== this.turnGeneration || !['listening'].includes(this.state)) throw new Error('cancelled')
    } catch (error) {
      if (generation === this.generation && turn === this.turnGeneration && this.session && this.state !== 'ready') this.interrupt()
      throw error
    }
  }
  private processing() {
    this.setState('processing')
    this.turnTimeout = setTimeout(() => { this.callbacks.error('delivery_unknown'); this.cleanup(); this.setState('uncertain') }, 120000)
  }
  async sendText(text: string) {
    if (!text.trim() || [...text.trim()].length > 4096) throw new Error('invalid_text')
    await this.listen()
    this.send({ type: 'listen', state: 'detect', text: text.trim() })
    this.processing()
  }
  async startMicrophone() {
    await this.listen()
    const generation = this.generation
    const turn = this.turnGeneration
    try {
      await this.audio.startMicrophone(packet => {
        if (generation !== this.generation || turn !== this.turnGeneration || this.state !== 'listening') return
        if (!this.socket || this.socket.bufferedAmount > 65536) { this.callbacks.error('audio_backpressure'); this.interrupt(); return }
        this.socket.send(packet)
      })
    } catch (error) {
      if (generation === this.generation && turn === this.turnGeneration) this.interrupt()
      throw error
    }
  }
  async stopMicrophone() {
    if (this.state !== 'listening') return
    const generation = this.generation
    const turn = this.turnGeneration
    await this.audio.stopMicrophone()
    if (generation !== this.generation || turn !== this.turnGeneration || this.state !== 'listening') return
    this.send({ type: 'listen', state: 'stop' })
    this.processing()
  }
  interrupt() {
    if (!this.session || !['listening', 'processing', 'speaking'].includes(this.state)) return
    this.audio.cancelMicrophone()
    this.audio.cancelPlayback()
    this.settle(new Error('cancelled'))
    this.send({ type: 'abort' })
    this.setState('aborting')
  }
  private fail(code: string) {
    this.callbacks.error(code)
    const active = ['listening', 'processing', 'speaking', 'aborting'].includes(this.state)
    this.cleanup()
    this.setState(active ? 'uncertain' : 'error')
  }
  private cleanup() {
    ++this.generation
    this.request?.abort()
    this.request = undefined
    clearTimeout(this.retry)
    clearTimeout(this.turnTimeout)
    this.settle(new Error('connection_closed'))
    this.audio.disconnect()
    if (this.socket) {
      this.socket.onopen = this.socket.onmessage = this.socket.onerror = this.socket.onclose = null
      this.socket.close()
    }
    this.socket = undefined
    this.session = undefined
    this.connecting = undefined
  }
  disconnect() { this.cleanup(); this.retries = 0; this.setState('disconnected') }
}
