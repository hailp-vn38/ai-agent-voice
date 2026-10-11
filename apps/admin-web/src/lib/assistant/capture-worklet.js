// AudioContext performs the hardware resampling; emit exactly 60 ms of mono PCM at 16 kHz.
class AssistantCapture extends AudioWorkletProcessor {
  constructor() { super(); this.frame = new Float32Array(960); this.cursor = 0 }
  process(inputs) {
    const channels = inputs[0]
    if (!channels?.length) return true
    for (let i = 0; i < channels[0].length; i++) {
      let sample = 0
      for (const channel of channels) sample += channel[i]
      this.frame[this.cursor++] = sample / channels.length
      if (this.cursor === 960) {
        this.port.postMessage(this.frame, [this.frame.buffer])
        this.frame = new Float32Array(960)
        this.cursor = 0
      }
    }
    return true
  }
}
registerProcessor('assistant-capture', AssistantCapture)
