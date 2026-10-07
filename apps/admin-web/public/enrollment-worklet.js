// Forwards raw capture frames to the main thread. It only copies and transfers; downmix,
// resample, and encoding all happen in the testable JS module.
class EnrollmentCapture extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0]
    if (input && input.length > 0 && input[0].length > 0) {
      const channels = input.map((channel) => channel.slice())
      this.port.postMessage(channels, channels.map((channel) => channel.buffer))
    }
    return true
  }
}

registerProcessor('enrollment-capture', EnrollmentCapture)
