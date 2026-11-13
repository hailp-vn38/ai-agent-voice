import assert from 'node:assert/strict'
import { get as httpGet } from 'node:http'

const base = 'http://web:8080'
async function get(path, token) {
  return fetch(`${base}${path}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal: AbortSignal.timeout(10_000),
  })
}

function sseReader(body) {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  const state = { buffer: '', event: '', data: [], lastCr: false }

  async function nextSnapshot() {
    while (true) {
      const newline = state.buffer.indexOf('\n')
      if (newline >= 0) {
        const line = state.buffer.slice(0, newline)
        state.buffer = state.buffer.slice(newline + 1)
        if (line === '') {
          const event = state.event || 'message'
          const data = state.data.join('\n')
          state.event = ''
          state.data = []
          if (event === 'snapshot') return JSON.parse(data)
          continue
        }
        if (line.startsWith(':')) continue
        const colon = line.indexOf(':')
        const field = colon < 0 ? line : line.slice(0, colon)
        const value = colon < 0 ? '' : line.slice(colon + 1).replace(/^ /, '')
        if (field === 'event') state.event = value
        else if (field === 'data') state.data.push(value)
        continue
      }

      const { done, value } = await reader.read()
      if (done) return null
      let text = decoder.decode(value, { stream: true })
      if (state.lastCr && text.startsWith('\n')) text = text.slice(1)
      state.lastCr = text.endsWith('\r')
      state.buffer += text.replace(/\r\n?/g, '\n')
    }
  }

  return { reader, nextSnapshot }
}

function within(promise, milliseconds, label) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(label)), milliseconds)
    }),
  ]).finally(() => clearTimeout(timer))
}

async function openSystemLive() {
  const startedAt = Date.now()
  const controller = new AbortController()
  const connectionTimeout = setTimeout(() => controller.abort(), 10_000)
  let response
  try {
    response = await fetch(`${base}/api/admin/system/live`, {
      headers: {
        Accept: 'text/event-stream',
        Authorization: 'Bearer docker-test-admin',
      },
      cache: 'no-store',
      signal: controller.signal,
    })
  } finally {
    clearTimeout(connectionTimeout)
  }
  assert.equal(response.status, 200, 'System Live SSE through Nginx')
  assert.ok(response.headers.get('content-type')?.startsWith('text/event-stream'))
  assert.equal(response.headers.get('cache-control'), 'no-store')
  assert.ok(response.body, 'System Live response body')
  return { ...sseReader(response.body), startedAt }
}

async function readSystemLiveSnapshots(stream) {
  const first = await within(
    stream.nextSnapshot(),
    Math.max(1, 2_000 - (Date.now() - stream.startedAt)),
    'first System Live snapshot exceeded 2s',
  )
  assert.equal(first?.v, 1, 'first System Live snapshot contract')
  const firstElapsed = Date.now() - stream.startedAt
  const second = await within(
    stream.nextSnapshot(),
    Math.max(1, 7_000 - (Date.now() - stream.startedAt)),
    'second System Live snapshot exceeded 7s',
  )
  assert.equal(second?.v, 1, 'second System Live snapshot contract')
  const secondElapsed = Date.now() - stream.startedAt
  return { firstElapsed, secondElapsed }
}

async function checkSystemLiveProxy() {
  const stream = await openSystemLive()
  try {
    const { firstElapsed, secondElapsed } = await readSystemLiveSnapshots(stream)
    console.log(`System Live through Nginx passed: snapshot 1 ${firstElapsed}ms, snapshot 2 ${secondElapsed}ms`)
  } finally {
    await stream.reader.cancel()
  }
}

async function waitForSystemLiveShutdown() {
  const stream = await openSystemLive()
  try {
    await readSystemLiveSnapshots(stream)
    console.log('SYSTEM_LIVE_WAITING_FOR_SERVER_STOP')
    const closed = await within(stream.reader.read(), 35_000, 'System Live stream did not close after server stop')
    assert.equal(closed.done, true, 'server shutdown should end the SSE body')
    console.log('SYSTEM_LIVE_SERVER_STOP_CLOSED')
  } finally {
    await stream.reader.cancel().catch(() => {})
  }
}

// Compose reports a started web container before Nginx necessarily accepts connections.
for (let attempt = 0; ; attempt++) {
  try {
    assert.equal((await get('/ready')).status, 200)
    break
  } catch (error) {
    if (attempt === 19) throw error
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
}
for (const path of ['/health', '/ready']) {
  const response = await get(path)
  assert.equal(response.status, 200, path)
}
const home = await get('/')
assert.equal(home.status, 200)
const html = await home.text()
assert.ok(html.includes('<div id="app">'))
const route = await get('/agents/docker-smoke')
assert.equal(route.status, 200)
assert.equal(await route.text(), html, 'SPA history fallback')
assert.equal((await get('/api/admin/system')).status, 401)
assert.equal((await get('/api/admin/system', 'incorrect')).status, 401)
assert.equal((await get('/api/admin/system', 'docker-test-admin')).status, 200)
if (!['after-restart', 'system-live-shutdown'].includes(process.argv[2])) {
  await checkSystemLiveProxy()
}

// An unauthenticated upgrade must reach Rust and fail before device admission.
const upgradeStatus = await new Promise((resolve, reject) => {
  const request = httpGet(`${base}/voice/v1/`, {
    headers: {
      Connection: 'Upgrade',
      Upgrade: 'websocket',
      'Sec-WebSocket-Version': '13',
      'Sec-WebSocket-Key': 'dGhlIHNhbXBsZSBub25jZQ==',
      'Device-Id': 'docker-smoke',
      'Client-Id': 'docker-smoke',
    },
    signal: AbortSignal.timeout(10_000),
  }, (response) => {
    response.resume()
    resolve(response.statusCode)
  })
  request.on('upgrade', (_, socket) => {
    socket.destroy()
    reject(new Error('Unauthenticated WebSocket unexpectedly accepted'))
  })
  request.on('error', reject)
})
assert.equal(upgradeStatus, 401, 'WebSocket proxy/auth')

const speakerPath = '/api/admin/speakers/docker_smoke'
if (process.argv[2] === 'system-live-shutdown') {
  await waitForSystemLiveShutdown()
} else if (process.argv[2] === 'after-restart') {
  const response = await get(speakerPath, 'docker-test-admin')
  assert.equal(response.status, 200, 'SQLite data survives restart')
  assert.equal((await response.json()).name, 'Docker smoke')
} else {
  const response = await fetch(`${base}/api/admin/speakers`, {
    method: 'POST',
    headers: { Authorization: 'Bearer docker-test-admin', 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'docker_smoke', name: 'Docker smoke' }),
    signal: AbortSignal.timeout(10_000),
  })
  assert.equal(response.status, 201, 'SQLite write through proxy')
}
if (process.argv[2] !== 'system-live-shutdown') {
  console.log('Docker smoke passed: readiness, SPA, admin auth, WebSocket proxy/auth, SQLite')
}
