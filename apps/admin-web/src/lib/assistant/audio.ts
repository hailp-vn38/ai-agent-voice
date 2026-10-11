// Raw Opus WebCodecs configuration follows https://www.w3.org/TR/webcodecs-opus-codec-registration/.
const encoderConfig: AudioEncoderConfig & { opus: { frameDuration: number; format: string } } = {
  codec: 'opus', sampleRate: 16000, numberOfChannels: 1, bitrate: 24000,
  opus: { frameDuration: 60000, format: 'opus' },
}
const decoderConfig: AudioDecoderConfig = { codec: 'opus', sampleRate: 24000, numberOfChannels: 1 }

export class AssistantAudio {
  muted = true
  private context?: AudioContext
  private capture?: AudioContext
  private stream?: MediaStream
  private node?: AudioWorkletNode
  private encoder?: AudioEncoder
  private decoder?: AudioDecoder
  private sources = new Set<AudioBufferSourceNode>()
  private nextPlayback = 0
  private timestamp = 0
  private generation = 0
  private playbackGeneration = 0
  constructor(private readonly fail: (code: string) => void, private readonly level: (value: number) => void = () => {}) {}

  static microphoneSupported() {
    return window.isSecureContext && !!navigator.mediaDevices?.getUserMedia && typeof AudioEncoder !== 'undefined' && typeof AudioWorkletNode !== 'undefined'
  }

  async enableSpeaker() {
    if (typeof AudioDecoder === 'undefined' || !(await AudioDecoder.isConfigSupported(decoderConfig)).supported) throw new Error('audio_unsupported')
    this.context ??= new AudioContext()
    await this.context.resume()
    this.muted = false
  }
  muteSpeaker() { this.muted = true; this.cancelPlayback() }
  beginPlayback() {
    this.cancelPlayback()
    if (this.muted || !this.context) return
    const generation = this.playbackGeneration
    this.decoder = new AudioDecoder({
      output: (data) => {
        try {
          if (generation !== this.playbackGeneration || this.muted || !this.context) return
          if (this.nextPlayback - this.context.currentTime > 2) { this.muteSpeaker(); this.fail('audio_backpressure'); return }
          const pcm = new Float32Array(data.numberOfFrames)
          data.copyTo(pcm, { planeIndex: 0, format: 'f32-planar' })
          const buffer = this.context.createBuffer(1, pcm.length, data.sampleRate)
          buffer.copyToChannel(pcm, 0)
          const source = this.context.createBufferSource()
          source.buffer = buffer
          source.connect(this.context.destination)
          this.sources.add(source)
          source.onended = () => { this.sources.delete(source); source.disconnect() }
          this.nextPlayback = Math.max(this.nextPlayback, this.context.currentTime + 0.02)
          source.start(this.nextPlayback)
          this.nextPlayback += buffer.duration
        } finally { data.close() }
      },
      error: () => { this.muteSpeaker(); this.fail('audio_decode_failed') },
    })
    this.decoder.configure(decoderConfig)
    this.timestamp = 0
  }
  receive(packet: ArrayBuffer) {
    if (!isCanonicalOpusPacket(packet)) throw new Error('invalid_audio_frame')
    if (this.muted) return
    if (!this.decoder || this.decoder.state !== 'configured') throw new Error('unexpected_audio')
    if (this.decoder.decodeQueueSize >= 32) { this.muteSpeaker(); this.fail('audio_backpressure'); return }
    this.decoder.decode(new EncodedAudioChunk({ type: 'key', timestamp: this.timestamp, duration: 60000, data: packet }))
    this.timestamp += 60000
  }
  cancelPlayback() {
    ++this.playbackGeneration
    if (this.decoder && this.decoder.state !== 'closed') this.decoder.close()
    this.decoder = undefined
    for (const source of this.sources) { source.onended = null; source.stop(); source.disconnect() }
    this.sources.clear()
    this.nextPlayback = 0
  }

