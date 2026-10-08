import assert from 'node:assert/strict'
import { get as httpGet } from 'node:http'

const base = 'http://web:8080'
async function get(path, token) {
  return fetch(`${base}${path}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal: AbortSignal.timeout(10_000),
  })
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

const speakerPath = '/api/admin/speakers/docker-smoke'
if (process.argv[2] === 'after-restart') {
  const response = await get(speakerPath, 'docker-test-admin')
  assert.equal(response.status, 200, 'SQLite data survives restart')
  assert.equal((await response.json()).name, 'Docker smoke')
} else {
  const response = await fetch(`${base}/api/admin/speakers`, {
    method: 'POST',
    headers: { Authorization: 'Bearer docker-test-admin', 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'docker-smoke', name: 'Docker smoke' }),
    signal: AbortSignal.timeout(10_000),
  })
  assert.equal(response.status, 201, 'SQLite write through proxy')
}
console.log('Docker smoke passed: readiness, SPA, admin auth, WebSocket proxy/auth, SQLite')
