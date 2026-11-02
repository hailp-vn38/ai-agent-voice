export interface SseFrame {
  event: string
  data: string
}

export interface SseParserState {
  buffer: string
  event: string
  data: string[]
  lastCr: boolean
}

export function createSseParserState(): SseParserState {
  return { buffer: '', event: '', data: [], lastCr: false }
}

/** Consume one text chunk and return complete SSE frames, preserving CRLF across chunks. */
export function consumeSse(
  chunk: string,
  state: SseParserState,
  maxFrameChars = 8192,
): SseFrame[] {
  let text = chunk
  if (state.lastCr && text.startsWith('\n')) text = text.slice(1)
  state.lastCr = text.endsWith('\r')
  state.buffer += text.replace(/\r\n?/g, '\n')

  const frames: SseFrame[] = []
  let newline: number
  while ((newline = state.buffer.indexOf('\n')) !== -1) {
    const line = state.buffer.slice(0, newline)
    state.buffer = state.buffer.slice(newline + 1)
    if (line === '') {
      if (state.event || state.data.length > 0) {
        frames.push({ event: state.event || 'message', data: state.data.join('\n') })
      }
      state.event = ''
      state.data = []
    } else if (!line.startsWith(':')) {
      const colon = line.indexOf(':')
      const field = colon < 0 ? line : line.slice(0, colon)
      const value = colon < 0 ? '' : line.slice(colon + 1).replace(/^ /, '')
      if (field === 'event') state.event = value
      else if (field === 'data') {
        state.data.push(value)
        if (state.data.reduce((sum, part) => sum + part.length, 0) > maxFrameChars) {
          throw new Error('sse_frame_too_large')
        }
      }
    }
  }
  if (state.buffer.length > maxFrameChars) throw new Error('sse_frame_too_large')
  return frames
}