  async startMicrophone(send: (packet: ArrayBuffer) => void) {
    const generation = ++this.generation
    if (document.hidden) throw new Error('capture_cancelled')
    if (!AssistantAudio.microphoneSupported() || !(await AudioEncoder.isConfigSupported(encoderConfig)).supported) throw new Error('microphone_unsupported')
    if (generation !== this.generation) throw new Error('capture_cancelled')
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } })
    if (generation !== this.generation || document.hidden) { stream.getTracks().forEach(t => t.stop()); throw new Error('capture_cancelled') }
    this.stream = stream
    try {
      this.capture = new AudioContext({ sampleRate: 16000 })
      if (this.capture.sampleRate !== 16000) throw new Error('microphone_sample_rate')
      await this.capture.audioWorklet.addModule(new URL('./capture-worklet.js', import.meta.url))
      if (generation !== this.generation) throw new Error('capture_cancelled')
      let timestamp = 0
      this.encoder = new AudioEncoder({
        output: chunk => {
          if (generation !== this.generation) return
          const packet = new ArrayBuffer(chunk.byteLength)
          chunk.copyTo(packet)
          if (!isCanonicalOpusPacket(packet)) { this.fail('opus_frame_duration'); this.cancelMicrophone(); return }
          send(packet)
        }, error: () => { if (generation === this.generation) { this.fail('audio_encode_failed'); this.cancelMicrophone() } },
      })
      this.encoder.configure(encoderConfig)
      this.node = new AudioWorkletNode(this.capture, 'assistant-capture')
      this.node.port.onmessage = (event: MessageEvent<Float32Array<ArrayBuffer>>) => {
        if (generation !== this.generation || this.encoder?.state !== 'configured') return
        if (this.encoder.encodeQueueSize >= 8) { this.fail('audio_backpressure'); this.cancelMicrophone(); return }
        this.level(Math.sqrt(event.data.reduce((sum, sample) => sum + sample * sample, 0) / 960))
        const frame = new AudioData({ format: 'f32', sampleRate: 16000, numberOfFrames: 960, numberOfChannels: 1, timestamp, data: event.data })
        timestamp += 60000
        try { this.encoder.encode(frame) } finally { frame.close() }
      }
      const source = this.capture.createMediaStreamSource(stream)
      source.connect(this.node)
      this.node.connect(this.capture.destination)
      for (const track of stream.getTracks()) track.onended = () => { if (generation === this.generation) { this.fail('microphone_ended'); this.cancelMicrophone() } }
      await this.capture.resume()
      if (generation !== this.generation) throw new Error('capture_cancelled')
    } catch (e) { if (generation === this.generation) this.cancelMicrophone(); throw e }
  }
  async stopMicrophone() {
    const generation = this.generation
    this.node?.disconnect()
    if (this.node) this.node.port.onmessage = null
    this.stream?.getTracks().forEach(t => { t.onended = null; t.stop() })
    try { if (this.encoder?.state === 'configured') await this.encoder.flush() } finally { if (generation === this.generation) this.cancelMicrophone() }
  }
  cancelMicrophone() {
    this.level(0)
    ++this.generation
    if (this.node) { this.node.port.onmessage = null; this.node.disconnect() }
    this.node = undefined
    this.stream?.getTracks().forEach(t => { t.onended = null; t.stop() })
    this.stream = undefined
    if (this.encoder && this.encoder.state !== 'closed') this.encoder.close()
    this.encoder = undefined
    void this.capture?.close().catch(() => {})
    this.capture = undefined
  }
  disconnect() {
    this.cancelMicrophone()
    this.cancelPlayback()
    void this.context?.close().catch(() => {})
    this.context = undefined
    this.muted = true
  }
}

// RFC 6716 section 3: packet TOC determines frame duration and packet frame count.
export function isCanonicalOpusPacket(packet: ArrayBuffer): boolean {
  if (!packet.byteLength || packet.byteLength > 4096) return false
  const bytes = new Uint8Array(packet)
  const toc = bytes[0]!
  const countCode = toc & 3
  const count = countCode === 0 ? 1 : countCode === 3 ? (bytes[1] ?? 0) & 63 : 2
  const duration = toc & 128 ? 2.5 * 2 ** ((toc >> 3) & 3) : (toc & 96) === 96 ? 10 * 2 ** ((toc >> 3) & 1) : [10, 20, 40, 60][(toc >> 3) & 3]!
  return count > 0 && duration * count === 60
}
